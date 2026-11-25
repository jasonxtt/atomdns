//! A read-only [`MixMatcher<()>`](crate::MixMatcher) kept in compact hash-indexed byte pools.
//!
//! The mutable matcher spends a per-node hash table and several small allocations per name;
//! this one stores the `full:` and `domain:` rules as two [`CompactIndex`] pools, so a block
//! list of hundreds of thousands of names costs tens of bytes each. Matching answers
//! exactly as `MixMatcher` does (full → domain → regexp → keyword).

use crate::compact::{CompactIndex, domain_canonical};
use crate::mix::{
    MATCHER_DOMAIN, MATCHER_FULL, MATCHER_KEYWORD, MATCHER_REGEXP, UnsupportedType, split_rule,
};
use crate::normalize::{NonAsciiRule, normalize, with_normalized_ascii};
use crate::regex::compile_go_compatible;

/// Collects rules, then [`finish`](Self::finish)es into a [`MixSet`].
pub struct MixSetBuilder {
    default_type: Option<&'static str>,
    full: Vec<(String, usize)>,
    domain: Vec<(Option<String>, usize)>,
    regex: Vec<regex::Regex>,
    keyword: Vec<String>,
}

impl MixSetBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            default_type: None,
            full: Vec::new(),
            domain: Vec::new(),
            regex: Vec::new(),
            keyword: Vec::new(),
        }
    }

    /// The type of a bare rule without a `type:` prefix (`"full"` or `"domain"`).
    pub fn set_default(&mut self, type_name: &'static str) {
        self.default_type = Some(type_name);
    }

    /// Adds a rule string, rejecting exactly what `MixMatcher::add` rejects.
    ///
    /// # Errors
    ///
    /// A bad regexp, an unsupported type prefix, a non-ASCII rule, or a bare rule
    /// without a default type.
    pub fn add(&mut self, rule: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (type_name, pattern) = split_rule(rule, self.default_type)?;
        if !pattern.is_ascii() {
            return Err(Box::new(NonAsciiRule));
        }
        match type_name {
            MATCHER_REGEXP => self.regex.push(compile_go_compatible(pattern)?),
            MATCHER_FULL => self.full.push((normalize(pattern), 0)),
            MATCHER_DOMAIN => {
                let norm = normalize(pattern);
                self.domain
                    .push((domain_canonical(&norm).map(str::to_owned), 0));
            }
            MATCHER_KEYWORD => self.keyword.push(normalize(pattern)),
            other => return Err(Box::new(UnsupportedType(other.to_string()))),
        }
        Ok(())
    }

    #[must_use]
    pub fn finish(self) -> MixSet {
        MixSet {
            full: CompactIndex::build(self.full),
            domain: CompactIndex::build_domain(self.domain),
            regex: self.regex,
            keyword: self.keyword,
        }
    }
}

impl Default for MixSetBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// The immutable matcher built by [`MixSetBuilder`].
pub struct MixSet {
    full: CompactIndex,
    domain: CompactIndex,
    regex: Vec<regex::Regex>,
    keyword: Vec<String>,
}

impl MixSet {
    #[must_use]
    pub fn is_match(&self, domain: &str) -> bool {
        if !domain.is_ascii() || self.is_empty() {
            return false;
        }
        with_normalized_ascii(domain, |d| {
            self.full.get(d.as_bytes()).is_some()
                || self.domain.suffix_normalized(d).is_some()
                || self.regex.iter().any(|re| re.is_match(d))
                || self.keyword.iter().any(|kw| d.contains(kw.as_str()))
        })
    }

