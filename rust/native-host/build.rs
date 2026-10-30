fn main() {
    println!("cargo:rerun-if-env-changed=ATOMDNS_BUILD_VERSION");
    if let Ok(version) = std::env::var("ATOMDNS_BUILD_VERSION") {
        assert!(
            !version.trim().is_empty() && !version.contains(['\n', '\r']),
            "invalid product version"
        );
        println!("cargo:rustc-env=ATOMDNS_BUILD_VERSION={version}");
    }
}
