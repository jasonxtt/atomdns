//! Requested-heap regression for a wide/deep valued-rule construction batch.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use atomdns_matcher_core::{ValuedDomainMatcher, ValuedRule};

struct Count;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn add(bytes: usize) {
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
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, bytes) };
        if !next.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            add(bytes);
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Count = Count;

#[test]
fn valued_construction_bounds_peak_and_preserves_results() {
    let rules: Vec<_> = (0..20_000)
        .map(|i| {
            ValuedRule::new(
                format!("domain:leaf{i}.branch{i}.example"),
                1,
                [],
                "tag",
                "source",
            )
        })
        .collect();
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let matcher = ValuedDomainMatcher::build(&rules).expect("valid rules");
    let peak = PEAK.load(Relaxed) - base;
    assert_eq!(matcher.len(), rules.len());
    let result = matcher
        .r#match("x.leaf42.branch42.example.")
        .expect("suffix");
    assert_eq!(result.fast_marks, 1);
    assert_eq!(result.joined_tags, "tag");
    assert_eq!(result.joined_sources, "source");
    assert!(
        peak <= 8 * 1024 * 1024,
        "build requested heap {peak} exceeds8MiB budget"
    );
}
