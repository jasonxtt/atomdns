//! Immutable exact and domain-suffix byte keys in one allocation, indexed by a
//! compact open-addressing hash table.
//!
//! Unlike a fingerprint-only table, a hit always compares every byte against
//! the key pool. Most real-world rule sets use the `Packed` 8-byte-per-slot
//! layout (`offset:32 | len:10 | (result+1):14 | tag:8`), while oversized keys,
//! huge byte pools, or high result-id counts seamlessly fall back to `Wide`
//! (`usize` offsets and results) so this representation adds no rule-count,
//! key-length, or text-size admission limit.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

const PACKED_LEN_SHIFT: u32 = 32;
const PACKED_LEN_MASK: u64 = 0x3ff;
const PACKED_MAX_KEY_LEN: usize = 0x3ff;
const PACKED_RESULT_SHIFT: u32 = 42;
const PACKED_RESULT_MASK: u64 = 0x3fff;
// `result + 1` is stored in 14 bits so an occupied slot is always non-zero.
const PACKED_MAX_RESULT: usize = 0x3fff - 1;
const PACKED_TAG_SHIFT: u32 = 56;
const PACKED_META_MASK: u64 =
    (0xff_u64 << PACKED_TAG_SHIFT) | (PACKED_LEN_MASK << PACKED_LEN_SHIFT);

enum CompactTable {
    Empty,
    Packed {
        slots: Box<[u64]>,
        mask: usize,
    },
    Wide {
        slots: Box<[usize]>,
        mask: usize,
        offsets: Box<[usize]>,
        results: Box<[usize]>,
    },
}

pub(crate) struct CompactIndex {
    bytes: Box<[u8]>,
    table: CompactTable,
    root_result: Option<usize>,
    seed: u64,
    len: usize,
    #[cfg(test)]
    force_constant_hash: Option<u64>,
}

/// 64-bit word-folding FxHash with a SplitMix64 finalizer and per-index seed.
#[inline]
fn hash_bytes(seed: u64, bytes: &[u8]) -> u64 {
    const K: u64 = 0xf135_7aea_2e62_a9c5;
    let mut h = seed ^ (bytes.len() as u64).wrapping_mul(K);
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let mut word = [0_u8; 8];
        word.copy_from_slice(chunk);
        h = (h.rotate_left(5) ^ u64::from_ne_bytes(word)).wrapping_mul(K);
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut tail = 0_u64;
        for (i, &b) in rem.iter().enumerate() {
            tail |= u64::from(b) << (i * 8);
        }
        h = (h.rotate_left(5) ^ tail).wrapping_mul(K);
    }
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

fn random_seed() -> u64 {
    RandomState::new().hash_one(0x5a5a_5a5a_a5a5_a5a5_u64)
}

/// Maps a normalized domain name to its canonical forward suffix key, matching
/// the exact empty-label, root, leading-dot, and trailing-dot semantics of
/// [`crate::trie::reverse_label_iter`]:
///
/// - `None` denotes zero labels (the root rule: `""`, `"."`, `".."`).
/// - `Some(canon)` denotes one or more labels (`canon.split('.')` is the exact
///   reverse of `reverse_label_iter(normalized)`).
#[inline]
pub(crate) fn domain_canonical(normalized: &str) -> Option<&str> {
    let s = normalized.strip_suffix('.').unwrap_or(normalized);
    if s.is_empty() {
        None
    } else {
        Some(s.strip_prefix('.').unwrap_or(s))
    }
}

