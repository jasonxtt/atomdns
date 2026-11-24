//! Plain-UDP exchange primitive over a small per-upstream socket pool.
//!
//! Every UDP [`crate::Upstream`] owns a lazily created [`Pool`] of at most
//! [`ACTIVE_SOCKETS`] unconnected sockets, each bound to a kernel-chosen
//! ephemeral port in the endpoint's address family. An exchange leases one
//! socket chosen at random, draws a fresh random DNS ID that has never been
//! issued on that socket before, sends a copy of the caller's query carrying
//! that ID exactly once, and waits for its datagram. The caller's query bytes
//! are never modified, and the accepted response is returned with the caller's
//! original ID restored, so the upstream-visible ID never leaves this module.
//!
//! Each pooled socket has one small receive task, held by the pool, that reads
//! the socket and hands a datagram to the exchange whose ID it carries. A
//! socket stops taking new exchanges after [`EXCHANGES_PER_SOCKET`] exchanges
//! or [`SOCKET_LIFETIME`], and closes once its last exchange has ended. Owner
//! close ends every receive task and `Upstream::close` awaits them; dropping
//! the owner aborts them. No task outlives its owner.
//!
//! Acceptance is unchanged from the one-socket-per-exchange primitive: only a
//! datagram from the configured numeric peer address (including port) whose
//! ID matches the exchange is attributed to it. Header, TC and full DNS
//! validation then run exactly as before, and a valid truncated (TC=1) header
//! is returned as an owned observation for the composite policy; this module
//! never starts TCP. Question-section checks stay with the caller.

use std::future::poll_fn;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use atomdns_dns_core::{inspect_response_header, validate_response};
use tokio::net::UdpSocket;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{
    ExchangeResponse, IgnoredDatagrams, PreparedExchange, ResponseCommit, SideEffectState,
    TerminalError, Transport, TransportCancellation, UpstreamError,
};

/// The largest possible DNS wire size. Responses are received with this full
/// legal datagram capacity and must never inherit Go's 4095-byte
/// implementation buffer, which silently truncated legal UDP responses.
const RECV_BUFFER_BYTES: usize = 65_535;

/// Sockets that take new exchanges at the same time, per upstream.
pub(crate) const ACTIVE_SOCKETS: usize = 4;

/// Exchanges one socket carries before it stops taking new ones. Every ID is
/// issued at most once per socket, so this also bounds the socket's ID table.
pub(crate) const EXCHANGES_PER_SOCKET: usize = 64;

/// The longest time one socket takes new exchanges, so an idle upstream also
/// moves to fresh ports.
pub(crate) const SOCKET_LIFETIME: Duration = Duration::from_secs(2);

/// Random ID draws before a socket that keeps colliding is retired.
const ID_DRAWS: usize = 8;

/// Classifies an ordinary local socket bind/setup failure.
///
/// Local setup has not sent a DNS payload, so it maps to the settled `NotSent`
/// [`UpstreamError::Connect`] category. This tiny seam keeps the mapping
/// unit-testable without injecting a socket factory.
fn bind_error(_error: &io::Error) -> UpstreamError {
    UpstreamError::Connect
}

/// Attaches retained ignored-datagram diagnostics to a terminal transport
/// error.
///
/// The primary cause is preserved verbatim and never relabelled; when nothing
/// was ignored the plain error is returned unchanged.
fn diagnosed(error: UpstreamError, ignored: IgnoredDatagrams) -> UpstreamError {
    if !ignored.any() {
        return error;
    }
    let cause = match error {
        UpstreamError::DeadlineExceeded(state) => TerminalError::DeadlineExceeded(state),
        UpstreamError::Cancelled(state) => TerminalError::Cancelled(state),
        UpstreamError::Closed(state) => TerminalError::Closed(state),
        UpstreamError::Receive(state) => TerminalError::Receive(state),
        other => return other,
    };
    UpstreamError::Diagnosed { cause, ignored }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Buffered kernel randomness for upstream IDs and socket choice.
struct RandomBytes {
    buffer: [u8; 64],
    used: usize,
}

impl RandomBytes {
    const fn new() -> Self {
        Self {
            buffer: [0; 64],
            used: 64,
        }
    }

    fn take<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        if self.used + N > self.buffer.len() {
            getrandom::fill(&mut self.buffer).map_err(io::Error::other)?;
            self.used = 0;
        }
        let mut out = [0; N];
        out.copy_from_slice(&self.buffer[self.used..self.used + N]);
        self.used += N;
        Ok(out)
    }

    fn id(&mut self) -> io::Result<u16> {
        self.take::<2>().map(u16::from_be_bytes)
    }

    /// A uniform-enough index below `count` (a handful of sockets).
    fn below(&mut self, count: usize) -> io::Result<usize> {
        let [byte] = self.take::<1>()?;
        Ok(usize::from(byte) % count)
    }
}

/// The per-upstream UDP socket pool. Its state is allocated on first use, so
/// an owner that never runs a UDP exchange (a TCP owner, for one) carries only
/// a pointer-sized empty slot.
pub(crate) struct Pool {
    inner: Mutex<Option<Box<PoolInner>>>,
}

struct PoolInner {
    sockets: Vec<PooledSocket>,
    random: RandomBytes,
    active: usize,
    lifetime: Duration,
    /// Sockets opened over the pool's life, for rotation tests.
    #[cfg(test)]
    opened: usize,
}

/// The pool's view of one socket. Only the receive task and the leases of
/// in-flight exchanges hold the socket strongly, so it closes as soon as the
/// task has ended and the last exchange on it is gone.
struct PooledSocket {
    shared: Weak<Shared>,
    reader: JoinHandle<()>,
    /// Cancelled when the receive task ends, however it ends (aborted or
    /// dropped by a runtime shutdown included).
    done: CancellationToken,
}

impl PooledSocket {
    /// The socket, if it still takes new exchanges.
    fn accepting(&self) -> Option<Arc<Shared>> {
        self.shared.upgrade().filter(|shared| shared.accepting())
    }
}

