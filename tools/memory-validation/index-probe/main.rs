//! Standalone diagnostic, excluded from formal timing binaries.
use atomdns_matcher_core::{ValuedDomainMatcher, ValuedRule};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;
struct Count;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn add(n: usize) {
    let now = LIVE.fetch_add(n, Relaxed) + n;
    PEAK.fetch_max(now, Relaxed);
}
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            add(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        unsafe { System.dealloc(p, l) };
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            add(l.size());
        }
        p
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_sub(l.size(), Relaxed);
            add(n);
        }
        q
    }
}
#[global_allocator]
static A: Count = Count;

fn main() {
    let mut rules = Vec::new();
    let mut queries = Vec::new();
    for (i, path) in std::env::args().skip(1).enumerate() {
        let bytes = std::fs::read(&path).unwrap();
        let source = std::path::Path::new(&path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let items = if path.ends_with(".srs") {
            atomdns_matcher_core::decode_domain_srs(&bytes, true)
                .unwrap()
                .rules
        } else {
            String::from_utf8(bytes)
                .unwrap()
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty() && !s.starts_with('#'))
                .map(str::to_owned)
                .collect()
        };
        for rule in items {
            let rule = if rule.contains(':') {
                rule
            } else {
                format!("domain:{rule}")
            };
            if !rule.is_ascii() {
                continue;
            }
            if let Some(name) = rule
                .strip_prefix("domain:")
                .or_else(|| rule.strip_prefix("full:"))
            {
                queries.push(name.to_owned());
            }
            rules.push(ValuedRule::new(rule, 1u64 << i, [i as u32], source, source));
        }
    }
    // Exercise complete result merges rather than boolean membership. Overlap
    // rules are distinct, deterministic metadata and retained in every build.
    rules.push(ValuedRule::new(
        "domain:com",
        1 << 50,
        [90],
        "parent",
        "parent",
    ));
    rules.push(ValuedRule::new(
        "keyword:example",
        1 << 51,
        [91],
        "keyword",
        "keyword",
    ));
    rules.push(ValuedRule::new(
        "regexp:^example[.]",
        1 << 52,
        [92],
        "regex",
        "regex",
    ));
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let start = Instant::now();
    let matcher = ValuedDomainMatcher::build(&rules).unwrap();
    let build_us = start.elapsed().as_micros();
    let live = LIVE.load(Relaxed) - base;
    let peak = PEAK.load(Relaxed) - base;
    let mut checksum = std::collections::hash_map::DefaultHasher::new();
    let start = Instant::now();
    let mut lookups = 0usize;
    for name in queries.iter().step_by((queries.len() / 10000).max(1)) {
        for q in [
            name.clone(),
            format!("child.{name}"),
            format!("{name}.invalid"),
            name.to_ascii_uppercase(),
            format!("{name}."),
        ] {
            black_box(matcher.r#match(black_box(&q))).hash(&mut checksum);
            lookups += 1;
        }
    }
    let lookup_ns = start.elapsed().as_nanos() as f64 / lookups as f64;
    println!(
        "{{\"rules\":{},\"pooled_results\":{},\"live_requested_bytes\":{live},\"build_peak_requested_bytes\":{peak},\"build_us\":{build_us},\"lookups\":{lookups},\"lookup_ns\":{lookup_ns},\"result_checksum\":\"{:016x}\"}}",
        rules.len(),
        matcher.pooled_result_len(),
        checksum.finish()
    );
    // Optional private, byte-for-byte metadata evidence. Keep filesystem I/O
    // outside the heap/build/lookup measurement above; never publish names.
    if let Some(path) = std::env::var_os("ATOMDNS_MEMORY_PROBE_OUTPUT") {
        use std::io::Write;
        let mut output = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        for name in queries.iter().step_by((queries.len() / 10000).max(1)) {
            for q in [
                name.clone(),
                format!("child.{name}"),
                format!("{name}.invalid"),
                name.to_ascii_uppercase(),
                format!("{name}."),
            ] {
                writeln!(output, "{q:?}\t{:?}", matcher.r#match(&q)).unwrap();
            }
        }
        output.flush().unwrap();
    }
}