    /// The number of distinct rules, counted as `MixMatcher::len` does.
    #[must_use]
    pub fn len(&self) -> usize {
        self.full.len() + self.domain.len() + self.regex.len() + self.keyword.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MixMatcher;
    use crate::compact::{ReferenceCompactIndex, suffix_key};

    /// Reference sorted-array `MixSet` retained strictly for differential tests.
    struct ReferenceMixSet {
        full: ReferenceCompactIndex,
        domain: ReferenceCompactIndex,
        regex: Vec<regex::Regex>,
        keyword: Vec<String>,
    }

    impl ReferenceMixSet {
        fn build(rules: &[String], default_type: Option<&'static str>) -> Self {
            let mut full = Vec::new();
            let mut domain = Vec::new();
            let mut regex = Vec::new();
            let mut keyword = Vec::new();
            for rule in rules {
                let (type_name, pattern) = split_rule(rule, default_type).expect("valid rule");
                match type_name {
                    MATCHER_REGEXP => regex.push(compile_go_compatible(pattern).unwrap()),
                    MATCHER_FULL => full.push((normalize(pattern), 0)),
                    MATCHER_DOMAIN => domain.push((suffix_key(&normalize(pattern)), 0)),
                    MATCHER_KEYWORD => keyword.push(normalize(pattern)),
                    _ => unreachable!(),
                }
            }
            Self {
                full: ReferenceCompactIndex::build(full),
                domain: ReferenceCompactIndex::build(domain),
                regex,
                keyword,
            }
        }

        fn len(&self) -> usize {
            self.full.len() + self.domain.len() + self.regex.len() + self.keyword.len()
        }

        fn is_match(&self, domain: &str) -> bool {
            if !domain.is_ascii() {
                return false;
            }
            let d = normalize(domain);
            self.full.get(d.as_bytes()).is_some()
                || self.domain.suffix(&suffix_key(&d)).is_some()
                || self.regex.iter().any(|re| re.is_match(&d))
                || self.keyword.iter().any(|kw| d.contains(kw.as_str()))
        }
    }

    struct SplitMix64(u64);

    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn usize(&mut self, bound: usize) -> usize {
            #[allow(clippy::cast_possible_truncation)]
            {
                (self.next() as usize) % bound
            }
        }
    }

    fn random_label(rng: &mut SplitMix64, max_len: usize) -> String {
        const ALPHABET: &[u8] = b"abcdefgh0123-AB";
        let len = 1 + rng.usize(max_len);
        let mut s = String::with_capacity(len);
        for _ in 0..len {
            s.push(char::from(ALPHABET[rng.usize(ALPHABET.len())]));
        }
        s
    }

    #[test]
    fn answers_and_counts_like_the_mutable_matcher() {
        let rules = [
            "a.example",
            "domain:B.Example.",
            "full:Exact.Test",
            "full:exact.test",
            "full:",
            "domain:",
            "domain:..",
            "domain:trail.example...",
            "regexp:^re[0-9]+\\.test$",
            "keyword:tracker",
            "domain:dup.example",
            "domain:DUP.example.",
            "bad:type",
            "regexp:(",
            "domain:é.test",
        ];
        let mut oracle = MixMatcher::<()>::new();
        oracle.set_default("domain");
        let mut set = MixSetBuilder::new();
        set.set_default("domain");
        for rule in rules {
            assert_eq!(
                oracle.add(rule, ()).is_ok(),
                set.add(rule).is_ok(),
                "{rule:?}"
            );
        }
        let set = set.finish();
        assert_eq!(set.len(), oracle.len());
        let names = [
            "",
            ".",
            "..",
            "a.example",
            "x.a.example",
            "example",
            "b.example.",
            "X.B.EXAMPLE",
            "exact.test",
            "EXACT.TEST.",
            "sub.exact.test",
            "re12.test",
            "re.test",
            "sub.re12.test",
            "my-tracker.net",
            "tracker",
            "dup.example",
            "z.dup.example",
            "trail.example.",
            "trail.example",
            "é.test",
            "x.é.test",
            "other.org",
            "a",
            "a.",
            ".a.",
            "..a",
        ];
        for name in names {
            assert_eq!(
                set.is_match(name),
                oracle.r#match(name).is_some(),
                "{name:?}"
            );
        }
        assert!(MixSetBuilder::new().finish().is_empty());
    }