impl Pool {
    pub(crate) const fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Runs `f` on the pool state under its lock, creating the state first.
    fn with<R>(&self, f: impl FnOnce(&mut PoolInner) -> R) -> R {
        let mut inner = lock(&self.inner);
        let inner = inner.get_or_insert_with(|| {
            Box::new(PoolInner {
                sockets: Vec::new(),
                random: RandomBytes::new(),
                active: ACTIVE_SOCKETS,
                lifetime: SOCKET_LIFETIME,
                #[cfg(test)]
                opened: 0,
            })
        });
        f(inner)
    }

    /// Leases a socket and a fresh ID for one exchange.
    ///
    /// A socket that cannot be opened is tolerated while another socket still
    /// takes exchanges; with none left the open error is returned.
    fn lease(&self, peer: SocketAddr, owner: &TransportCancellation) -> io::Result<Lease> {
        self.with(|inner| Self::lease_in(inner, peer, owner))
    }

    fn lease_in(
        inner: &mut PoolInner,
        peer: SocketAddr,
        owner: &TransportCancellation,
    ) -> io::Result<Lease> {
        let PoolInner {
            sockets,
            random,
            active,
            lifetime,
            #[cfg(test)]
            opened,
        } = inner;
        // Receive tasks that ended (retired and drained, owner closed, or a
        // runtime that shut down) leave the pool.
        sockets.retain(|socket| !socket.done.is_cancelled());
        let mut open_error = None;
        for _ in 0..=*active {
            let mut accepting = sockets
                .iter()
                .filter(|socket| socket.accepting().is_some())
                .count();
            if accepting < *active {
                match PooledSocket::open(peer, owner, *lifetime) {
                    Ok(socket) => {
                        sockets.push(socket);
                        accepting += 1;
                        #[cfg(test)]
                        {
                            *opened += 1;
                        }
                    }
                    Err(error) => open_error = Some(error),
                }
            }
            if accepting == 0 {
                break;
            }
            let pick = random.below(accepting)?;
            let shared = sockets.iter().filter_map(PooledSocket::accepting).nth(pick);
            // A pick can only vanish if its receive task retired it between
            // the count and the pick; try again.
            let Some(shared) = shared else { continue };
            if let Some((index, id)) = shared.allocate(random)? {
                return Ok(Lease { shared, index, id });
            }
        }
        Err(open_error.unwrap_or_else(|| io::Error::other("no usable upstream UDP socket")))
    }

    /// Waits for every receive task to end after owner close; with no
    /// exchange left, each socket closes with its task. The owner token is
    /// already cancelled, so each task ends at its next poll.
    ///
    /// Nothing is taken out of the pool, so concurrent close calls all wait
    /// for the same tasks, and a cancelled close loses nothing.
    pub(crate) async fn close(&self) {
        let done: Vec<CancellationToken> = lock(&self.inner)
            .as_ref()
            .map(|inner| {
                inner
                    .sockets
                    .iter()
                    .map(|socket| socket.done.clone())
                    .collect()
            })
            .unwrap_or_default();
        for done in done {
            done.cancelled().await;
        }
    }

    /// Sockets whose receive task is still running.
    #[cfg(test)]
    pub(crate) fn open_sockets(&self) -> usize {
        self.with(|inner| {
            inner
                .sockets
                .iter()
                .filter(|socket| !socket.done.is_cancelled())
                .count()
        })
    }

    /// Overrides the active socket count and lifetime for tests.
    #[cfg(test)]
    pub(crate) fn configure(&self, active: usize, lifetime: Duration) {
        self.with(|inner| {
            inner.active = active;
            inner.lifetime = lifetime;
        });
    }

    /// Sockets opened over the pool's life.
    #[cfg(test)]
    pub(crate) fn opened(&self) -> usize {
        self.with(|inner| inner.opened)
    }

    /// Leases not yet dropped, summed over the pool's sockets.
    #[cfg(test)]
    pub(crate) fn live_leases(&self) -> usize {
        self.with(|inner| {
            inner
                .sockets
                .iter()
                .filter_map(|socket| socket.shared.upgrade())
                .map(|shared| lock(&shared.state).live)
                .sum()
        })
    }

    /// A weak handle to every pooled socket, to observe its release.
    #[cfg(test)]
    pub(crate) fn weak_sockets(&self) -> Vec<std::sync::Weak<impl Sized + use<>>> {
        self.with(|inner| {
            inner
                .sockets
                .iter()
                .map(|socket| socket.shared.clone())
                .collect()
        })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        let inner = self.inner.get_mut().unwrap_or_else(PoisonError::into_inner);
        for socket in inner.iter().flat_map(|inner| &inner.sockets) {
            socket.reader.abort();
        }
    }
}

impl PooledSocket {
    fn open(
        peer: SocketAddr,
        owner: &TransportCancellation,
        lifetime: Duration,
    ) -> io::Result<Self> {
        let bind = match peer {
            SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
            SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
        };
        let socket = std::net::UdpSocket::bind(bind)?;
        socket.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            socket: UdpSocket::from_std(socket)?,
            state: Mutex::new(SocketState {
                entries: Vec::new(),
                live: 0,
                retired: false,
            }),
            idle: Notify::new(),
        });
        let done = CancellationToken::new();
        // The guard lives in the task's future from the start, so `done` is
        // cancelled even if the task is dropped before its first poll.
        let finished = done.clone().drop_guard();
        let reader = tokio::spawn({
            let shared = Arc::clone(&shared);
            let owner = owner.clone();
            async move {
                let _finished = finished;
                receive(shared, peer, owner, lifetime).await;
            }
        });
        Ok(Self {
            shared: Arc::downgrade(&shared),
            reader,
            done,
        })
    }
}

/// One pooled socket and the exchanges issued on it.
struct Shared {
    socket: UdpSocket,
    state: Mutex<SocketState>,
    /// Signalled when a retired socket's last exchange ends.
    idle: Notify,
}

struct SocketState {
    /// Every exchange issued on this socket, in issue order. IDs are unique
    /// within the vector and are never removed, so a late datagram for an
    /// ended exchange is still recognized.
    entries: Vec<Entry>,
    /// Exchanges whose lease has not been dropped yet.
    live: usize,
    /// No new exchanges; the socket closes once `live` reaches zero.
    retired: bool,
}

