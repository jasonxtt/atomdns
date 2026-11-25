use std::fmt::{Display, Formatter};

/// Error returned when a matcher rule is outside the ASCII-only Rust boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NonAsciiRule;

impl Display for NonAsciiRule {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("non-ASCII matcher rules are Go-only")
    }
}

impl std::error::Error for NonAsciiRule {}

/// Stack buffer size for per-query ASCII domain normalization.
/// Covers every valid DNS name (max 253 bytes) without heap allocation;
/// longer synthetic inputs fall back to heap allocation.
const STACK_NORMALIZE_MAX: usize = 512;

/// Normalise a domain string to match Go's `NormalizeDomain`:
/// lowercase + strip a single trailing dot.
///
/// ```
/// # use atomdns_matcher_core::normalize;
/// assert_eq!(normalize("GOOGLE.com."), "google.com");
/// assert_eq!(normalize("google.com"), "google.com");
/// assert_eq!(normalize(""), "");
/// assert_eq!(normalize("."), "");
/// assert_eq!(normalize(".."), ".");  // one trailing dot only
/// assert_eq!(normalize("EXAMPLE.."), "example.");
/// ```
#[must_use]
pub fn normalize(s: &str) -> String {
    let trimmed = s.strip_suffix('.').unwrap_or(s);
    trimmed.to_ascii_lowercase()
}

/// Runs `f` on the normalized form of an ASCII domain string without heap
/// allocation when it fits on the stack. Callers must ensure `s.is_ascii()`.
pub(crate) fn with_normalized_ascii<R>(s: &str, f: impl FnOnce(&str) -> R) -> R {
    let trimmed = s.strip_suffix('.').unwrap_or(s);
    if !trimmed.bytes().any(|b| b.is_ascii_uppercase()) {
        return f(trimmed);
    }
    if trimmed.len() <= STACK_NORMALIZE_MAX {
        let mut buf = [0_u8; STACK_NORMALIZE_MAX];
        for (dst, &src) in buf.iter_mut().zip(trimmed.as_bytes()) {
            *dst = src.to_ascii_lowercase();
        }
        let normalized = std::str::from_utf8(&buf[..trimmed.len()])
            .expect("ASCII bytes remain valid UTF-8 after to_ascii_lowercase");
        f(normalized)
    } else {
        let owned = trimmed.to_ascii_lowercase();
        f(&owned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_go_golden() {
        // Copied from pkg/matcher/domain/golden_test.go TestGoldenNormalizeDomain
        let cases = [
            ("google.com", "google.com"),
            ("google.com.", "google.com"),
            ("GOOGLE.com.", "google.com"),
            ("Google.COM", "google.com"),
            ("a.b.C.", "a.b.c"),
            ("", ""),
            (".", ""),
            ("..", "."),
            ("EXAMPLE.", "example"),
            ("EXAMPLE..", "example."),
        ];
        for (input, expected) in &cases {
            assert_eq!(&normalize(input), expected, "normalize({input:?})");
        }
    }
}
