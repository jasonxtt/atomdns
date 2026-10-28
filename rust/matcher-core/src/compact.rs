//! Immutable exact byte keys in one allocation, with a sorted offset/result table.
//!
//! Unlike a fingerprint, a hit compares every byte. Offsets use usize so this
//! representation adds no rule-count or text-size admission limit.

pub(crate) struct CompactIndex {
    bytes: Box<[u8]>,
    offsets: Box<[usize]>,
    results: Box<[usize]>,
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

    pub(crate) fn get(&self, key: &[u8]) -> Option<usize> {
        let mut lo = 0;
        let mut hi = self.results.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.bytes[self.offsets[mid]..self.offsets[mid + 1]].cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(self.results[mid]),
            }
        }
        None
    }

    pub(crate) fn suffix(&self, key: &str) -> Option<usize> {
        let mut end = key.len();
        loop {
            if let Some(result) = self.get(&key.as_bytes()[..end]) {
                return Some(result);
            }
            if end == 0 {
                return None;
            }
            end = key[..end - 1].rfind('.').map_or(0, |dot| dot + 1);
        }
    }
}

/// Preserve the exact scanner's empty-label/root/leading-dot behavior, after
/// the caller's ordinary normalization. A trailing separator distinguishes a
/// single empty label from zero labels (the root rule).
pub(crate) fn suffix_key(normalized: &str) -> String {
    let mut key = String::with_capacity(normalized.len() + 1);
    for label in crate::trie::reverse_labels(normalized) {
        key.push_str(label);
        key.push('.');
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DomainSuffixMatcher, normalize};

    #[test]
    fn pooled_suffixes_equal_the_scanner_including_normalized_aliases() {
        let rules = [
            ".", "..", "a", "a.", ".a", "..a", "a..", "a...", "a..b", "B", "A.B", "a.b.",
        ];
        let mut oracle = DomainSuffixMatcher::new();
        let mut keys = Vec::new();
        for (at, rule) in rules.iter().enumerate() {
            oracle.add(rule, at);
            keys.push((suffix_key(&normalize(rule)), at));
        }
        let index = CompactIndex::build(keys);
        for a in ["", ".", "..", "A", "a.", "a..", "b", "z"] {
            for b in ["", ".", "a", "a.", "b", ".b", "..b"] {
                let q = format!("{a}.{b}");
                assert_eq!(
                    index.suffix(&suffix_key(&normalize(&q))),
                    oracle.r#match(&q).copied(),
                    "{q:?}"
                );
            }
        }
    }

    #[test]
    fn empty_keys_and_last_write_wins_are_exact() {
        let index = CompactIndex::build(vec![("".into(), 1), ("a".into(), 2), ("".into(), 3)]);
        assert_eq!(index.get(b""), Some(3));
        assert_eq!(index.get(b"a"), Some(2));
        assert_eq!(index.get(b"aa"), None);
        assert_eq!(CompactIndex::build(vec![]).get(b""), None);
    }
}
