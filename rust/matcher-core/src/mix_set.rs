//! A read-only [`MixMatcher<()>`](crate::MixMatcher) kept in sorted byte pools.
//!
//! The mutable matcher spends a hash table and several small allocations per name; this
//! one stores the `full:` and `domain:` rules as two [`CompactIndex`] pools, so a block
//! list of hundreds of thousands of names costs tens of bytes each. Matching answers
//! exactly as `MixMatcher` does (full → domain → regexp → keyword).

use crate::compact::{CompactIndex, suffix_key};
use crate::mix::{
    MATCHER_DOMAIN, MATCHER_FULL, MATCHER_KEYWORD, MATCHER_REGEXP, UnsupportedType, split_rule,
};
use crate::normalize::{NonAsciiRule, normalize};
use crate::regex::compile_go_compatible;

/// Collects rules, then [`finish`](Self::finish)es into a [`MixSet`].
pub struct MixSetBuilder {
    default_type: Option<&'static str>,
    full: Vec<(String, usize)>,
    domain: Vec<(String, usize)>,
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
            MATCHER_DOMAIN => self.domain.push((suffix_key(&normalize(pattern)), 0)),
            MATCHER_KEYWORD => self.keyword.push(normalize(pattern)),
            other => return Err(Box::new(UnsupportedType(other.to_string()))),
        }
        Ok(())
    }

    #[must_use]
    pub fn finish(self) -> MixSet {
        MixSet {
            full: CompactIndex::build(self.full),
            domain: CompactIndex::build(self.domain),
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
        if !domain.is_ascii() {
            return false;
        }
        let d = normalize(domain);
        self.full.get(d.as_bytes()).is_some()
            || self.domain.suffix(&suffix_key(&d)).is_some()
            || self.regex.iter().any(|re| re.is_match(&d))
            || self.keyword.iter().any(|kw| d.contains(kw.as_str()))
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
}
