//! Product build identity shared by the standalone CLI and HTTP health.
//!
//! A release is built with `ATOMDNS_BUILD_VERSION=vX.Y.Z` (the release tag); any other
//! build reports the crate version in the same form, so every build has a parseable version.
pub const VERSION: &str = match option_env!("ATOMDNS_BUILD_VERSION") {
    Some(version) => version,
    None => concat!("v", env!("CARGO_PKG_VERSION")),
};
