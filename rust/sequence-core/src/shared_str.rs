use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::{Arc, LazyLock, Weak};

/// An immutable, reference-counted UTF-8 string shared across configuration,
/// sequence routing state, and audit records.
///
/// Cloning is a single atomic reference-count increment (zero heap allocation),
/// equality short-circuits on pointer identity via [`Arc::ptr_eq`], and `Debug`
/// formatting is transparent (identical to `String` / `&str`).
#[derive(Clone)]
pub struct SharedStr(Arc<str>);

const _: () = assert!(std::mem::size_of::<SharedStr>() == 2 * std::mem::size_of::<usize>());
const _: () = assert!(std::mem::size_of::<Option<SharedStr>>() == 2 * std::mem::size_of::<usize>());

impl SharedStr {
    /// Returns the string slice view.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Borrows the underlying [`Arc<str>`].
    #[must_use]
    pub fn as_arc(&self) -> &Arc<str> {
        &self.0
    }

    /// Unwraps the underlying [`Arc<str>`] without cloning.
    #[must_use]
    pub fn into_arc(self) -> Arc<str> {
        self.0
    }

    /// Returns `true` when both values point to the same underlying allocation.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Creates a weak reference to the underlying allocation.
    #[must_use]
    pub fn downgrade(&self) -> Weak<str> {
        Arc::downgrade(&self.0)
    }
}

impl Default for SharedStr {
    fn default() -> Self {
        static EMPTY: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from(""));
        Self(Arc::clone(&EMPTY))
    }
}

impl Deref for SharedStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for SharedStr {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for SharedStr {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq for SharedStr {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

impl Eq for SharedStr {}

impl PartialEq<str> for SharedStr {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for SharedStr {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

impl PartialEq<String> for SharedStr {
    fn eq(&self, other: &String) -> bool {
        &*self.0 == other.as_str()
    }
}

impl PartialEq<SharedStr> for str {
    fn eq(&self, other: &SharedStr) -> bool {
        self == &*other.0
    }
}

impl PartialEq<SharedStr> for &str {
    fn eq(&self, other: &SharedStr) -> bool {
        *self == &*other.0
    }
}

impl PartialEq<SharedStr> for String {
    fn eq(&self, other: &SharedStr) -> bool {
        self.as_str() == &*other.0
    }
}

impl PartialOrd for SharedStr {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SharedStr {
    fn cmp(&self, other: &Self) -> Ordering {
        if Arc::ptr_eq(&self.0, &other.0) {
            Ordering::Equal
        } else {
            (*self.0).cmp(&*other.0)
        }
    }
}

impl Hash for SharedStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self.0).hash(state);
    }
}

impl fmt::Debug for SharedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0, f)
    }
}

impl fmt::Display for SharedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&*self.0, f)
    }
}

impl From<Arc<str>> for SharedStr {
    fn from(value: Arc<str>) -> Self {
        Self(value)
    }
}

impl From<SharedStr> for Arc<str> {
    fn from(value: SharedStr) -> Self {
        value.0
    }
}

impl From<&str> for SharedStr {
    fn from(value: &str) -> Self {
        if value.is_empty() {
            Self::default()
        } else {
            Self(Arc::from(value))
        }
    }
}

impl From<String> for SharedStr {
    fn from(value: String) -> Self {
        if value.is_empty() {
            Self::default()
        } else {
            Self(Arc::from(value))
        }
    }
}

impl From<Box<str>> for SharedStr {
    fn from(value: Box<str>) -> Self {
        if value.is_empty() {
            Self::default()
        } else {
            Self(Arc::from(value))
        }
    }
}

impl From<Cow<'_, str>> for SharedStr {
    fn from(value: Cow<'_, str>) -> Self {
        match value {
            Cow::Borrowed(s) => Self::from(s),
            Cow::Owned(s) => Self::from(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use super::SharedStr;

    fn hash_of<T: Hash + ?Sized>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn clones_share_allocation_and_equal_distinct_allocations() {
        let first = SharedStr::from("main_sequence");
        let cloned = first.clone();
        let separate = SharedStr::from(String::from("main_sequence"));
        let different = SharedStr::from("other_sequence");

        assert!(first.ptr_eq(&cloned));
        assert!(!first.ptr_eq(&separate));
        assert_eq!(first, cloned);
        assert_eq!(first, separate);
        assert_ne!(first, different);
        assert_eq!(first, "main_sequence");
        assert_eq!("main_sequence", first);
        assert_eq!(first, String::from("main_sequence"));
        assert_eq!(String::from("main_sequence"), first);
        assert_eq!(hash_of(&first), hash_of("main_sequence"));
        assert_eq!(hash_of(&first), hash_of(&separate));
        assert_eq!(first.cmp(&cloned), std::cmp::Ordering::Equal);
        assert_eq!(first.cmp(&separate), std::cmp::Ordering::Equal);
        assert!(first < different);
    }

    #[test]
    fn debug_and_display_match_string_and_default_shares_empty() {
        let value = SharedStr::from("中国域名");
        assert_eq!(format!("{value:?}"), format!("{:?}", "中国域名"));
        assert_eq!(format!("{value}"), "中国域名");

        let empty_a = SharedStr::default();
        let empty_b = SharedStr::from("");
        let empty_c = SharedStr::from(String::new());
        assert!(empty_a.is_empty());
        assert!(empty_a.ptr_eq(&empty_b));
        assert!(empty_a.ptr_eq(&empty_c));
    }

    #[test]
    fn arc_conversion_and_weak_release_work_without_copying() {
        let arc: Arc<str> = Arc::from("ephemeral-gen-label");
        let shared = SharedStr::from(Arc::clone(&arc));
        let weak = shared.downgrade();
        assert!(Arc::ptr_eq(&arc, shared.as_arc()));
        assert!(weak.upgrade().is_some());
        drop(arc);
        assert!(weak.upgrade().is_some());
        drop(shared);
        assert!(weak.upgrade().is_none());
    }
}