    #[test]
    fn differential_100k_queries_against_reference_mix_set() {
        const SEED: u64 = 0x51a9_0001_2026_1125;
        let mut rng = SplitMix64(SEED);
        let tlds = ["test", "example", "invalid", "arpa", "net"];

        let mut rules: Vec<String> = Vec::new();
        let mut domain_bases: Vec<String> = Vec::new();
        let mut full_bases: Vec<String> = Vec::new();

        // Populate realistic + tricky boundary rules.
        for _ in 0..400 {
            let l1 = random_label(&mut rng, 8);
            let l2 = tlds[rng.usize(tlds.len())];
            let base = if rng.usize(4) == 0 {
                format!("{}.{l1}.{l2}", random_label(&mut rng, 5))
            } else {
                format!("{l1}.{l2}")
            };
            if rng.usize(5) == 0 {
                full_bases.push(base.clone());
                let suffix = ["", ".", ".."][rng.usize(3)];
                rules.push(format!("full:{base}{suffix}"));
            } else {
                domain_bases.push(base.clone());
                let prefix = ["", ".", ".."][rng.usize(3)];
                let suffix = ["", ".", "..", "..."][rng.usize(4)];
                rules.push(format!("domain:{prefix}{base}{suffix}"));
            }
        }
        // Add duplicates/aliases, empty-label quirks, keywords, and regexps.
        rules.extend([
            "domain:a..b.test".to_owned(),
            "domain:A..B.TEST.".to_owned(),
            "domain:...".to_owned(),
            "full:exact.example.".to_owned(),
            "full:EXACT.EXAMPLE".to_owned(),
            "keyword:track-hit".to_owned(),
            "keyword:CDN-EDGE".to_owned(),
            "regexp:^srv[0-9]+\\.example$".to_owned(),
            "regexp:\\.lan$".to_owned(),
        ]);

        let mut builder = MixSetBuilder::new();
        builder.set_default("domain");
        for rule in &rules {
            builder.add(rule).expect("generated rule is valid");
        }
        let set = builder.finish();
        let reference = ReferenceMixSet::build(&rules, Some("domain"));
        assert_eq!(set.len(), reference.len(), "seed={SEED}");

        // 100,000 deterministic queries mixing exact hits, suffix hits, near-misses,
        // boundary dot/case forms, keyword/regexp hits, and random misses.
        for case_idx in 0..100_000_usize {
            let query = match rng.usize(8) {
                // Exact/suffix hit on a real domain rule with random prefix and case.
                0 | 1 => {
                    let base = &domain_bases[rng.usize(domain_bases.len())];
                    let prefix = if rng.usize(2) == 0 {
                        format!("{}.", random_label(&mut rng, 6))
                    } else {
                        String::new()
                    };
                    let trail = ["", ".", ".."][rng.usize(3)];
                    format!("{prefix}{base}{trail}")
                }
                // Near-miss: flip one character of a real domain rule.
                2 => {
                    let mut bytes = domain_bases[rng.usize(domain_bases.len())]
                        .clone()
                        .into_bytes();
                    let pos = rng.usize(bytes.len());
                    bytes[pos] = if bytes[pos] == b'z' { b'q' } else { b'z' };
                    let trail = ["", "."][rng.usize(2)];
                    format!("{}{trail}", String::from_utf8(bytes).unwrap())
                }
                // Exact hit or near-miss on a full rule.
                3 => {
                    let base = &full_bases[rng.usize(full_bases.len())];
                    match rng.usize(3) {
                        0 => format!("{base}."),
                        1 => format!("sub.{base}."),
                        _ => base.to_ascii_uppercase(),
                    }
                }
                // Keyword / regexp hit or near-miss.
                4 => match rng.usize(4) {
                    0 => format!("a.track-hit.{}.test", random_label(&mut rng, 4)),
                    1 => format!("a.cdn-edge.{}.test", random_label(&mut rng, 4)),
                    2 => format!("srv{}.example.", rng.usize(500)),
                    _ => format!("{}.lan.", random_label(&mut rng, 6)),
                },
                // Boundary dot / empty-label quirks.
                5 => {
                    let quirks = [
                        "",
                        ".",
                        "..",
                        "...",
                        "....",
                        ".a..b.test",
                        "x.a..b.test.",
                        "a..b.test..",
                        "..a..b.test",
                        "exact.example",
                        "exact.example.",
                        "exact.example..",
                    ];
                    quirks[rng.usize(quirks.len())].to_owned()
                }
                // Completely random domain.
                _ => {
                    let depth = 1 + rng.usize(4);
                    let mut parts = Vec::with_capacity(depth);
                    for _ in 0..depth - 1 {
                        parts.push(random_label(&mut rng, 7));
                    }
                    parts.push(tlds[rng.usize(tlds.len())].to_owned());
                    let trail = ["", "."][rng.usize(2)];
                    format!("{}{trail}", parts.join("."))
                }
            };

            let got = set.is_match(&query);
            let want = reference.is_match(&query);
            assert_eq!(
                got, want,
                "MixSet diff mismatch at case={case_idx} seed={SEED} query={query:?}"
            );
        }

        // Also exercise Wide fallback in MixSet with an oversized >1023-byte rule.
        let huge_label = "a.".repeat(600);
        let huge_rule = format!("domain:{huge_label}example");
        let wide_rules = vec![huge_rule.clone(), "domain:normal.test".to_owned()];
        let mut wide_builder = MixSetBuilder::new();
        for r in &wide_rules {
            wide_builder.add(r).unwrap();
        }
        let wide_set = wide_builder.finish();
        assert!(wide_set.domain.is_wide());
        let wide_ref = ReferenceMixSet::build(&wide_rules, None);
        for q in [
            format!("sub.{huge_label}example."),
            format!("{huge_label}example"),
            format!("sub.{huge_label}other"),
            "x.normal.test.".to_owned(),
            "miss.invalid.".to_owned(),
        ] {
            assert_eq!(wide_set.is_match(&q), wide_ref.is_match(&q), "wide {q:?}");
        }
    }
}