impl CompactIndex {
    pub(crate) fn sort_dedup(keys: &mut Vec<(String, usize)>) {
        // Stable sorting preserves last-write-wins for normalized aliases.
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        keys.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                earlier.1 = later.1;
                true
            } else {
                false
            }
        });
    }

    pub(crate) fn build(mut keys: Vec<(String, usize)>) -> Self {
        Self::sort_dedup(&mut keys);
        Self::build_deduped_inner(keys, None, random_seed(), None)
    }

    /// Builds a forward-key domain suffix index from `(domain_canonical(norm), result)`
    /// pairs, preserving last-write-wins across both root and non-root entries.
    pub(crate) fn build_domain(keys: Vec<(Option<String>, usize)>) -> Self {
        let mut root_result = None;
        let mut non_root = Vec::with_capacity(keys.len());
        for (key, result) in keys {
            match key {
                Some(k) => non_root.push((k, result)),
                None => root_result = Some(result),
            }
        }
        Self::sort_dedup(&mut non_root);
        Self::build_deduped_inner(non_root, root_result, random_seed(), None)
    }

    /// Builds an index from keys already deduplicated by [`Self::sort_dedup`].
    pub(crate) fn build_deduped(keys: Vec<(String, usize)>, root_result: Option<usize>) -> Self {
        Self::build_deduped_inner(keys, root_result, random_seed(), None)
    }

    #[cfg(test)]
    pub(crate) fn build_deduped_with_options(
        keys: Vec<(String, usize)>,
        root_result: Option<usize>,
        seed: u64,
        force_constant_hash: Option<u64>,
    ) -> Self {
        Self::build_deduped_inner(keys, root_result, seed, force_constant_hash)
    }

    fn build_deduped_inner(
        keys: Vec<(String, usize)>,
        root_result: Option<usize>,
        seed: u64,
        #[cfg_attr(not(test), allow(unused_variables))] force_constant_hash: Option<u64>,
    ) -> Self {
        let n = keys.len();
        let total_len = n + usize::from(root_result.is_some());
        if n == 0 {
            return Self {
                bytes: Box::new([]),
                table: CompactTable::Empty,
                root_result,
                seed,
                len: total_len,
                #[cfg(test)]
                force_constant_hash,
            };
        }

        let mut total_bytes = 0_usize;
        let mut max_key_len = 0_usize;
        let mut max_result = 0_usize;
        for (k, r) in &keys {
            total_bytes += k.len();
            max_key_len = max_key_len.max(k.len());
            max_result = max_result.max(*r);
        }

        // Load factor <= 65%, strictly greater than `n` so at least one empty
        // slot always exists to terminate linear probing.
        let needed = n
            .checked_mul(100)
            .map_or(usize::MAX, |v| v / 65 + 1)
            .max(n + 1);
        let cap = needed
            .checked_next_power_of_two()
            .expect("compact index capacity overflow")
            .max(4);
        let mask = cap - 1;

        let use_packed = total_bytes <= u32::MAX as usize
            && max_key_len <= PACKED_MAX_KEY_LEN
            && max_result <= PACKED_MAX_RESULT;

        let mut bytes = Vec::with_capacity(total_bytes);
        let table = if use_packed {
            let mut slots = vec![0_u64; cap];
            for (k, r) in keys {
                let off = bytes.len();
                bytes.extend_from_slice(k.as_bytes());
                let h = Self::compute_hash(seed, k.as_bytes(), force_constant_hash);
                let tag = h >> PACKED_TAG_SHIFT;
                let entry = (off as u64)
                    | ((k.len() as u64) << PACKED_LEN_SHIFT)
                    | (((r + 1) as u64) << PACKED_RESULT_SHIFT)
                    | (tag << PACKED_TAG_SHIFT);
                #[allow(clippy::cast_possible_truncation)]
                let mut idx = (h as usize) & mask;
                while slots[idx] != 0 {
                    idx = (idx + 1) & mask;
                }
                slots[idx] = entry;
            }
            CompactTable::Packed {
                slots: slots.into_boxed_slice(),
                mask,
            }
        } else {
            let mut slots = vec![0_usize; cap];
            let mut offsets = Vec::with_capacity(n + 1);
            let mut results = Vec::with_capacity(n);
            for (entry_idx, (k, r)) in keys.into_iter().enumerate() {
                offsets.push(bytes.len());
                results.push(r);
                bytes.extend_from_slice(k.as_bytes());
                let h = Self::compute_hash(seed, k.as_bytes(), force_constant_hash);
                #[allow(clippy::cast_possible_truncation)]
                let mut idx = (h as usize) & mask;
                while slots[idx] != 0 {
                    idx = (idx + 1) & mask;
                }
                slots[idx] = entry_idx + 1;
            }
            offsets.push(bytes.len());
            CompactTable::Wide {
                slots: slots.into_boxed_slice(),
                mask,
                offsets: offsets.into_boxed_slice(),
                results: results.into_boxed_slice(),
            }
        };

        Self {
            bytes: bytes.into_boxed_slice(),
            table,
            root_result,
            seed,
            len: total_len,
            #[cfg(test)]
            force_constant_hash,
        }
    }

    #[inline]
    fn compute_hash(
        seed: u64,
        key: &[u8],
        #[cfg_attr(not(test), allow(unused_variables))] force_constant_hash: Option<u64>,
    ) -> u64 {
        #[cfg(test)]
        if let Some(forced) = force_constant_hash {
            return forced;
        }
        hash_bytes(seed, key)
    }

    #[inline]
    fn hash_key(&self, key: &[u8]) -> u64 {
        #[cfg(test)]
        if let Some(forced) = self.force_constant_hash {
            return forced;
        }
        hash_bytes(self.seed, key)
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub(crate) fn get(&self, key: &[u8]) -> Option<usize> {
        match &self.table {
            CompactTable::Empty => None,
            CompactTable::Packed { slots, mask } => {
                if key.len() > PACKED_MAX_KEY_LEN {
                    return None;
                }
                let h = self.hash_key(key);
                let tag = h >> PACKED_TAG_SHIFT;
                let want_meta =
                    (tag << PACKED_TAG_SHIFT) | ((key.len() as u64) << PACKED_LEN_SHIFT);
                #[allow(clippy::cast_possible_truncation)]
                let mut idx = (h as usize) & *mask;
                loop {
                    let slot = slots[idx];
                    if slot == 0 {
                        return None;
                    }
                    if (slot & PACKED_META_MASK) == want_meta {
                        #[allow(clippy::cast_possible_truncation)]
                        let off = (slot & 0xffff_ffff) as usize;
                        if &self.bytes[off..off + key.len()] == key {
                            #[allow(clippy::cast_possible_truncation)]
                            let result =
                                ((slot >> PACKED_RESULT_SHIFT) & PACKED_RESULT_MASK) as usize - 1;
                            return Some(result);
                        }
                    }
                    idx = (idx + 1) & *mask;
                }
            }
            CompactTable::Wide {
                slots,
                mask,
                offsets,
                results,
            } => {
                let h = self.hash_key(key);
                #[allow(clippy::cast_possible_truncation)]
                let mut idx = (h as usize) & *mask;
                loop {
                    let raw = slots[idx];
                    if raw == 0 {
                        return None;
                    }
                    let entry_idx = raw - 1;
                    if &self.bytes[offsets[entry_idx]..offsets[entry_idx + 1]] == key {
                        return Some(results[entry_idx]);
                    }
                    idx = (idx + 1) & *mask;
                }
            }
        }
    }

    /// Longest-suffix lookup against an index built with [`Self::build_domain`]
    /// or [`Self::build_deduped`] over [`domain_canonical`] keys.
    #[inline]
    pub(crate) fn suffix_normalized(&self, normalized: &str) -> Option<usize> {
        let Some(canon) = domain_canonical(normalized) else {
            return self.root_result;
        };
        if !matches!(self.table, CompactTable::Empty) {
            let mut s = canon.as_bytes();
            loop {
                if let Some(result) = self.get(s) {
                    return Some(result);
                }
                match s.iter().position(|&b| b == b'.') {
                    Some(dot) => s = &s[dot + 1..],
                    None => break,
                }
            }
        }
        self.root_result
    }

    #[cfg(test)]
    pub(crate) fn is_wide(&self) -> bool {
        matches!(self.table, CompactTable::Wide { .. })
    }

    #[cfg(test)]
    pub(crate) fn build_with_seed(mut keys: Vec<(String, usize)>, seed: u64) -> Self {
        Self::sort_dedup(&mut keys);
        Self::build_deduped_inner(keys, None, seed, None)
    }

    #[cfg(test)]
    pub(crate) fn build_domain_with_seed(
        keys: Vec<(Option<String>, usize)>,
        seed: u64,
        force_constant_hash: Option<u64>,
    ) -> Self {
        let mut root_result = None;
        let mut non_root = Vec::with_capacity(keys.len());
        for (key, result) in keys {
            match key {
                Some(k) => non_root.push((k, result)),
                None => root_result = Some(result),
            }
        }
        Self::sort_dedup(&mut non_root);
        Self::build_deduped_inner(non_root, root_result, seed, force_constant_hash)
    }

    #[cfg(test)]
    pub(crate) fn build_with_constant_hash(
        mut keys: Vec<(String, usize)>,
        constant_hash: u64,
    ) -> Self {
        Self::sort_dedup(&mut keys);
        Self::build_deduped_inner(keys, None, 0, Some(constant_hash))
    }
}

