//! Requested-allocation diagnostic, never a formal timing binary.
use mosdns_cache_core::{CacheConfig, NativeCache};
use mosdns_native_host::{CacheTestClock, NativeCacheAdapter};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hash::Hasher;
use std::hint::black_box;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Count;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
fn add(bytes: usize) {
    ALLOCATED.fetch_add(bytes, Relaxed);
    let live = LIVE.fetch_add(bytes, Relaxed) + bytes;
    PEAK.fetch_max(live, Relaxed);
}
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            add(layout.size());
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            add(size);
        }
        result
    }
}
#[global_allocator]
static ALLOCATOR: Count = Count;

fn measure<T>(action: impl FnOnce() -> T) -> (T, usize, usize) {
    let live = LIVE.load(Relaxed);
    let allocated = ALLOCATED.load(Relaxed);
    PEAK.store(live, Relaxed);
    let value = action();
    let peak = PEAK.load(Relaxed).saturating_sub(live);
    (value, ALLOCATED.load(Relaxed) - allocated, peak)
}

fn packet(index: usize, large: bool) -> (Vec<u8>, Vec<u8>) {
    let name = format!("item-{index}.fixture.test");
    let kind = if large { 16_u16 } else { 1 };
    let mut query = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        query.push(label.len().try_into().unwrap());
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&kind.to_be_bytes());
    query.extend_from_slice(&1_u16.to_be_bytes());
    let mut wire = query.clone();
    wire[2] = 0x81;
    wire[7] = 1;
    wire.extend_from_slice(&[0xc0, 12]);
    wire.extend_from_slice(&kind.to_be_bytes());
    wire.extend_from_slice(&1_u16.to_be_bytes());
    wire.extend_from_slice(&300_u32.to_be_bytes());
    let data = if large {
        let mut data = Vec::new();
        for _ in 0..16 {
            data.push(255);
            data.extend_from_slice(&[b'x'; 255]);
        }
        data
    } else {
        vec![192, 0, 2, 1]
    };
    wire.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
    wire.extend_from_slice(&data);
    (query, wire)
}

fn run(large: bool) {
    let clock = CacheTestClock::new(0);
    let adapter =
        NativeCacheAdapter::with_options_and_clock(10_000, 600, Rc::new(clock.clone())).unwrap();
    let (mut query, wire) = packet(0, large);
    assert!(
        adapter
            .begin_store(&query)
            .unwrap()
            .unwrap()
            .publish_with_domain(&wire, "route-A")
            .unwrap()
    );
    let mut expected = wire.clone();
    expected[3] |= 0x80;
    for (stage, now) in [("fresh", 0), ("lazy", 301)] {
        clock.set(now);
        if stage == "lazy" {
            expected[query.len() + 6..query.len() + 10].copy_from_slice(&5_u32.to_be_bytes());
        }
        for _ in 0..32 {
            black_box(adapter.lookup(&query).unwrap().unwrap());
        }
        let mut digest = std::collections::hash_map::DefaultHasher::new();
        let (_, allocated, peak) = measure(|| {
            for id in 0..1000_u16 {
                query[..2].copy_from_slice(&id.to_be_bytes());
                let reply = adapter.lookup(&query).unwrap().unwrap();
                assert_eq!(&reply[..2], &id.to_be_bytes());
                assert_eq!(&reply[2..], &expected[2..]);
                digest.write(black_box(&reply));
            }
        });
        assert_eq!(adapter.snapshot().unwrap()[0].response, wire);
        println!(
            "{{\"stage\":\"{stage}\",\"large\":{large},\"replies\":1000,\"wire_bytes\":{},\"allocated_bytes\":{allocated},\"peak_requested_delta\":{peak},\"checksum\":{}}}",
            wire.len(),
            digest.finish()
        );
    }
    clock.set(0);
    let core = NativeCache::new(CacheConfig {
        capacity: 10_000,
        lazy_cache_ttl_secs: 600,
        flags: 0,
    })
    .unwrap();
    for index in 0..2000 {
        let (query, wire) = packet(index, large);
        core.store(&query, &wire, b"route-A", 0, 300, 600).unwrap();
        if index > 0 {
            assert!(
                adapter
                    .begin_store(&query)
                    .unwrap()
                    .unwrap()
                    .publish_with_domain(&wire, "route-A")
                    .unwrap()
            );
        }
    }
    assert_eq!(core.live_len(0), 2000);
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    let (_, allocated, peak) = measure(|| {
        #[cfg(feature = "shared")]
        let snapshot = core.shared_snapshot(0);
        #[cfg(not(feature = "shared"))]
        let snapshot = core.snapshot(0);
        assert_eq!(snapshot.len(), 2000);
        // Make the checksum independent of Moka's randomized iteration order.
        let mut checksums = [0_u64; 2000];
        for (index, entry) in snapshot.iter().enumerate() {
            #[cfg(feature = "shared")]
            let response = entry.response();
            #[cfg(not(feature = "shared"))]
            let response = entry.response.as_slice();
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            hash.write(black_box(response));
            checksums[index] = hash.finish();
        }
        checksums.sort_unstable();
        for checksum in checksums {
            digest.write_u64(checksum);
        }
        black_box(&snapshot);
    });
    println!(
        "{{\"stage\":\"snapshot\",\"large\":{large},\"entries\":2000,\"allocated_bytes\":{allocated},\"peak_requested_delta\":{peak},\"checksum\":{}}}",
        digest.finish()
    );
    let (dump, allocated, peak) = measure(|| adapter.dump().unwrap());
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    digest.write(&dump);
    // Dump entry order is not a persistence contract; compare decoded contents
    // in the codec tests, not gzip hashes across independent cache instances.
    println!(
        "{{\"stage\":\"dump\",\"large\":{large},\"entries\":2000,\"allocated_bytes\":{allocated},\"peak_requested_delta\":{peak},\"compressed_bytes\":{},\"checksum\":{}}}",
        dump.len(),
        digest.finish()
    );
}
fn main() {
    run(false);
    run(true);
}
