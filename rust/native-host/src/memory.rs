//! Keeping the resident size near the live heap across large rule builds.
//!
//! A finished rule build frees the previous matcher generation and the build's
//! temporaries; the allocator must hand those pages back to the kernel or the
//! resident size stays at the build peak.
//!
//! With the `mimalloc` feature (the default; `main.rs` makes it the global
//! allocator) the purge delay is 10 ms instead of mimalloc's 1000 ms, and
//! `release_free_memory` is `mi_collect(true)`: it frees the calling thread's
//! empty pages and purges every arena's freed ranges at once. Measurements:
//! `docs/rust/validation-records/11-24-mimalloc.md`.
//!
//! Without it, glibc builds use three settings (measurements in
//! `docs/rust/validation-records/10-08-mem-1/`):
//!
//! - a fixed mmap threshold, so every allocation of 128 KiB or more (matcher
//!   tables, build vectors) is its own mapping and is unmapped when freed. By
//!   default glibc raises the threshold after such a free, and later large
//!   blocks then come from, and fragment, the heap;
//! - `malloc_trim(0)` after a build is published, which releases every whole
//!   free page of every arena;
//! - no transparent huge pages for this process (for every allocator): with the
//!   common `always` setting, `khugepaged` refills the released pages of any
//!   2 MiB range that still holds one live page, and the resident size climbs
//!   back.
//!
//! Other C libraries and systems without mimalloc: the glibc settings are no-ops.
#![allow(unsafe_code)]

#[cfg(all(target_os = "linux", target_env = "gnu", not(feature = "mimalloc")))]
unsafe extern "C" {
    fn mallopt(param: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
    fn malloc_trim(pad: usize) -> std::ffi::c_int;
}

/// Turns transparent huge pages off for this process. With mimalloc it sets
/// the purge delay to 10 ms (unless `MIMALLOC_PURGE_DELAY` is set); for glibc
/// builds without mimalloc it fixes glibc's mmap threshold at 128 KiB (its
/// initial value), which also stops its dynamic adjustment. Call once at
/// process start, before the configuration is compiled.
pub fn configure_process_memory() {
    #[cfg(target_os = "linux")]
    let _ = rustix::thread::disable_transparent_huge_pages(true);
    #[cfg(feature = "mimalloc")]
    if std::env::var_os("MIMALLOC_PURGE_DELAY").is_none() {
        /// `mi_option_purge_delay` in mimalloc 3.3's `mimalloc.h` (libmimalloc-sys
        /// 0.1.49 binds the function but not this constant).
        const MI_OPTION_PURGE_DELAY: libmimalloc_sys::mi_option_t = 15;
        // mimalloc returns freed pages after 1000 ms by default (arenas 10x
        // that), so a rule build's temporaries pile up into a peak about 30 MiB
        // above glibc's. 10 ms keeps build peaks near glibc's and costs about
        // 8% on the new-domain path; 0 costs about 25%.
        // SAFETY: `mi_option_set` takes no pointers; it stores one option value
        // and is thread-safe.
        unsafe {
            libmimalloc_sys::mi_option_set(MI_OPTION_PURGE_DELAY, 10);
        }
    }
    #[cfg(all(target_os = "linux", target_env = "gnu", not(feature = "mimalloc")))]
    {
        /// `M_MMAP_THRESHOLD` in glibc's `malloc.h`.
        const M_MMAP_THRESHOLD: std::ffi::c_int = -3;
        // SAFETY: `mallopt` takes no pointers; it only changes a glibc
        // allocator parameter and is thread-safe.
        unsafe {
            mallopt(M_MMAP_THRESHOLD, 128 * 1024);
        }
    }
}

/// Returns the allocator's free pages to the kernel. Call it off the DNS
/// thread (a few milliseconds on the production rule set) after a build's
/// results are published and its temporaries and the replaced generation have
/// been dropped.
pub(crate) fn release_free_memory() {
    #[cfg(feature = "mimalloc")]
    {
        // SAFETY: `mi_collect` takes no pointers; it only frees empty pages of
        // the calling thread's heap and purges freed arena ranges, and is
        // thread-safe.
        unsafe {
            libmimalloc_sys::mi_collect(true);
        }
    }
    #[cfg(all(target_os = "linux", target_env = "gnu", not(feature = "mimalloc")))]
    {
        // SAFETY: `malloc_trim` takes no pointers; it only returns free pages
        // of glibc's own arenas to the kernel and is thread-safe.
        unsafe {
            malloc_trim(0);
        }
    }
}