/// Preserve the exact scanner's empty-label/root/leading-dot behavior, after
/// the caller's ordinary normalization. Retained for the test-only reference
/// matchers.
#[cfg(test)]
pub(crate) fn suffix_key(normalized: &str) -> String {
    let mut key = String::with_capacity(normalized.len() + 1);
    for label in crate::trie::reverse_label_iter(normalized) {
        key.push_str(label);
        key.push('.');
    }
    key
}

/// Reference sorted-array `CompactIndex` retained strictly for differential
/// testing against the hash-indexed implementation.
#[cfg(test)]
pub(crate) struct ReferenceCompactIndex {
    bytes: Box<[u8]>,
    offsets: Box<[usize]>,
    results: Box<[usize]>,
}

#[cfg(test)]
impl ReferenceCompactIndex {
    pub(crate) fn build(mut keys: Vec<(String, usize)>) -> Self {
        CompactIndex::sort_dedup(&mut keys);
        let mut bytes = Vec::with_capacity(keys.iter().map(|(s, _)| s.len()).sum());
        let mut offsets = Vec::with_capacity(keys.len() + 1);
        let mut results = Vec::with_capacity(keys.len());
        for (key, result) in keys {
            offsets.push(bytes.len());
            results.push(result);
            bytes.extend_from_slice(key.as_bytes());
        }
        offsets.push(bytes.len());
        Self {
            bytes: bytes.into_boxed_slice(),
            offsets: offsets.into_boxed_slice(),
            results: results.into_boxed_slice(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.results.len()
    }

    pub(crate) fn get(&self, key: &[u8]) -> Option<usize> {
        self.search(key, self.results.len()).ok()
    }

    fn search(&self, key: &[u8], mut hi: usize) -> Result<usize, usize> {
        let mut lo = 0;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.bytes[self.offsets[mid]..self.offsets[mid + 1]].cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Ok(self.results[mid]),
            }
        }
        Err(lo)
    }

