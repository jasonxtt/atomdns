//! Linux-only batched UDP datagram I/O (`recvmmsg` / `sendmmsg`).
//!
//! All `unsafe` FFI and buffer-backing logic for batched UDP lives in this
//! module; the rest of `native-host` keeps `#![deny(unsafe_code)]`.
#![allow(unsafe_code)]

use std::io;
use std::mem::{self, MaybeUninit};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::os::fd::AsRawFd;
use std::ptr::{self, NonNull};

use tokio::io::Interest;
use tokio::net::UdpSocket;

/// Maximum UDP datagram payload size (matching `udp::MAX_UDP_PACKET`).
pub(crate) const MAX_UDP_PACKET: usize = 65_535;

/// Per-slot stride in the receive slab: holds a full 65,535-byte datagram
/// while staggering consecutive slot starts by 256 bytes modulo 4 KiB to avoid
/// L1d cache-set aliasing across slots.
const SLOT_STRIDE: usize = 65_536 + 256;

/// `MSG_DONTWAIT` typed for `libc::recvmmsg` / `libc::sendmmsg` flags, which
/// take `c_uint` on musl and `c_int` on glibc.
#[cfg(target_env = "musl")]
const MMSG_DONTWAIT: libc::c_uint = libc::MSG_DONTWAIT as libc::c_uint;
#[cfg(not(target_env = "musl"))]
const MMSG_DONTWAIT: libc::c_int = libc::MSG_DONTWAIT;

/// Contiguous byte backing for `RecvBatch` slots.
///
/// Backed by an anonymous `mmap` so that each new listener (including after
/// hot-apply rebinds) receives fresh demand-paged zero pages: normal DNS
/// queries only fault in the first 4 KiB page of the slots actually used
/// (~4 KiB at low load, ~128 KiB at 32-packet full load), while large UDP
/// datagrams up to 65,535 bytes are received without truncation. On drop the
/// mapping is returned immediately to the kernel via `munmap`.
enum BufferBacking {
    Mmap { ptr: NonNull<u8>, len: usize },
    Heap(Box<[u8]>),
}

// SAFETY: `BufferBacking` owns its memory exclusively and has no thread-local
// state; moving it across threads is sound.
unsafe impl Send for BufferBacking {}