struct Entry {
    id: u16,
    slot: Slot,
    ignored: IgnoredDatagrams,
}

enum Slot {
    Waiting(Option<Waker>),
    Delivered(Vec<u8>),
    ReceiveFailed,
    /// Consumed, or the exchange ended: later datagrams are late.
    Gone,
}

enum Delivery {
    Datagram(Vec<u8>),
    ReceiveFailed,
}

impl Shared {
    fn accepting(&self) -> bool {
        !lock(&self.state).retired
    }

    /// Issues a fresh ID on this socket, or retires it when none is found.
    fn allocate(&self, random: &mut RandomBytes) -> io::Result<Option<(usize, u16)>> {
        let mut state = lock(&self.state);
        if state.retired {
            return Ok(None);
        }
        for _ in 0..ID_DRAWS {
            let id = random.id()?;
            if state.entries.iter().any(|entry| entry.id == id) {
                continue;
            }
            let index = state.entries.len();
            state.entries.push(Entry {
                id,
                slot: Slot::Waiting(None),
                ignored: IgnoredDatagrams::NONE,
            });
            state.live += 1;
            if state.entries.len() >= EXCHANGES_PER_SOCKET {
                state.retired = true;
            }
            return Ok(Some((index, id)));
        }
        state.retired = true;
        let drained = state.live == 0;
        drop(state);
        if drained {
            self.idle.notify_one();
        }
        Ok(None)
    }

    fn retire(&self) {
        lock(&self.state).retired = true;
    }

    fn drained(&self) -> bool {
        let state = lock(&self.state);
        state.retired && state.live == 0
    }

    /// Hands one received datagram to the exchange whose ID it carries.
    ///
    /// | source | ID | effect |
    /// | --- | --- | --- |
    /// | peer | issued, waiting | delivered, exchange woken |
    /// | peer | issued, ended or already delivered | dropped (late) |
    /// | peer | never issued, or no ID | `response_id_mismatch` on every waiting exchange |
    /// | other | issued, waiting | `unexpected_peer` on that exchange |
    /// | other | issued, ended | dropped (late) |
    /// | other | never issued, or no ID | `unexpected_peer` on every waiting exchange |
    fn dispatch(&self, datagram: &[u8], from_peer: bool) {
        let mut state = lock(&self.state);
        let id = datagram
            .get(..2)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]));
        let Some(entry) = id.and_then(|id| state.entries.iter_mut().find(|entry| entry.id == id))
        else {
            for entry in &mut state.entries {
                if matches!(entry.slot, Slot::Waiting(_)) {
                    if from_peer {
                        entry.ignored.record_response_id_mismatch();
                    } else {
                        entry.ignored.record_unexpected_peer();
                    }
                }
            }
            return;
        };
        let Slot::Waiting(waker) = &mut entry.slot else {
            return;
        };
        if !from_peer {
            entry.ignored.record_unexpected_peer();
            return;
        }
        let waker = waker.take();
        entry.slot = Slot::Delivered(datagram.to_vec());
        drop(state);
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// A receive failure ends every waiting exchange and retires the socket.
    fn fail_waiters(&self) {
        let mut wakers = Vec::new();
        {
            let mut state = lock(&self.state);
            state.retired = true;
            for entry in &mut state.entries {
                if let Slot::Waiting(waker) = &mut entry.slot {
                    wakers.extend(waker.take());
                    entry.slot = Slot::ReceiveFailed;
                }
            }
        }
        for waker in wakers {
            waker.wake();
        }
    }
}

/// The receive task of one pooled socket.
async fn receive(
    shared: Arc<Shared>,
    peer: SocketAddr,
    owner: TransportCancellation,
    lifetime: Duration,
) {
    let owner_closed = owner.cancelled();
    tokio::pin!(owner_closed);
    let expiry = tokio::time::sleep(lifetime);
    tokio::pin!(expiry);
    let mut expired = false;
    let mut buffer = Vec::with_capacity(RECV_BUFFER_BYTES);
    enum Wake {
        Datagram(io::Result<(usize, SocketAddr)>),
        Check,
        Stop,
    }
    loop {
        if shared.drained() {
            break;
        }
        buffer.clear();
        let wake = tokio::select! {
            biased;
            () = &mut owner_closed => Wake::Stop,
            () = shared.idle.notified() => Wake::Check,
            () = &mut expiry, if !expired => {
                expired = true;
                shared.retire();
                Wake::Check
            }
            received = shared.socket.recv_buf_from(&mut buffer) => Wake::Datagram(received),
        };
        match wake {
            Wake::Datagram(Ok((length, from))) => shared.dispatch(&buffer[..length], from == peer),
            Wake::Datagram(Err(_)) => {
                shared.fail_waiters();
                break;
            }
            Wake::Check => {}
            Wake::Stop => break,
        }
    }
    shared.retire();
}

/// One exchange's registration on a pooled socket, released on drop.
struct Lease {
    shared: Arc<Shared>,
    index: usize,
    id: u16,
}

impl Lease {
    fn poll_delivery(&self, cx: &mut Context<'_>) -> Poll<Delivery> {
        let mut state = lock(&self.shared.state);
        let slot = &mut state.entries[self.index].slot;
        match slot {
            Slot::Waiting(registered) => {
                if !registered
                    .as_ref()
                    .is_some_and(|waker| waker.will_wake(cx.waker()))
                {
                    *registered = Some(cx.waker().clone());
                }
                Poll::Pending
            }
            Slot::Delivered(_) => match std::mem::replace(slot, Slot::Gone) {
                Slot::Delivered(datagram) => Poll::Ready(Delivery::Datagram(datagram)),
                _ => unreachable!("matched Delivered"),
            },
            Slot::ReceiveFailed => {
                *slot = Slot::Gone;
                Poll::Ready(Delivery::ReceiveFailed)
            }
            // A lease is consumed at most once; a second poll never happens.
            Slot::Gone => Poll::Pending,
        }
    }