    pub(crate) fn suffix(&self, key: &str) -> Option<usize> {
        let mut hi = self.results.len();
        let mut end = key.len();
        loop {
            match self.search(&key.as_bytes()[..end], hi) {
                Ok(result) => return Some(result),
                Err(place) => hi = place,
            }
            if end == 0 {
                return None;
            }
            end = key[..end - 1].rfind('.').map_or(0, |dot| dot + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DomainSuffixMatcher, normalize};

    #[test]
    fn pooled_suffixes_equal_the_scanner_including_normalized_aliases() {
        let rules = [
            ".", "..", "...", "....", "a", "a.", ".a", "..a", "...a", "a..", "a...", "a..b", "B",
            "A.B", "a.b.",
        ];
        let mut oracle = DomainSuffixMatcher::new();
        let mut domain_keys = Vec::new();
        let mut ref_keys = Vec::new();
        for (at, rule) in rules.iter().enumerate() {
            oracle.add(rule, at);
            let norm = normalize(rule);
            domain_keys.push((domain_canonical(&norm).map(str::to_owned), at));
            ref_keys.push((suffix_key(&norm), at));
        }
        let index = CompactIndex::build_domain_with_seed(domain_keys, 0x1234_5678, None);
        let reference = ReferenceCompactIndex::build(ref_keys);
        assert_eq!(index.len(), reference.len());

        for a in ["", ".", "..", "...", "A", "a.", "a..", "b", "z"] {
            for b in ["", ".", "..", "a", "a.", "b", ".b", "..b"] {
                let q = format!("{a}.{b}");
                let norm = normalize(&q);
                let got = index.suffix_normalized(&norm);
                assert_eq!(got, oracle.r#match(&q).copied(), "{q:?}");
                assert_eq!(got, reference.suffix(&suffix_key(&norm)), "{q:?}");
            }
        }
    }

    #[test]
    fn empty_keys_and_last_write_wins_are_exact() {
        let index = CompactIndex::build_with_seed(
            vec![("".into(), 0), ("a".into(), 2), ("".into(), 3)],
            0xdead_beef,
        );
        assert_eq!(index.get(b""), Some(3));
        assert_eq!(index.get(b"a"), Some(2));
        assert_eq!(index.get(b"aa"), None);
        assert_eq!(CompactIndex::build(vec![]).get(b""), None);

        // Result id 0 with empty key and zero tag must not look like an empty slot.
        let zero_zero = CompactIndex::build_with_constant_hash(vec![("".into(), 0)], 0);
        assert_eq!(zero_zero.get(b""), Some(0));
        assert_eq!(zero_zero.get(b"x"), None);
    }

    #[test]
    fn packed_boundaries_and_wide_fallback_match_reference() {
        // Exactly at Packed boundaries: key_len = 1023, result = PACKED_MAX_RESULT.
        let max_packed_key = "a".repeat(PACKED_MAX_KEY_LEN);
        let packed = CompactIndex::build_with_seed(
            vec![
                (max_packed_key.clone(), PACKED_MAX_RESULT),
                ("short".into(), 0),
            ],
            42,
        );
        assert!(!packed.is_wide());
        assert_eq!(
            packed.get(max_packed_key.as_bytes()),
            Some(PACKED_MAX_RESULT)
        );
        assert_eq!(packed.get(b"short"), Some(0));
        // Query longer than PACKED_MAX_KEY_LEN against a Packed table must cleanly return None.
        let over_key = "a".repeat(PACKED_MAX_KEY_LEN + 1);
        assert_eq!(packed.get(over_key.as_bytes()), None);

        // One byte over PACKED_MAX_KEY_LEN triggers Wide fallback.
        let wide_by_len =
            CompactIndex::build_with_seed(vec![(over_key.clone(), 7), ("short".into(), 1)], 42);
        assert!(wide_by_len.is_wide());
        assert_eq!(wide_by_len.get(over_key.as_bytes()), Some(7));
        assert_eq!(wide_by_len.get(b"short"), Some(1));
        assert_eq!(wide_by_len.get(max_packed_key.as_bytes()), None);

        // One above PACKED_MAX_RESULT triggers Wide fallback.
        let over_res = PACKED_MAX_RESULT + 1;
        let wide_by_result = CompactIndex::build_with_seed(
            vec![("a.example".into(), over_res), ("b.example".into(), 0)],
            42,
        );
        assert!(wide_by_result.is_wide());
        assert_eq!(wide_by_result.get(b"a.example"), Some(over_res));
        assert_eq!(wide_by_result.get(b"b.example"), Some(0));
        assert_eq!(wide_by_result.get(b"c.example"), None);

        // Wide fallback on domain suffix index too.
        let wide_domain = CompactIndex::build_domain_with_seed(
            vec![
                (None, over_res),
                (Some(over_key.clone()), over_res + 5),
                (Some("example.test".into()), 3),
            ],
            42,
            None,
        );
        assert!(wide_domain.is_wide());
        assert_eq!(
            wide_domain.suffix_normalized(&format!("sub.{over_key}")),
            Some(over_res + 5)
        );
        assert_eq!(wide_domain.suffix_normalized("x.example.test"), Some(3));
        assert_eq!(
            wide_domain.suffix_normalized("unmatched.invalid"),
            Some(over_res)
        );
    }

    #[test]
    fn worst_case_single_slot_collisions_terminate_and_match_in_packed_and_wide() {
        // Force every key to hash to the very last slot (testing wrap-around too).
        let constant_hash = u64::MAX;
        let mut packed_keys = Vec::new();
        for i in 0..300_usize {
            packed_keys.push((format!("k{i:03}.example"), i));
        }
        packed_keys.push(("".into(), 0));
        let packed = CompactIndex::build_with_constant_hash(packed_keys.clone(), constant_hash);
        assert!(!packed.is_wide());
        for (k, r) in &packed_keys {
            assert_eq!(packed.get(k.as_bytes()), Some(*r), "packed hit {k:?}");
        }
        // Same length, same forced hash + tag, but absent key must terminate and return None.
        for i in 300..500_usize {
            let absent = format!("k{i:03}.example");
            assert_eq!(
                packed.get(absent.as_bytes()),
                None,
                "packed miss {absent:?}"
            );
        }

        // Same collision stress test on Wide fallback.
        let mut wide_keys: Vec<(String, usize)> = (0..300_usize)
            .map(|i| (format!("w{i:03}.example"), PACKED_MAX_RESULT + 1 + i))
            .collect();
        wide_keys.push(("".into(), 0));
        let wide = CompactIndex::build_with_constant_hash(wide_keys.clone(), constant_hash);
        assert!(wide.is_wide());
        for (k, r) in &wide_keys {
            assert_eq!(wide.get(k.as_bytes()), Some(*r), "wide hit {k:?}");
        }
        for i in 300..500_usize {
            let absent = format!("w{i:03}.example");
            assert_eq!(wide.get(absent.as_bytes()), None, "wide miss {absent:?}");
        }

        // Suffix lookup under total hash collision.
        let dom_keys: Vec<(Option<String>, usize)> = (0..200_usize)
            .map(|i| (Some(format!("d{i:03}.example")), i))
            .collect();
        let dom = CompactIndex::build_domain_with_seed(dom_keys, 0, Some(constant_hash));
        for i in 0..200_usize {
            assert_eq!(
                dom.suffix_normalized(&format!("sub.d{i:03}.example")),
                Some(i)
            );
        }
        assert_eq!(dom.suffix_normalized("sub.absent.example"), None);
    }
}