impl BufferBacking {
    fn new(min_bytes: usize) -> Self {
        assert!(min_bytes > 0, "receive slab must be non-empty");
        let page = page_size();
        let aligned = min_bytes
            .checked_add(page - 1)
            .map_or(min_bytes, |n| n & !(page - 1));
        // SAFETY: `mmap` with `MAP_PRIVATE | MAP_ANONYMOUS` and `fd = -1`
        // allocates a fresh anonymous read/write mapping of `aligned` bytes.
        let raw = unsafe {
            libc::mmap(
                ptr::null_mut(),
                aligned,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if raw != libc::MAP_FAILED {
            if let Some(ptr) = NonNull::new(raw.cast::<u8>()) {
                return Self::Mmap { ptr, len: aligned };
            }
        }
        Self::Heap(vec![0_u8; min_bytes].into_boxed_slice())
    }

    fn len(&self) -> usize {
        match self {
            Self::Mmap { len, .. } => *len,
            Self::Heap(buf) => buf.len(),
        }
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        match self {
            Self::Mmap { ptr, .. } => ptr.as_ptr(),
            Self::Heap(buf) => buf.as_mut_ptr(),
        }
    }

    fn slice(&self, offset: usize, len: usize) -> &[u8] {
        let end = offset
            .checked_add(len)
            .expect("receive buffer slice overflow");
        assert!(end <= self.len(), "receive buffer slice out of bounds");
        match self {
            Self::Mmap { ptr, .. } => {
                // SAFETY: `offset + len <= self.len()`, the mapping is valid
                // and initialized (anonymous zero pages overwritten by kernel
                // `recvmmsg`), and `&self` prevents concurrent mutation.
                unsafe { std::slice::from_raw_parts(ptr.as_ptr().add(offset), len) }
            }
            Self::Heap(buf) => &buf[offset..end],
        }
    }
}

impl Drop for BufferBacking {
    fn drop(&mut self) {
        if let Self::Mmap { ptr, len } = *self {
            // SAFETY: `(ptr, len)` came from a successful `libc::mmap` call in
            // `BufferBacking::new` and is unmapped exactly once on drop.
            unsafe {
                let _ = libc::munmap(ptr.as_ptr().cast(), len);
            }
        }
    }
}

fn page_size() -> usize {
    // SAFETY: `sysconf(_SC_PAGESIZE)` takes no pointers and is thread-safe.
    let raw = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(raw)
        .ok()
        .filter(|&n| n.is_power_of_two())
        .unwrap_or(4096)
}

/// Reusable receive state for `recvmmsg`. Allocated once per UDP listener.
pub(crate) struct RecvBatch {
    bufs: BufferBacking,
    names: Box<[MaybeUninit<libc::sockaddr_storage>]>,
    iovs: Box<[libc::iovec]>,
    hdrs: Box<[libc::mmsghdr]>,
    cap: usize,
    last_count: usize,
}

impl RecvBatch {
    pub(crate) fn new(cap: usize) -> Self {
        assert!(cap > 0, "RecvBatch capacity must be non-zero");
        assert!(
            libc::c_uint::try_from(cap).is_ok(),
            "RecvBatch capacity exceeds c_uint"
        );
        let total_bytes = cap
            .checked_mul(SLOT_STRIDE)
            .expect("RecvBatch slab size overflow");
        let bufs = BufferBacking::new(total_bytes);
        let names = (0..cap)
            .map(|_| MaybeUninit::uninit())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        // SAFETY: `libc::iovec` and `libc::mmsghdr` are plain C structs whose
        // all-zero bit pattern is valid.
        let iovs = (0..cap)
            .map(|_| unsafe { mem::zeroed::<libc::iovec>() })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let hdrs = (0..cap)
            .map(|_| unsafe { mem::zeroed::<libc::mmsghdr>() })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            bufs,
            names,
            iovs,
            hdrs,
            cap,
            last_count: 0,
        }
    }

    fn prepare(&mut self, count: usize) {
        debug_assert!(count > 0 && count <= self.cap);
        let base = self.bufs.as_mut_ptr();
        let namelen = libc::socklen_t::try_from(mem::size_of::<libc::sockaddr_storage>())
            .expect("sockaddr_storage fits socklen_t");
        for i in 0..count {
            // SAFETY: `i < self.cap` and `self.bufs.len() >= self.cap * SLOT_STRIDE`,
            // so `i * SLOT_STRIDE + MAX_UDP_PACKET <= self.bufs.len()`.
            let slot_ptr = unsafe { base.add(i * SLOT_STRIDE) };
            self.iovs[i] = libc::iovec {
                iov_base: slot_ptr.cast(),
                iov_len: MAX_UDP_PACKET,
            };
            let hdr = &mut self.hdrs[i];
            hdr.msg_len = 0;
            hdr.msg_hdr.msg_name = self.names[i].as_mut_ptr().cast();
            hdr.msg_hdr.msg_namelen = namelen;
            hdr.msg_hdr.msg_iov = ptr::addr_of_mut!(self.iovs[i]);
            hdr.msg_hdr.msg_iovlen = 1;
            hdr.msg_hdr.msg_control = ptr::null_mut();
            hdr.msg_hdr.msg_controllen = 0;
            hdr.msg_hdr.msg_flags = 0;
        }
    }

    /// Awaits socket readability and receives between `1` and `max_count`
    /// datagrams without blocking for additional packets.
    ///
    /// Cancel-safe: the syscall runs synchronously only after readiness is
    /// observed and returns immediately once the kernel populates the batch.
    pub(crate) async fn recv(&mut self, socket: &UdpSocket, max_count: usize) -> io::Result<usize> {
        let count = max_count.clamp(1, self.cap);
        self.last_count = 0;
        // Prepare headers once outside `async_io` so an `EAGAIN` retry after
        // readiness wakeup does not rewrite the header array. Every pointer in
        // `hdrs[..count]` is freshly derived from `&mut self` and stays valid
        // because none of `bufs`, `names`, or `iovs` is reallocated or touched
        // until `async_io` returns.
        self.prepare(count);
        let fd = socket.as_raw_fd();
        let hdrs_ptr = self.hdrs.as_mut_ptr();
        let vlen = libc::c_uint::try_from(count).expect("batch size fits c_uint");
        let received = socket
            .async_io(Interest::READABLE, || {
                loop {
                    // SAFETY: `fd` is the live descriptor of `socket`; `hdrs_ptr`
                    // points to `vlen` initialized `mmsghdr` entries whose
                    // `msg_name` and `iov_base` point to disjoint writable slots
                    // owned exclusively by `&mut self` for the duration of this
                    // future; `timeout` is null (non-blocking with `MSG_DONTWAIT`).
                    let n = unsafe {
                        libc::recvmmsg(fd, hdrs_ptr, vlen, MMSG_DONTWAIT, ptr::null_mut())
                    };
                    if n >= 0 {
                        return Ok(usize::try_from(n).expect("non-negative recvmmsg count"));
                    }
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
            })
            .await?;
        self.last_count = received.min(count);
        Ok(self.last_count)
    }

    /// Returns the `index`-th datagram and its raw peer address from the last
    /// successful `recv` call. Returns `None` if `index` is out of range or
    /// the kernel returned an unsupported/truncated peer address.
    pub(crate) fn packet(&self, index: usize) -> Option<(&[u8], SocketAddr)> {
        if index >= self.last_count {
            return None;
        }
        let hdr = &self.hdrs[index];
        let len = (hdr.msg_len as usize).min(MAX_UDP_PACKET);
        let peer = decode_sockaddr(&self.names[index], hdr.msg_hdr.msg_namelen)?;
        let bytes = self.bufs.slice(index * SLOT_STRIDE, len);
        Some((bytes, peer))
    }
}

/// Reusable `sendmmsg` header/address scratch space. Allocated once per UDP
/// listener.
pub(crate) struct SendScratch {
    names: Box<[MaybeUninit<libc::sockaddr_storage>]>,
    iovs: Box<[libc::iovec]>,
    hdrs: Box<[libc::mmsghdr]>,
    cap: usize,
}

impl SendScratch {
    pub(crate) fn new(cap: usize) -> Self {
        assert!(cap > 0, "SendScratch capacity must be non-zero");
        assert!(
            libc::c_uint::try_from(cap).is_ok(),
            "SendScratch capacity exceeds c_uint"
        );
        let names = (0..cap)
            .map(|_| MaybeUninit::uninit())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        // SAFETY: `libc::iovec` and `libc::mmsghdr` are plain C structs whose
        // all-zero bit pattern is valid.
        let iovs = (0..cap)
            .map(|_| unsafe { mem::zeroed::<libc::iovec>() })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let hdrs = (0..cap)
            .map(|_| unsafe { mem::zeroed::<libc::mmsghdr>() })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            names,
            iovs,
            hdrs,
            cap,
        }
    }

    /// Attempts to send up to `count` datagrams in one non-blocking `sendmmsg`
    /// call.
    ///
    /// Returns the number of datagrams accepted by the kernel (`>= 1` on
    /// partial or full success), or `Err(WouldBlock)` (clearing tokio's
    /// writable readiness) / another `io::Error` if the very first datagram
    /// failed. `EINTR` is retried internally.
    pub(crate) fn try_send_batch<'a>(
        &mut self,
        socket: &UdpSocket,
        count: usize,
        mut get: impl FnMut(usize) -> (&'a [u8], SocketAddr),
    ) -> io::Result<usize> {
        let count = count.min(self.cap);
        if count == 0 {
            return Ok(0);
        }
        for i in 0..count {
            let (wire, peer) = get(i);
            let namelen = encode_sockaddr(peer, &mut self.names[i]);
            self.iovs[i] = libc::iovec {
                // POSIX `iovec::iov_base` is `*mut c_void`, but `sendmmsg`
                // only reads from the buffer.
                iov_base: wire.as_ptr().cast_mut().cast(),
                iov_len: wire.len(),
            };
            let hdr = &mut self.hdrs[i];
            hdr.msg_len = 0;
            hdr.msg_hdr.msg_name = self.names[i].as_mut_ptr().cast();
            hdr.msg_hdr.msg_namelen = namelen;
            hdr.msg_hdr.msg_iov = ptr::addr_of_mut!(self.iovs[i]);
            hdr.msg_hdr.msg_iovlen = 1;
            hdr.msg_hdr.msg_control = ptr::null_mut();
            hdr.msg_hdr.msg_controllen = 0;
            hdr.msg_hdr.msg_flags = 0;
        }
        let fd = socket.as_raw_fd();
        let hdrs_ptr = self.hdrs.as_mut_ptr();
        let vlen = libc::c_uint::try_from(count).expect("batch size fits c_uint");
        socket.try_io(Interest::WRITABLE, || {
            loop {
                // SAFETY: `fd` is the live descriptor of `socket`; `hdrs_ptr`
                // points to `vlen` initialized `mmsghdr` entries whose `msg_name`
                // points into `self.names` and whose `iov_base` points to caller
                // slices valid for `'a` (which outlives this synchronous call).
                let n = unsafe { libc::sendmmsg(fd, hdrs_ptr, vlen, MMSG_DONTWAIT) };
                if n >= 0 {
                    return Ok(usize::try_from(n).expect("non-negative sendmmsg count"));
                }
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
        })
    }
}

fn decode_sockaddr(
    storage: &MaybeUninit<libc::sockaddr_storage>,
    namelen: libc::socklen_t,
) -> Option<SocketAddr> {
    let len = usize::try_from(namelen).ok()?;
    if len < mem::size_of::<libc::sa_family_t>() {
        return None;
    }
    let ptr = storage.as_ptr();
    // SAFETY: `namelen >= size_of::<sa_family_t>()` guarantees the kernel
    // populated the leading `ss_family` field of `sockaddr_storage`.
    let family = unsafe { ptr::addr_of!((*ptr).ss_family).read() };
    match libc::c_int::from(family) {
        libc::AF_INET if len >= mem::size_of::<libc::sockaddr_in>() => {
            // SAFETY: `ss_family == AF_INET` and `namelen >= size_of::<sockaddr_in>()`
            // guarantees a valid initialized `sockaddr_in` in `storage` (whose
            // alignment is at least that of `sockaddr_in`).
            let sin = unsafe { &*ptr.cast::<libc::sockaddr_in>() };
            let ip = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
            let port = u16::from_be(sin.sin_port);
            Some(SocketAddr::V4(SocketAddrV4::new(ip, port)))
        }
        libc::AF_INET6 if len >= mem::size_of::<libc::sockaddr_in6>() => {
            // SAFETY: `ss_family == AF_INET6` and `namelen >= size_of::<sockaddr_in6>()`
            // guarantees a valid initialized `sockaddr_in6` in `storage` (whose
            // alignment is at least that of `sockaddr_in6`).
            let sin6 = unsafe { &*ptr.cast::<libc::sockaddr_in6>() };
            let ip = Ipv6Addr::from(sin6.sin6_addr.s6_addr);
            let port = u16::from_be(sin6.sin6_port);
            Some(SocketAddr::V6(SocketAddrV6::new(
                ip,
                port,
                sin6.sin6_flowinfo,
                sin6.sin6_scope_id,
            )))
        }
        _ => None,
    }
}

fn encode_sockaddr(
    addr: SocketAddr,
    storage: &mut MaybeUninit<libc::sockaddr_storage>,
) -> libc::socklen_t {
    // SAFETY: `storage` is exclusively borrowed and properly aligned for both
    // `sockaddr_in` and `sockaddr_in6`; zero-initializing clears all padding
    // before setting the active address fields.
    unsafe {
        ptr::write_bytes(storage.as_mut_ptr(), 0, 1);
        match addr {
            SocketAddr::V4(v4) => {
                let sin = &mut *storage.as_mut_ptr().cast::<libc::sockaddr_in>();
                sin.sin_family = libc::AF_INET as libc::sa_family_t;
                sin.sin_port = v4.port().to_be();
                sin.sin_addr = libc::in_addr {
                    s_addr: u32::from(*v4.ip()).to_be(),
                };
                libc::socklen_t::try_from(mem::size_of::<libc::sockaddr_in>())
                    .expect("sockaddr_in fits socklen_t")
            }
            SocketAddr::V6(v6) => {
                let sin6 = &mut *storage.as_mut_ptr().cast::<libc::sockaddr_in6>();
                sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sin6.sin6_port = v6.port().to_be();
                sin6.sin6_flowinfo = v6.flowinfo();
                sin6.sin6_addr = libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                };
                sin6.sin6_scope_id = v6.scope_id();
                libc::socklen_t::try_from(mem::size_of::<libc::sockaddr_in6>())
                    .expect("sockaddr_in6 fits socklen_t")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn buffer_backing_uses_anonymous_mmap_and_zero_initializes() {
        let mut backing = BufferBacking::new(2 * SLOT_STRIDE);
        assert!(matches!(backing, BufferBacking::Mmap { .. }));
        assert!(backing.len() >= 2 * SLOT_STRIDE);
        assert_eq!(backing.slice(0, 16), &[0_u8; 16]);
        // SAFETY: writing within the first slot of the exclusively owned slab.
        unsafe {
            ptr::copy_nonoverlapping(b"atomdns".as_ptr(), backing.as_mut_ptr(), 7);
        }
        assert_eq!(backing.slice(0, 7), b"atomdns");
        assert_eq!(backing.slice(SLOT_STRIDE, 8), &[0_u8; 8]);
    }

    #[test]
    fn sockaddr_roundtrip_matches_socket2_for_v4_v6_and_mapped_addresses() {
        let cases: Vec<SocketAddr> = vec![
            "192.0.2.1:53".parse().unwrap(),
            "198.51.100.77:5353".parse().unwrap(),
            "203.0.113.254:65535".parse().unwrap(),
            "[2001:db8::1]:53".parse().unwrap(),
            "[::ffff:192.0.2.9]:4000".parse().unwrap(),
            SocketAddr::V6(SocketAddrV6::new(
                "2001:db8::42".parse().unwrap(),
                8053,
                0x000a_bcde,
                3,
            )),
        ];
        for addr in cases {
            let mut storage = MaybeUninit::<libc::sockaddr_storage>::uninit();
            let len = encode_sockaddr(addr, &mut storage);
            let expected = socket2::SockAddr::from(addr);
            assert_eq!(len, expected.len(), "length mismatch for {addr}");
            // SAFETY: both buffers are initialized for `len` bytes.
            let encoded_bytes =
                unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), len as usize) };
            let expected_bytes = unsafe {
                std::slice::from_raw_parts(expected.as_ptr().cast::<u8>(), expected.len() as usize)
            };
            assert_eq!(encoded_bytes, expected_bytes, "wire mismatch for {addr}");
            assert_eq!(decode_sockaddr(&storage, len), Some(addr));
        }
    }

    #[test]
    fn decode_sockaddr_rejects_truncated_and_unknown_families() {
        let mut storage = MaybeUninit::<libc::sockaddr_storage>::uninit();
        let len_v4 = encode_sockaddr("192.0.2.5:53".parse().unwrap(), &mut storage);
        assert_eq!(decode_sockaddr(&storage, 0), None);
        assert_eq!(decode_sockaddr(&storage, 1), None);
        assert_eq!(decode_sockaddr(&storage, len_v4 - 1), None);

        let len_v6 = encode_sockaddr("[2001:db8::5]:53".parse().unwrap(), &mut storage);
        assert_eq!(decode_sockaddr(&storage, len_v6 - 1), None);

        // SAFETY: writing a zeroed sockaddr with AF_UNSPEC family.
        unsafe {
            ptr::write_bytes(storage.as_mut_ptr(), 0, 1);
        }
        assert_eq!(
            decode_sockaddr(
                &storage,
                mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t
            ),
            None
        );
    }

    #[test]
    fn recv_batch_receives_queued_bursts_and_wakes_after_draining() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let server = crate::bind::udp("127.0.0.1:0".parse().unwrap())
                .await
                .unwrap();
            let server_addr = server.local_addr().unwrap();
            let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let client_addr = client.local_addr().unwrap();

            for i in 0..40_u16 {
                let payload = i.to_be_bytes();
                client.send_to(&payload, server_addr).await.unwrap();
            }

            let mut batch = RecvBatch::new(32);
            let mut seen = Vec::new();
            let first = batch.recv(&server, 32).await.unwrap();
            assert_eq!(first, 32);
            for i in 0..first {
                let (bytes, peer) = batch.packet(i).expect("packet in range");
                assert_eq!(peer, client_addr);
                seen.push(u16::from_be_bytes(bytes.try_into().unwrap()));
            }
            assert!(batch.packet(first).is_none());

            let second = batch.recv(&server, 32).await.unwrap();
            assert_eq!(second, 8);
            for i in 0..second {
                let (bytes, peer) = batch.packet(i).expect("packet in range");
                assert_eq!(peer, client_addr);
                seen.push(u16::from_be_bytes(bytes.try_into().unwrap()));
            }
            assert_eq!(seen, (0..40_u16).collect::<Vec<_>>());

            // Queue is now empty: a new recv must hit EAGAIN, clear readiness,
            // and wake up when one more datagram arrives.
            let delayed = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(15)).await;
                client.send_to(b"wake", server_addr).await.unwrap();
            });
            let woke = tokio::time::timeout(Duration::from_secs(2), batch.recv(&server, 32))
                .await
                .expect("wakeup after EAGAIN")
                .unwrap();
            assert_eq!(woke, 1);
            let (bytes, peer) = batch.packet(0).unwrap();
            assert_eq!(bytes, b"wake");
            assert_eq!(peer, client_addr);
            delayed.await.unwrap();
        });
    }

    #[test]
    fn batch_handles_mixed_sizes_up_to_large_udp_datagrams_without_truncation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let server = crate::bind::udp("127.0.0.1:0".parse().unwrap())
                .await
                .unwrap();
            let server_addr = server.local_addr().unwrap();
            let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let client_addr = client.local_addr().unwrap();

            let sizes = [12_usize, 30, 512, 1232, 4096, 60_000];
            let payloads: Vec<Vec<u8>> = sizes
                .iter()
                .enumerate()
                .map(|(idx, &sz)| vec![(idx as u8).wrapping_add(0x21); sz])
                .collect();
            for payload in &payloads {
                client.send_to(payload, server_addr).await.unwrap();
            }

            let mut recv = RecvBatch::new(16);
            let mut got = Vec::new();
            while got.len() < payloads.len() {
                let n = recv.recv(&server, 16).await.unwrap();
                for i in 0..n {
                    let (bytes, peer) = recv.packet(i).unwrap();
                    assert_eq!(peer, client_addr);
                    got.push(bytes.to_vec());
                }
            }
            assert_eq!(got, payloads);

            // Echo them all back in one sendmmsg batch and verify exact bytes.
            let mut scratch = SendScratch::new(16);
            let mut cursor = 0;
            while cursor < got.len() {
                let sent = scratch
                    .try_send_batch(&server, got.len() - cursor, |i| {
                        (&got[cursor + i], client_addr)
                    })
                    .unwrap();
                assert!(sent > 0);
                cursor += sent;
            }

            let mut buf = vec![0_u8; MAX_UDP_PACKET];
            for expected in &payloads {
                let (len, _) =
                    tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buf))
                        .await
                        .expect("echo reply")
                        .unwrap();
                assert_eq!(&buf[..len], expected.as_slice());
            }
        });
    }

    #[test]
    fn dual_stack_batch_preserves_raw_ipv4_mapped_and_native_ipv6_peers() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let Ok(server) = crate::bind::udp("[::]:0".parse().unwrap()).await else {
                return;
            };
            let port = server.local_addr().unwrap().port();
            let v4_client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let Ok(v6_client) = UdpSocket::bind("[::1]:0").await else {
                return;
            };
            let v4_port = v4_client.local_addr().unwrap().port();
            let v6_addr = v6_client.local_addr().unwrap();

            v4_client
                .send_to(b"from-v4", SocketAddr::from(([127, 0, 0, 1], port)))
                .await
                .unwrap();
            v6_client
                .send_to(b"from-v6", SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)))
                .await
                .unwrap();

            let mut recv = RecvBatch::new(8);
            let mut received = Vec::new();
            while received.len() < 2 {
                let n = recv.recv(&server, 8).await.unwrap();
                for i in 0..n {
                    let (bytes, peer) = recv.packet(i).unwrap();
                    received.push((bytes.to_vec(), peer));
                }
            }
            let v4_entry = received
                .iter()
                .find(|(b, _)| b.as_slice() == b"from-v4")
                .expect("v4 datagram");
            let v6_entry = received
                .iter()
                .find(|(b, _)| b.as_slice() == b"from-v6")
                .expect("v6 datagram");

            // Dual-stack wildcard reports IPv4 peers as IPv4-mapped IPv6;
            // raw peer must be kept for sending, while canonical_peer unmaps it.
            assert!(
                matches!(v4_entry.1, SocketAddr::V6(v6) if v6.ip().to_ipv4_mapped() == Some(Ipv4Addr::LOCALHOST)),
                "expected IPv4-mapped IPv6 peer, got {}",
                v4_entry.1
            );
            assert_eq!(
                crate::bind::canonical_peer(v4_entry.1),
                SocketAddr::from(([127, 0, 0, 1], v4_port))
            );
            assert_eq!(v6_entry.1, v6_addr);

            // Send both replies in one batch using raw peers.
            let mut scratch = SendScratch::new(8);
            let replies = [
                (b"to-v4".as_slice(), v4_entry.1),
                (b"to-v6".as_slice(), v6_entry.1),
            ];
            let sent = scratch
                .try_send_batch(&server, replies.len(), |i| replies[i])
                .unwrap();
            assert_eq!(sent, 2);

            let mut buf = [0_u8; 32];
            let (n4, _) = tokio::time::timeout(Duration::from_secs(2), v4_client.recv_from(&mut buf))
                .await
                .expect("v4 reply")
                .unwrap();
            assert_eq!(&buf[..n4], b"to-v4");
            let (n6, _) = tokio::time::timeout(Duration::from_secs(2), v6_client.recv_from(&mut buf))
                .await
                .expect("v6 reply")
                .unwrap();
            assert_eq!(&buf[..n6], b"to-v6");
        });
    }
}