    fn ignored(&self) -> IgnoredDatagrams {
        lock(&self.shared.state).entries[self.index].ignored
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let drained = {
            let mut state = lock(&self.shared.state);
            state.entries[self.index].slot = Slot::Gone;
            state.live -= 1;
            state.retired && state.live == 0
        };
        if drained {
            self.shared.idle.notify_one();
        }
    }
}

/// Runs one UDP exchange for a prepared exchange over the upstream's pool.
///
/// The caller has already validated the request, transport, and owner
/// lifecycle through [`crate::Upstream::prepare_exchange`]. Every async wake
/// re-applies the owner-close, caller-cancellation, and absolute-deadline
/// precedence from [`crate::ExchangeControl::check_at`]: owner close is
/// `Closed`, caller cancellation is `Cancelled`, and the deadline is
/// `DeadlineExceeded`, with cancellation winning a tie.
pub(crate) async fn exchange(
    prepared: &PreparedExchange<'_>,
    commit: &ResponseCommit<'_>,
    pool: &Pool,
) -> Result<ExchangeResponse, UpstreamError> {
    debug_assert_eq!(prepared.endpoint().transport(), Transport::Udp);

    // Before the lease: owner close, caller cancellation, then deadline.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;

    let endpoint = prepared.endpoint().address();
    let owner = prepared.owner_cancellation();
    // A local socket setup failure is a `Connect` runtime failure that has
    // sent nothing; `Connect.side_effect()` is already `NotSent`. The lease is
    // registered before the send, so an immediate answer is never missed, and
    // it is released on every return and drop path below.
    let lease = pool
        .lease(endpoint, &owner)
        .map_err(|error| bind_error(&error))?;

    let caller = prepared.context().cancellation();
    let owner_cancelled = owner.cancelled();
    let caller_cancelled = caller.cancelled();
    tokio::pin!(owner_cancelled);
    tokio::pin!(caller_cancelled);

    // One absolute deadline, created once and reused by every select below.
    let deadline = tokio::time::sleep_until(tokio::time::Instant::from_std(
        prepared.context().deadline(),
    ));
    tokio::pin!(deadline);

    let request_id = prepared.request().request_id();
    let query = prepared.request().query();
    // Only the owned outbound copy carries the upstream ID.
    let mut outbound = query.to_vec();
    outbound[..2].copy_from_slice(&lease.id.to_be_bytes());

    // Before send: re-apply the control, then send the query exactly once,
    // racing owner close, caller cancellation, and the deadline.
    prepared.check_at(Instant::now(), SideEffectState::NotSent)?;
    let sent = tokio::select! {
        biased;
        () = &mut owner_cancelled => Err(UpstreamError::Closed(SideEffectState::NotSent)),
        () = &mut caller_cancelled => Err(UpstreamError::Cancelled(SideEffectState::NotSent)),
        () = &mut deadline => Err(UpstreamError::DeadlineExceeded(SideEffectState::NotSent)),
        result = lease.shared.socket.send_to(&outbound, endpoint) => {
            result.map_err(|_| UpstreamError::Send(SideEffectState::MaybeSent))
        }
    }?;
    // Only a complete datagram send is `Sent`; a short send is uncertain.
    if sent != outbound.len() {
        return Err(UpstreamError::Send(SideEffectState::MaybeSent));
    }

    // Re-apply owner close, caller cancellation, and the shared absolute
    // deadline before waiting. Ignored datagrams never wake this exchange:
    // the receive task records them as diagnostics on the lease, and they are
    // attached only to the terminal error.
    prepared
        .check_at(Instant::now(), SideEffectState::Sent)
        .map_err(|error| diagnosed(error, lease.ignored()))?;
    let delivery = tokio::select! {
        biased;
        () = &mut owner_cancelled => {
            return Err(diagnosed(UpstreamError::Closed(SideEffectState::Sent), lease.ignored()))
        }
        () = &mut caller_cancelled => {
            return Err(diagnosed(UpstreamError::Cancelled(SideEffectState::Sent), lease.ignored()))
        }
        () = &mut deadline => {
            return Err(diagnosed(
                UpstreamError::DeadlineExceeded(SideEffectState::Sent),
                lease.ignored(),
            ))
        }
        delivery = poll_fn(|cx| lease.poll_delivery(cx)) => delivery,
    };
    let mut datagram = match delivery {
        Delivery::Datagram(datagram) => datagram,
        Delivery::ReceiveFailed => {
            return Err(diagnosed(
                UpstreamError::Receive(SideEffectState::Sent),
                lease.ignored(),
            ));
        }
    };

    // The datagram came from the configured peer and carries this exchange's
    // ID. A QR-clear or too-short one is terminally malformed.
    let header = match inspect_response_header(&datagram) {
        Ok(header) => header,
        Err(_) => return Err(UpstreamError::MalformedResponse),
    };
    debug_assert_eq!(header.id, lease.id);
    // The caller only ever sees its own ID.
    datagram[..2].copy_from_slice(&request_id.to_be_bytes());
    // A valid matching TC header is a successful UDP observation owned by the
    // response; the composite policy decides on TCP from it.
    if header.truncated {
        // The commit gate is the final decision before the owned observation.
        commit.before_commit().await;
        commit
            .commit()
            .map_err(|error| diagnosed(error, lease.ignored()))?;
        return Ok(ExchangeResponse::new(
            datagram,
            request_id,
            request_id,
            Transport::Udp,
            true,
        ));
    }
    // Only a complete non-TC wire that passes dns-core validation may be
    // returned as the accepted response.
    if validate_response(&datagram).is_err() {
        return Err(UpstreamError::MalformedResponse);
    }
    // Validation is complete; the commit gate is the single linearization
    // point against owner close, immediately before the owned response.
    commit.before_commit().await;
    commit
        .commit()
        .map_err(|error| diagnosed(error, lease.ignored()))?;
    Ok(ExchangeResponse::new(
        datagram,
        request_id,
        request_id,
        Transport::Udp,
        false,
    ))
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio::task::{JoinHandle, spawn_blocking};
    use tokio::time::timeout;

    use super::bind_error;
    use crate::{
        CloseResult, CloseTransition, CommitPause, Endpoint, ExchangeContext, ExchangeRequest,
        LifecycleState, SideEffectState, TerminalError, Transport, TransportCancellation, Upstream,
        UpstreamError,
    };

    /// The largest legal IPv4 UDP payload, matching the production receive
    /// buffer and the Slice1 integration fixtures.
    const LEGAL_UDP_PAYLOAD: usize = 65_507;

    /// Bounds every exchange so a missing or broken socket path cannot hang CI.
    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    /// Runs one bounded current-thread runtime for a single test.
    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build current-thread test runtime")
            .block_on(future)
    }

    /// A valid query with a caller-chosen ID.
    fn query_wire(id: u16) -> Vec<u8> {
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x01, 0x00]); // RD=1, QR=0, opcode QUERY
        wire.extend_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
        wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        wire
    }

    /// A complete, dns-core-valid response with a one-byte answer marker.
    fn response_wire(id: u16, marker: u8) -> Vec<u8> {
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x81, 0x80]); // QR=1, RD=1, RA=1
        wire.extend_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
        wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
        wire.extend_from_slice(&60u32.to_be_bytes());
        wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, marker]);
        wire
    }

    /// A truncated (TC=1) response header that the TC path accepts without a
    /// complete verified body and without any TCP fallback.
    fn truncated_response_wire(id: u16) -> Vec<u8> {
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x83, 0x80]); // QR=1, TC=1, RD=1, RA=1
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire
    }

    fn bind_ipv4() -> (UdpSocket, SocketAddr) {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind ipv4 loopback");
        let address = socket.local_addr().expect("local address");
        (socket, address)
    }

    fn udp_endpoint(address: SocketAddr) -> Endpoint {
        Endpoint::new(address, Transport::Udp).expect("numeric udp endpoint")
    }

    fn open_context() -> ExchangeContext {
        ExchangeContext::new(
            Instant::now() + Duration::from_secs(30),
            TransportCancellation::new(),
        )
    }

    /// Receives one datagram and sends `reply` back to its source, carrying
    /// the received (upstream-visible) ID.
    fn reply_once(socket: UdpSocket, reply: Vec<u8>) -> JoinHandle<()> {
        spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            let (_, peer) = socket.recv_from(&mut buffer).expect("receive query");
            let reply = with_id(reply, wire_id(&buffer));
            socket.send_to(&reply, peer).expect("send reply");
        })
    }

    /// Installs the deterministic pre-commit pause seam and returns the test's
    /// handle to it.
    fn install_pause(upstream: &Upstream) -> Arc<CommitPause> {
        let pause = Arc::new(CommitPause::new());
        upstream.install_commit_pause(Arc::clone(&pause));
        pause
    }

    /// Spawns one exchange on the current-thread runtime.
    fn spawn_exchange(
        upstream: &Arc<Upstream>,
        query: Vec<u8>,
    ) -> tokio::task::JoinHandle<Result<crate::ExchangeResponse, UpstreamError>> {
        let upstream = Arc::clone(upstream);
        tokio::spawn(async move {
            let request = ExchangeRequest::new(&query).expect("valid query");
            upstream.exchange(request, open_context()).await
        })
    }

    #[test]
    fn owner_close_wins_when_it_reaches_the_commit_gate_first() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let id = 0xe001;
            let server_task = reply_once(server, response_wire(id, 40));

            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let pause = install_pause(&upstream);
            let exchange_task = spawn_exchange(&upstream, query_wire(id));

            // The datagram is received and validated, but the response commit
            // has not run yet: the exchange is parked on the test seam.
            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("exchange reaches the pre-commit gate");
            assert_eq!(upstream.in_flight_exchanges(), 1);

            // Owner close reaches the shared gate first.
            assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closing);

            // Releasing the seam lets the commit observe Closing and lose.
            pause.release();
            let outcome = timeout(TEST_TIMEOUT, exchange_task)
                .await
                .expect("exchange bounded")
                .expect("exchange joined");
            assert_eq!(
                outcome.err().expect("owner close wins the commit gate"),
                UpstreamError::Closed(SideEffectState::Sent)
            );

            // The already-begun async close drains and reaches Closed.
            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closed);
            assert_eq!(upstream.in_flight_exchanges(), 0);

            server_task.await.expect("server task joined");
        });
    }

    #[test]
    fn owner_close_wins_the_commit_gate_for_a_truncated_response() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let id = 0xe002;
            let server_task = reply_once(server, truncated_response_wire(id));

            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let pause = install_pause(&upstream);
            let exchange_task = spawn_exchange(&upstream, query_wire(id));

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("TC exchange reaches the pre-commit gate");
            assert_eq!(upstream.begin_close(), CloseTransition::BeganClosing);
            pause.release();

            let outcome = timeout(TEST_TIMEOUT, exchange_task)
                .await
                .expect("exchange bounded")
                .expect("exchange joined");
            assert_eq!(
                outcome
                    .err()
                    .expect("owner close wins the commit gate for a TC response"),
                UpstreamError::Closed(SideEffectState::Sent)
            );
            assert_eq!(upstream.close().await, CloseResult::Closed);

            server_task.await.expect("server task joined");
        });
    }

    #[test]
    fn response_commit_before_owner_close_is_not_reversed() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let id = 0xe003;
            let expected = response_wire(id, 41);
            let server_task = reply_once(server, expected.clone());

            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let pause = install_pause(&upstream);
            let exchange_task = spawn_exchange(&upstream, query_wire(id));

            timeout(TEST_TIMEOUT, pause.arrived())
                .await
                .expect("exchange reaches the pre-commit gate");
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Open);

            // The commit reaches the gate while the owner is Open and wins.
            pause.release();
            let response = timeout(TEST_TIMEOUT, exchange_task)
                .await
                .expect("exchange bounded")
                .expect("exchange joined")
                .expect("the response commits while the owner is Open");
            assert_eq!(response.response_id(), id);
            assert_eq!(response.wire(), expected.as_slice());
            assert!(!response.truncated());

            // A close that starts afterwards cannot reverse the committed
            // response and still drains the released registration to Closed.
            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.lifecycle_state(), LifecycleState::Closed);
            assert_eq!(upstream.in_flight_exchanges(), 0);

            server_task.await.expect("server task joined");
        });
    }

    #[test]
    fn local_bind_failure_maps_to_connect_without_side_effects() {
        let error = bind_error(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "bind denied",
        ));
        assert_eq!(error, UpstreamError::Connect);
        assert_eq!(error.side_effect(), SideEffectState::NotSent);
    }

    /// The upstream-visible ID of a received query.
    fn wire_id(wire: &[u8]) -> u16 {
        u16::from_be_bytes([wire[0], wire[1]])
    }

    /// `reply` carrying `id`.
    fn with_id(mut reply: Vec<u8>, id: u16) -> Vec<u8> {
        reply[..2].copy_from_slice(&id.to_be_bytes());
        reply
    }

    /// A query whose QTYPE tags the exchange, so concurrent answers can be
    /// told apart after the upstream ID is rewritten.
    fn tagged_query(id: u16, tag: u8) -> Vec<u8> {
        let mut wire = query_wire(id);
        let qtype = wire.len() - 3;
        wire[qtype] = tag;
        wire
    }

    /// A valid answer to `query` (question copied) whose A record ends in
    /// `marker`.
    fn answer_to(query: &[u8], marker: u8) -> Vec<u8> {
        let mut wire = query.to_vec();
        wire[2] = 0x81;
        wire[3] = 0x80;
        wire[6..8].copy_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
        wire.extend_from_slice(&60u32.to_be_bytes());
        wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, marker]);
        wire
    }

    /// An ID different from every one in `taken`.
    fn unissued_id(taken: &[u16]) -> u16 {
        (0..=u16::MAX)
            .map(|offset| taken[0].wrapping_add(offset).wrapping_add(1))
            .find(|id| !taken.contains(id))
            .expect("a free id")
    }

    /// One upstream whose pool keeps a single socket, so every exchange
    /// shares it.
    fn single_socket_upstream(address: SocketAddr) -> Arc<Upstream> {
        let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
        upstream.udp_pool.configure(1, super::SOCKET_LIFETIME);
        upstream
    }

    fn short_context(millis: u64) -> ExchangeContext {
        ExchangeContext::new(
            Instant::now() + Duration::from_millis(millis),
            TransportCancellation::new(),
        )
    }

    fn spawn_with_context(
        upstream: &Arc<Upstream>,
        query: Vec<u8>,
        context: ExchangeContext,
    ) -> tokio::task::JoinHandle<Result<crate::ExchangeResponse, UpstreamError>> {
        let upstream = Arc::clone(upstream);
        tokio::spawn(async move {
            let request = ExchangeRequest::new(&query).expect("valid query");
            upstream.exchange(request, context).await
        })
    }

    /// Receives `count` queries on a blocking thread and returns them with
    /// their source addresses.
    fn receive_queries(
        socket: Arc<UdpSocket>,
        count: usize,
    ) -> JoinHandle<Vec<(Vec<u8>, SocketAddr)>> {
        spawn_blocking(move || {
            let mut buffer = vec![0u8; LEGAL_UDP_PAYLOAD];
            (0..count)
                .map(|_| {
                    let (read, peer) = socket.recv_from(&mut buffer).expect("receive query");
                    (buffer[..read].to_vec(), peer)
                })
                .collect()
        })
    }

    #[test]
    fn upstream_sees_a_fresh_id_and_the_caller_gets_its_own_back() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let client_id = 0x4d4d;
            let query = query_wire(client_id);
            let mut upstream_ids = Vec::new();
            for marker in 0..8 {
                let received = receive_queries(Arc::clone(&server), 1);
                let task = spawn_exchange(&upstream, query.clone());
                let (wire, peer) = received.await.expect("server joined").remove(0);
                // Everything but the ID reaches the upstream unchanged.
                assert_eq!(&wire[2..], &query[2..]);
                upstream_ids.push(wire_id(&wire));
                server
                    .send_to(&response_wire(wire_id(&wire), marker), peer)
                    .expect("send reply");
                let response = timeout(TEST_TIMEOUT, task)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .expect("exchange succeeds");
                assert_eq!(response.request_id(), client_id);
                assert_eq!(response.response_id(), client_id);
                assert_eq!(response.wire(), response_wire(client_id, marker).as_slice());
            }
            assert!(
                upstream_ids.iter().any(|id| *id != client_id),
                "the upstream must not simply see the client's ID"
            );
        });
    }

    #[test]
    fn concurrent_exchanges_on_one_socket_each_get_their_own_answer() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = single_socket_upstream(address);
            let count = 40u8;
            let received = receive_queries(Arc::clone(&server), usize::from(count));
            let tasks: Vec<_> = (1..=count)
                .map(|tag| spawn_exchange(&upstream, tagged_query(0x7000 + u16::from(tag), tag)))
                .collect();
            let queries = received.await.expect("server joined");
            let peer = queries[0].1;
            assert!(queries.iter().all(|(_, from)| *from == peer), "one socket");
            let ids: Vec<u16> = queries.iter().map(|(wire, _)| wire_id(wire)).collect();
            // A stray never-issued ID, then every answer in reverse order,
            // each followed by a duplicate that must be dropped as late.
            server
                .send_to(&response_wire(unissued_id(&ids), 0), peer)
                .expect("send stray");
            for (wire, _) in queries.iter().rev() {
                let tag = wire[wire.len() - 3];
                server
                    .send_to(&answer_to(wire, tag), peer)
                    .expect("send answer");
                server
                    .send_to(&answer_to(wire, 0xff), peer)
                    .expect("send duplicate");
            }
            for (tag, task) in (1..=count).zip(tasks) {
                let response = timeout(TEST_TIMEOUT, task)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .expect("exchange succeeds");
                let wire = response.wire();
                assert_eq!(wire_id(wire), 0x7000 + u16::from(tag));
                assert_eq!(wire[wire.len() - 3 - 16], tag, "own question");
                assert_eq!(wire[wire.len() - 1], tag, "own answer");
            }
            assert_eq!(upstream.udp_pool.live_leases(), 0);
        });
    }

    #[test]
    fn unknown_id_is_recorded_on_every_waiting_exchange_of_the_socket() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = single_socket_upstream(address);
            let received = receive_queries(Arc::clone(&server), 2);
            let first = spawn_with_context(&upstream, query_wire(0x0101), short_context(400));
            let second = spawn_with_context(&upstream, query_wire(0x0202), short_context(400));
            let queries = received.await.expect("server joined");
            let ids: Vec<u16> = queries.iter().map(|(wire, _)| wire_id(wire)).collect();
            server
                .send_to(&response_wire(unissued_id(&ids), 1), queries[0].1)
                .expect("send unknown id");
            for task in [first, second] {
                let error = timeout(TEST_TIMEOUT, task)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .err()
                    .expect("deadline");
                assert_eq!(
                    error.terminal_cause(),
                    Some(TerminalError::DeadlineExceeded(SideEffectState::Sent))
                );
                let ignored = error.ignored_datagrams();
                assert!(ignored.response_id_mismatch());
                assert!(!ignored.unexpected_peer());
            }
        });
    }

    #[test]
    fn wrong_peer_with_a_live_id_marks_only_that_exchange() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let (spoof, _) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = single_socket_upstream(address);
            let received = receive_queries(Arc::clone(&server), 2);
            let first = spawn_with_context(&upstream, query_wire(0x0303), short_context(400));
            let second = spawn_with_context(&upstream, query_wire(0x0404), short_context(400));
            let queries = received.await.expect("server joined");
            // Spoof the live upstream ID of one of the two exchanges from the
            // wrong peer.
            let (wire, peer) = &queries[0];
            spoof
                .send_to(&response_wire(wire_id(wire), 1), *peer)
                .expect("send spoof");
            let mut marked = 0;
            for task in [first, second] {
                let error = timeout(TEST_TIMEOUT, task)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .err()
                    .expect("a spoof is never accepted");
                assert_eq!(
                    error.terminal_cause(),
                    Some(TerminalError::DeadlineExceeded(SideEffectState::Sent))
                );
                let ignored = error.ignored_datagrams();
                assert!(!ignored.response_id_mismatch());
                if ignored.unexpected_peer() {
                    marked += 1;
                }
            }
            assert_eq!(marked, 1, "only the exchange owning the ID is marked");
        });
    }

    #[test]
    fn late_answer_for_an_ended_exchange_is_dropped_without_diagnostics() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = single_socket_upstream(address);
            let cancellation = TransportCancellation::new();
            let received = receive_queries(Arc::clone(&server), 1);
            let first = spawn_with_context(
                &upstream,
                query_wire(0x0505),
                ExchangeContext::new(
                    Instant::now() + Duration::from_secs(30),
                    cancellation.clone(),
                ),
            );
            let (first_wire, peer) = received.await.expect("server joined").remove(0);
            cancellation.cancel();
            assert!(
                timeout(TEST_TIMEOUT, first)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .is_err()
            );

            let received = receive_queries(Arc::clone(&server), 1);
            let second = spawn_with_context(&upstream, query_wire(0x0606), short_context(400));
            let (second_wire, second_peer) = received.await.expect("server joined").remove(0);
            assert_eq!(second_peer, peer, "same pooled socket");
            assert_ne!(wire_id(&second_wire), wire_id(&first_wire));
            // The first exchange's answer arrives late on the shared socket.
            server
                .send_to(&response_wire(wire_id(&first_wire), 2), peer)
                .expect("send late answer");
            let error = timeout(TEST_TIMEOUT, second)
                .await
                .expect("bounded")
                .expect("joined")
                .err()
                .expect("a late answer never completes another exchange");
            assert_eq!(
                error,
                UpstreamError::DeadlineExceeded(SideEffectState::Sent),
                "a late answer is not a diagnostic for others"
            );
        });
    }

    #[test]
    fn a_socket_retires_after_its_exchange_budget() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = single_socket_upstream(address);
            let mut peers = Vec::new();
            for index in 0..=super::EXCHANGES_PER_SOCKET {
                let received = receive_queries(Arc::clone(&server), 1);
                let task = spawn_exchange(&upstream, query_wire(0x0707));
                let (wire, peer) = received.await.expect("server joined").remove(0);
                server
                    .send_to(&response_wire(wire_id(&wire), 3), peer)
                    .expect("send reply");
                timeout(TEST_TIMEOUT, task)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .unwrap_or_else(|error| panic!("exchange {index}: {error}"));
                peers.push(peer);
            }
            let first = &peers[..super::EXCHANGES_PER_SOCKET];
            assert!(first.iter().all(|peer| *peer == first[0]));
            assert_eq!(
                upstream.udp_pool.opened(),
                2,
                "the next exchange opens a new socket"
            );
        });
    }

    #[test]
    fn an_idle_socket_closes_after_its_lifetime() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            upstream.udp_pool.configure(1, Duration::from_millis(100));
            let server = Arc::new(server);
            let received = receive_queries(Arc::clone(&server), 1);
            let task = spawn_exchange(&upstream, query_wire(0x0808));
            let (wire, peer) = received.await.expect("server joined").remove(0);
            server
                .send_to(&response_wire(wire_id(&wire), 4), peer)
                .expect("send reply");
            timeout(TEST_TIMEOUT, task)
                .await
                .expect("bounded")
                .expect("joined")
                .expect("exchange succeeds");
            assert_eq!(upstream.udp_pool.open_sockets(), 1);
            let sockets = upstream.udp_pool.weak_sockets();
            // Retired by age with no exchange left, the receive task ends and
            // the socket closes at once, without waiting for another lease.
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(upstream.udp_pool.open_sockets(), 0);
            assert!(
                sockets.iter().all(|socket| socket.upgrade().is_none()),
                "a retired, drained socket is released"
            );
        });
    }

    #[test]
    fn a_retired_socket_still_finishes_its_exchange() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            upstream.udp_pool.configure(1, Duration::from_millis(50));
            let received = receive_queries(Arc::clone(&server), 1);
            let first = spawn_exchange(&upstream, query_wire(0x0909));
            let (first_wire, first_peer) = received.await.expect("server joined").remove(0);
            tokio::time::sleep(Duration::from_millis(150)).await;
            // The aged socket takes no new exchange.
            let received = receive_queries(Arc::clone(&server), 1);
            let second = spawn_exchange(&upstream, query_wire(0x0a0a));
            let (second_wire, second_peer) = received.await.expect("server joined").remove(0);
            assert_ne!(first_peer, second_peer);
            server
                .send_to(&response_wire(wire_id(&first_wire), 5), first_peer)
                .expect("send first reply");
            server
                .send_to(&response_wire(wire_id(&second_wire), 6), second_peer)
                .expect("send second reply");
            for (task, marker) in [(first, 5), (second, 6)] {
                let response = timeout(TEST_TIMEOUT, task)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .expect("exchange succeeds");
                assert_eq!(response.wire()[response.wire().len() - 1], marker);
            }
            assert_eq!(upstream.udp_pool.opened(), 2);
        });
    }

    #[test]
    fn close_waits_for_receive_tasks_and_releases_every_socket() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            for marker in 0..6 {
                let received = receive_queries(Arc::clone(&server), 1);
                let task = spawn_exchange(&upstream, query_wire(0x0b0b));
                let (wire, peer) = received.await.expect("server joined").remove(0);
                server
                    .send_to(&response_wire(wire_id(&wire), marker), peer)
                    .expect("send reply");
                timeout(TEST_TIMEOUT, task)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .expect("exchange succeeds");
            }
            let sockets = upstream.udp_pool.weak_sockets();
            assert!(!sockets.is_empty());
            assert_eq!(upstream.close().await, CloseResult::Closed);
            assert_eq!(upstream.udp_pool.open_sockets(), 0);
            assert!(
                sockets.iter().all(|socket| socket.upgrade().is_none()),
                "close releases every socket"
            );
        });
    }

    #[test]
    fn concurrent_close_calls_all_wait_for_the_sockets() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            for marker in 0..4 {
                let received = receive_queries(Arc::clone(&server), 1);
                let task = spawn_exchange(&upstream, query_wire(0x1010));
                let (wire, peer) = received.await.expect("server joined").remove(0);
                server
                    .send_to(&response_wire(wire_id(&wire), marker), peer)
                    .expect("send reply");
                timeout(TEST_TIMEOUT, task)
                    .await
                    .expect("bounded")
                    .expect("joined")
                    .expect("exchange succeeds");
            }
            let sockets = upstream.udp_pool.weak_sockets();
            assert!(!sockets.is_empty());
            let released = || sockets.iter().all(|socket| socket.upgrade().is_none());
            // Whichever call returns, every socket must already be closed.
            let (first, second) =
                tokio::join!(async { (upstream.close().await, released()) }, async {
                    (upstream.close().await, released())
                },);
            assert!(
                first.1 && second.1,
                "no close returns before the sockets close"
            );
            assert!(matches!(
                (first.0, second.0),
                (CloseResult::Closed, _) | (_, CloseResult::Closed)
            ));
        });
    }

    #[test]
    fn dropping_an_upstream_aborts_its_receive_tasks() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let received = receive_queries(Arc::clone(&server), 1);
            let task = spawn_exchange(&upstream, query_wire(0x0c0c));
            let (wire, peer) = received.await.expect("server joined").remove(0);
            server
                .send_to(&response_wire(wire_id(&wire), 7), peer)
                .expect("send reply");
            timeout(TEST_TIMEOUT, task)
                .await
                .expect("bounded")
                .expect("joined")
                .expect("exchange succeeds");
            let sockets = upstream.udp_pool.weak_sockets();
            drop(upstream);
            for _ in 0..10 {
                tokio::task::yield_now().await;
            }
            assert!(
                sockets.iter().all(|socket| socket.upgrade().is_none()),
                "drop releases every socket"
            );
        });
    }

    #[test]
    fn an_aborted_exchange_releases_its_lease() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = single_socket_upstream(address);
            let received = receive_queries(Arc::clone(&server), 1);
            let task = spawn_exchange(&upstream, query_wire(0x0d0d));
            let (wire, peer) = received.await.expect("server joined").remove(0);
            assert_eq!(upstream.udp_pool.live_leases(), 1);
            task.abort();
            match task.await {
                Err(error) => assert!(error.is_cancelled()),
                Ok(_) => panic!("the exchange was aborted"),
            }
            assert_eq!(upstream.udp_pool.live_leases(), 0);
            assert_eq!(upstream.in_flight_exchanges(), 0);
            // Its answer arriving now is late and harmless.
            server
                .send_to(&response_wire(wire_id(&wire), 8), peer)
                .expect("send late answer");
            tokio::task::yield_now().await;
        });
    }

    #[test]
    fn unreachable_upstream_waits_for_the_deadline() {
        block_on(async {
            // A bound-then-closed loopback port answers with ICMP port
            // unreachable, which an unconnected socket never reports.
            let address = {
                let (socket, address) = bind_ipv4();
                drop(socket);
                address
            };
            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let task = spawn_with_context(&upstream, query_wire(0x0e0e), short_context(200));
            let error = timeout(TEST_TIMEOUT, task)
                .await
                .expect("bounded")
                .expect("joined")
                .err()
                .expect("deadline");
            assert_eq!(
                error,
                UpstreamError::DeadlineExceeded(SideEffectState::Sent)
            );
        });
    }

    #[test]
    fn truncated_response_is_returned_with_the_caller_id() {
        block_on(async {
            let (server, address) = bind_ipv4();
            let server = Arc::new(server);
            let upstream = Arc::new(Upstream::new(udp_endpoint(address)));
            let received = receive_queries(Arc::clone(&server), 1);
            let task = spawn_exchange(&upstream, query_wire(0x0f0f));
            let (wire, peer) = received.await.expect("server joined").remove(0);
            server
                .send_to(&truncated_response_wire(wire_id(&wire)), peer)
                .expect("send tc");
            let response = timeout(TEST_TIMEOUT, task)
                .await
                .expect("bounded")
                .expect("joined")
                .expect("TC observation");
            assert!(response.truncated());
            assert_eq!(response.wire(), with_id(truncated_response_wire(0), 0x0f0f));
            assert_eq!(response.request_id(), 0x0f0f);
            assert_eq!(response.response_id(), 0x0f0f);
        });
    }
}
