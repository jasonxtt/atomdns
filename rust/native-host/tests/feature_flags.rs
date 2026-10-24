//! M2-1: named feature flags (`feature_flag`) — declarations and their
//! validation, the `flag` matcher, the `flags` section of `settings.json`, the
//! one-time migration from the schema-2 `rule/switchN.txt` files, the
//! `/api/v1/flags` routes with hot apply, and the rule that every settings save
//! keeps the `flags` section (R1). Local DNS stubs stand in for the network.

#![allow(clippy::too_many_lines)]

use std::net::UdpSocket as StdUdpSocket;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use mosdns_native_host::{BoundHost, HostAssembly};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ----- fixtures -----------------------------------------------------------------

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("m2-1-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn text(&self, relative: &str) -> String {
        std::fs::read_to_string(self.0.join(relative)).unwrap_or_default()
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    /// Writes `rule/switchN.txt` for each `(N, value)`.
    fn switches(&self, values: &[(u8, &str)]) {
        for (number, value) in values {
            self.write(&format!("rule/switch{number}.txt"), value);
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn query_packet(name: &str, qtype: u16) -> Vec<u8> {
    let mut packet = vec![0x4b, 0x21, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        packet.push(u8::try_from(label.len()).unwrap());
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&qtype.to_be_bytes());
    packet.extend_from_slice(&[0, 1]);
    packet
}

/// Answers with one A record `192.0.2.<marker>` whatever the question type.
fn answer_for(query: &[u8], marker: u8) -> Vec<u8> {
    let mut end = 12;
    while query[end] != 0 {
        end += 1 + usize::from(query[end]);
    }
    end += 5;
    let mut reply = vec![query[0], query[1], 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
    reply.extend_from_slice(&query[12..end]);
    reply.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, marker]);
    reply
}

/// A UDP DNS stub answering `192.0.2.<marker>` after `delay`.
struct Stub {
    port: u16,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<usize>>,
    thread: Option<JoinHandle<()>>,
}

impl Stub {
    fn start(marker: u8, delay: Duration) -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let port = socket.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(0));
        let (stopped, count) = (Arc::clone(&stop), Arc::clone(&seen));
        let thread = thread::spawn(move || {
            let mut packet = vec![0_u8; 4096];
            while !stopped.load(Ordering::SeqCst) {
                let Ok((length, client)) = socket.recv_from(&mut packet) else {
                    continue;
                };
                *count.lock().unwrap() += 1;
                let reply = answer_for(&packet[..length], marker);
                let socket = socket.try_clone().unwrap();
                thread::spawn(move || {
                    thread::sleep(delay);
                    let _ = socket.send_to(&reply, client);
                });
            }
        });
        Self {
            port,
            stop,
            seen,
            thread: Some(thread),
        }
    }

    fn count(&self) -> usize {
        *self.seen.lock().unwrap()
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

/// The flags of the tests: the shapes of the `config_rust` ones.
const FLAGS: &str = r"  - tag: core_mode
    type: feature_flag
    args:
      label: 核心运行模式
      description: 兼容或安全
      section: mode
      kind: choice
      options:
        - {value: compat, label: 兼容模式}
        - {value: safe, label: 安全模式}
      default: compat
  - tag: dns_mode
    type: feature_flag
    args:
      label: DNS 分流模式
      description: FakeIP 或 RealIP
      section: mode
      kind: choice
      options: [{value: fakeip, label: FakeIP}, {value: realip, label: RealIP}]
      default: fakeip
  - tag: cn_fakeip
    type: feature_flag
    args: {label: 国内 FakeIP, description: 测试, section: mode, kind: bool, default: false}
  - tag: prefer_v4
    type: feature_flag
    args: {label: IPv4 优先, description: 测试, section: mode, kind: bool, default: false}
  - tag: ddns_foreign
    type: feature_flag
    args: {label: DDNS 国外, description: 测试, section: upstream, kind: bool, default: false}
  - tag: block_empty
    type: feature_flag
    args: {label: 空结果屏蔽, description: 测试, section: blocking, kind: bool, default: true}
  - tag: block_blocklist
    type: feature_flag
    args: {label: 黑名单屏蔽, description: 测试, section: blocking, kind: bool, default: true}
  - tag: block_qtypes
    type: feature_flag
    args: {label: 类型屏蔽, description: 测试, section: blocking, kind: bool, default: true}
  - tag: block_ads
    type: feature_flag
    args: {label: 广告屏蔽, description: 测试, section: blocking, kind: bool, default: true}
  - tag: block_aaaa
    type: feature_flag
    args: {label: IPv6 屏蔽, description: 测试, section: blocking, kind: bool, default: false}
  - tag: lazy_cache
    type: feature_flag
    args: {label: 过期缓存 1, description: 测试, section: cache, kind: bool, default: true}
  - tag: full_cache
    type: feature_flag
    args: {label: 过期缓存 2, description: 测试, section: cache, kind: bool, default: true}
  - tag: client_routing
    type: feature_flag
    args:
      label: 客户端分流
      description: 测试
      section: client
      kind: choice
      options:
        - {value: none, label: 不区分}
        - {value: only_listed_proxy, label: 仅名单内客户端走代理}
        - {value: listed_direct, label: 名单内客户端不走代理}
      default: none
";

struct Ports {
    dns: u16,
    api: u16,
}

fn ports() -> Ports {
    Ports {
        dns: free_port(),
        api: free_port(),
    }
}

/// A configuration whose answers show the flags: `safe` (marker 2) or
/// `compat` (marker 1) by `core_mode`; AAAA refused (NXDOMAIN) while
/// `block_aaaa` is on, names under `blocked.example` while `block_blocklist`
/// is on (and not when only `block_empty` changes).
fn config(tree: &Tree, ports: &Ports, compat: &Stub, safe: &Stub) {
    let yaml = format!(
        r#"log: {{level: error}}
api: {{http: "127.0.0.1:{api}"}}
plugins:
{FLAGS}  - tag: main_proxy
    type: socks5_proxy
    args: {{addr: ""}}
  - tag: compat_fwd
    type: forward
    args: {{upstreams: [{{addr: "udp://127.0.0.1:{compat}"}}]}}
  - tag: safe_fwd
    type: forward
    args: {{upstreams: [{{addr: "udp://127.0.0.1:{safe}"}}]}}
  - tag: entry
    type: sequence
    args:
      - matches: ["qtype 28", flag block_aaaa]
        exec: reject 3
      - matches: ["qname blocked.example", flag block_blocklist]
        exec: reject 3
      - matches: ["qname empty.example", flag block_empty]
        exec: reject 3
      - matches: flag core_mode=safe
        exec: $safe_fwd
      - matches: flag core_mode=compat
        exec: $compat_fwd
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:{dns}"}}
"#,
        api = ports.api,
        dns = ports.dns,
        compat = compat.port,
        safe = safe.port,
    );
    tree.write("config.yaml", &yaml);
}

/// A configuration with only the flags and a trivial entry (compile-level tests).
fn flags_only(tree: &Tree, extra: &str, rules: &str) {
    tree.write(
        "config.yaml",
        &format!(
            "log: {{level: error}}\nplugins:\n{FLAGS}{extra}  - tag: fwd\n    type: forward\n    args: {{upstreams: [{{addr: \"udp://127.0.0.1:9\"}}]}}\n  - tag: entry\n    type: sequence\n    args:\n{rules}      - exec: reject 3\n  - tag: listener\n    type: udp_server\n    args: {{entry: entry, listen: \"127.0.0.1:{}\"}}\n",
            free_port()
        ),
    );
}

fn compile(tree: &Tree) -> Result<mosdns_native_host::CompiledConfig, String> {
    mosdns_native_host::load_and_compile(&tree.path("config.yaml"))
        .map_err(|error| error.to_string())
}

/// The load error of a configuration that must not compile.
fn compile_error(tree: &Tree, what: &str) -> String {
    match compile(tree) {
        Ok(_) => panic!("compiled: {what}"),
        Err(error) => error,
    }
}

/// `key -> (value, source)` from the effective report.
fn effective(
    config: &mosdns_native_host::CompiledConfig,
) -> std::collections::BTreeMap<String, (String, String)> {
    config
        .settings_report()
        .iter()
        .filter(|item| item.kind == "feature_flag")
        .map(|item| {
            (
                item.object.clone(),
                (item.value.clone(), item.source.to_owned()),
            )
        })
        .collect()
}

fn value(config: &mosdns_native_host::CompiledConfig, key: &str) -> String {
    effective(config)[key].0.clone()
}

/// Starts the host the way `rtdns start` does (with the startup migration).
fn start(tree: &Tree) -> HostAssembly {
    HostAssembly::from_config_file(&tree.path("config.yaml")).expect("assembles")
}

async fn serve(host: &HostAssembly) -> (TransportCancellation, tokio::task::JoinHandle<()>) {
    let bound: BoundHost = host.bind_host().await.expect("binds");
    let shutdown = TransportCancellation::new();
    let scope = shutdown.clone();
    let task = tokio::task::spawn_local(async move {
        bound.serve(scope).await.expect("clean shutdown");
    });
    (shutdown, task)
}

struct Http {
    status: u16,
    body: String,
}

async fn api(port: u16, method: &str, path: &str, body: &str) -> Http {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("http response");
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Http {
        status,
        body: body.to_owned(),
    }
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

/// The answer's marker; `None` for a refused (non-NOERROR) answer.
async fn marker(port: u16, name: &str, qtype: u16) -> Option<u8> {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket
        .send_to(&query_packet(name, qtype), ("127.0.0.1", port))
        .await
        .unwrap();
    let mut packet = vec![0; 1500];
    let (length, _) = tokio::time::timeout(Duration::from_secs(5), socket.recv_from(&mut packet))
        .await
        .expect("response in time")
        .unwrap();
    let packet = &packet[..length];
    if packet[3] & 15 != 0 || u16::from_be_bytes([packet[6], packet[7]]) == 0 {
        return None;
    }
    Some(packet[length - 1])
}

// ----- declarations and references -----------------------------------------------

#[test]
fn invalid_declarations_are_load_errors_with_their_path() {
    let tree = Tree::new("declarations");
    let declaration =
        |args: &str| format!("  - tag: bad\n    type: feature_flag\n    args: {args}\n");
    for (extra, expected) in [
        (
            "  - tag: Bad\n    type: feature_flag\n    args: {label: a, description: b, section: mode, kind: bool, default: true}\n".to_owned(),
            "must match",
        ),
        (declaration("{description: b, section: mode, kind: bool, default: true}"), "label"),
        (declaration("{label: a, description: ' ', section: mode, kind: bool, default: true}"), "must not be empty"),
        (declaration("{label: a, description: b, section: nowhere, kind: bool, default: true}"), "unknown section"),
        (declaration("{label: a, description: b, section: mode, kind: number, default: 1}"), "unknown kind"),
        (declaration("{label: a, description: b, section: mode, kind: bool, default: 'yes'}"), "expected true or false"),
        (declaration("{label: a, description: b, section: mode, kind: bool, default: true, options: []}"), "no options"),
        (declaration("{label: a, description: b, section: mode, kind: bool, default: true, value: x}"), "unsupported field"),
        (declaration("{label: a, description: b, section: mode, kind: bool, default: true, requery_on_change: 'yes'}"), "requery_on_change"),
        (declaration("{label: a, description: b, section: mode, kind: choice, options: [{value: x, label: X}], default: x}"), "at least two"),
        (declaration("{label: a, description: b, section: mode, kind: choice, options: [{value: x, label: X}, {value: x, label: Y}], default: x}"), "duplicate option"),
        (declaration("{label: a, description: b, section: mode, kind: choice, options: [{value: x, label: X}, {value: y, label: Y}], default: z}"), "not one of the options"),
        (declaration("{label: a, description: b, section: mode, kind: choice, options: [{value: X, label: X}, {value: y, label: Y}], default: y}"), "must match"),
        (
            "  - tag: core_mode\n    type: feature_flag\n    args: {label: a, description: b, section: mode, kind: bool, default: true}\n".to_owned(),
            "duplicate plugin tag",
        ),
    ] {
        flags_only(&tree, &extra, "");
        let error = compile_error(&tree, &extra);
        assert!(error.contains(expected), "{extra}: {error}");
    }
}

#[test]
fn invalid_references_and_removed_switches_are_load_errors() {
    let tree = Tree::new("references");
    for (rules, expected) in [
        (
            "      - matches: flag nothing\n        exec: reject 0\n",
            "unknown feature_flag `nothing`",
        ),
        (
            "      - matches: flag block_aaaa=on\n        exec: reject 0\n",
            "is a bool flag",
        ),
        (
            "      - matches: flag core_mode\n        exec: reject 0\n",
            "is a choice flag",
        ),
        (
            "      - matches: flag core_mode=leaky\n        exec: reject 0\n",
            "not an option of `core_mode`",
        ),
        (
            "      - matches: $core_mode\n        exec: reject 0\n",
            "is not a matcher",
        ),
        ("      - exec: $core_mode\n", "is not executable"),
        (
            "      - matches: \"switch3 'A'\"\n        exec: reject 0\n",
            "removed in configuration schema 3",
        ),
    ] {
        flags_only(&tree, "", rules);
        let error = compile_error(&tree, rules);
        assert!(error.contains(expected), "{rules}: {error}");
    }
    flags_only(
        &tree,
        "  - tag: switch3\n    type: switch3\n    args: {initial_value: rule/switch3.txt}\n",
        "",
    );
    let error = compile_error(&tree, "switch type");
    assert!(
        error.contains("removed in configuration schema 3"),
        "{error}"
    );
    assert!(error.contains("feature_flag"), "{error}");
}

#[test]
fn bad_flag_settings_are_load_errors() {
    let tree = Tree::new("bad-settings");
    flags_only(&tree, "", "");
    for (settings, expected) in [
        (
            r#"{"version":1,"flags":{"nothing":true}}"#,
            "no feature_flag `nothing`",
        ),
        (
            r#"{"version":1,"flags":{"block_aaaa":"on"}}"#,
            "not a valid bool value",
        ),
        (
            r#"{"version":1,"flags":{"core_mode":true}}"#,
            "not a valid choice value",
        ),
        (
            r#"{"version":1,"flags":{"core_mode":"leaky"}}"#,
            "not a valid choice value",
        ),
        (r#"{"version":1,"flags":[]}"#, "invalid settings"),
        // `null` is not "absent" (absent means "not migrated yet").
        (r#"{"version":1,"flags":null}"#, "invalid settings"),
    ] {
        tree.write("webinfo/settings.json", settings);
        let error = compile_error(&tree, settings);
        assert!(error.contains(expected), "{settings}: {error}");
    }
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,"flags":{"core_mode":"safe","block_aaaa":true}}"#,
    );
    let config = compile(&tree).unwrap();
    let flags = effective(&config);
    assert_eq!(flags["core_mode"], ("safe".into(), "settings".into()));
    assert_eq!(flags["block_aaaa"], ("true".into(), "settings".into()));
    assert_eq!(flags["dns_mode"], ("fakeip".into(), "yaml".into()));
}

// ----- the migration table ---------------------------------------------------------

#[test]
fn legacy_switch_files_map_to_the_flags() {
    let tree = Tree::new("legacy-table");
    flags_only(&tree, "", "");
    // Every switch the opposite of the package default.
    tree.switches(&[
        (1, "B"),
        (3, "B"),
        (4, "B"),
        (5, "B"),
        (6, "A"),
        (7, "B"),
        (8, "A"),
        (9, "B"),
        (13, "B"),
        (16, "A"),
        (17, "B"),
        (2, "A"),
        (12, "B"),
    ]);
    let config = compile(&tree).unwrap();
    let flags = effective(&config);
    for (key, expected) in [
        ("core_mode", "safe"),
        ("dns_mode", "realip"),
        ("cn_fakeip", "true"),
        ("prefer_v4", "true"),
        ("ddns_foreign", "true"),
        ("block_empty", "false"),
        ("block_blocklist", "false"),
        ("block_qtypes", "false"),
        ("block_ads", "false"),
        ("block_aaaa", "true"),
        ("lazy_cache", "false"),
        ("full_cache", "false"),
        ("client_routing", "only_listed_proxy"),
    ] {
        assert_eq!(
            flags[key],
            (expected.to_owned(), "legacy".to_owned()),
            "{key}"
        );
    }
    // The package defaults give the YAML defaults (source yaml), and a
    // surrounding blank is trimmed as the switch owners did.
    tree.switches(&[
        (1, "A\n"),
        (3, " A "),
        (4, "A"),
        (5, "A"),
        (6, "B"),
        (7, "A"),
        (8, "B"),
        (9, "A"),
        (13, "A"),
        (16, "B"),
        (17, "A"),
        (2, "B"),
        (12, "B"),
    ]);
    let config = compile(&tree).unwrap();
    assert!(
        effective(&config)
            .values()
            .all(|(_, source)| source == "yaml"),
        "{:?}",
        effective(&config)
    );
}

#[test]
fn client_routing_reproduces_every_switch2_switch12_combination() {
    let tree = Tree::new("legacy-client");
    flags_only(&tree, "", "");
    for (two, twelve, expected) in [
        (Some("A"), Some("B"), "only_listed_proxy"),
        (Some("B"), Some("A"), "listed_direct"),
        (Some("A"), Some("A"), "none"),
        (Some("B"), Some("B"), "none"),
        (Some("A"), Some("x"), "none"),
        (Some("a"), Some("B"), "none"),
        (None, Some("A"), "none"),
        (Some("B"), None, "none"),
        (None, None, "none"),
    ] {
        for (number, value) in [(2, two), (12, twelve)] {
            let path = tree.path(&format!("rule/switch{number}.txt"));
            match value {
                Some(value) => tree.write(&format!("rule/switch{number}.txt"), value),
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        let config = compile(&tree).unwrap();
        assert_eq!(
            value(&config, "client_routing"),
            expected,
            "{two:?} {twelve:?}"
        );
    }
}

#[test]
fn missing_invalid_and_oversized_switch_files_keep_the_defaults() {
    let tree = Tree::new("legacy-invalid");
    flags_only(&tree, "", "");
    tree.switches(&[(3, "C"), (6, ""), (9, "b")]);
    tree.write("rule/switch16.txt", &"A".repeat(1024 * 1024 + 1));
    let config = compile(&tree).unwrap();
    assert_eq!(value(&config, "core_mode"), "compat");
    assert_eq!(value(&config, "block_aaaa"), "false");
    assert_eq!(value(&config, "cn_fakeip"), "false");
    assert_eq!(value(&config, "ddns_foreign"), "false");
    // Missing files (switch1, 4, …) keep the defaults as well.
    assert_eq!(value(&config, "block_empty"), "true");
    assert!(
        effective(&config)
            .values()
            .all(|(_, source)| source == "yaml")
    );
}

#[test]
fn a_declaration_the_table_does_not_fit_is_not_migrated() {
    let tree = Tree::new("legacy-unfit");
    // `core_mode` as a bool does not fit the table's options: left alone.
    tree.write(
        "config.yaml",
        &format!(
            "log: {{level: error}}\nplugins:\n  - tag: core_mode\n    type: feature_flag\n    args: {{label: a, description: b, section: mode, kind: bool, default: false}}\n  - tag: fwd\n    type: forward\n    args: {{upstreams: [{{addr: \"udp://127.0.0.1:9\"}}]}}\n  - tag: entry\n    type: sequence\n    args: [{{exec: reject 3}}]\n  - tag: listener\n    type: udp_server\n    args: {{entry: entry, listen: \"127.0.0.1:{}\"}}\n",
            free_port()
        ),
    );
    tree.switches(&[(3, "B")]);
    let config = compile(&tree).unwrap();
    assert_eq!(
        effective(&config)["core_mode"],
        ("false".into(), "yaml".into())
    );
}

#[test]
fn check_shows_the_legacy_values_and_writes_nothing() {
    let tree = Tree::new("check");
    flags_only(&tree, "", "");
    tree.switches(&[(3, "B"), (6, "A")]);
    let report = mosdns_native_host::check_config(&tree.path("config.yaml")).expect("check runs");
    let text = report.to_string();
    assert!(
        text.contains("EFFECTIVE feature_flag core_mode value=safe source=legacy"),
        "{text}"
    );
    assert!(
        text.contains("EFFECTIVE feature_flag block_aaaa value=true source=legacy"),
        "{text}"
    );
    assert!(
        text.contains("EFFECTIVE feature_flag dns_mode value=fakeip source=yaml"),
        "{text}"
    );
    assert!(!tree.path("webinfo").exists());
}

// ----- the startup migration --------------------------------------------------------

#[test]
fn startup_migrates_once_and_keeps_the_switch_files() {
    let tree = Tree::new("startup");
    flags_only(&tree, "", "");
    tree.switches(&[(3, "B"), (12, "A"), (2, "B"), (16, "A"), (4, "A")]);
    drop(start(&tree));
    let file = json(&tree.text("webinfo/settings.json"));
    assert_eq!(
        file,
        serde_json::json!({
            "version": 1,
            "flags": {"core_mode": "safe", "client_routing": "listed_direct", "ddns_foreign": true}
        })
    );
    // The switch files stay, but are not read again.
    assert_eq!(tree.text("rule/switch3.txt"), "B");
    tree.switches(&[(3, "A"), (6, "A")]);
    drop(start(&tree));
    assert_eq!(json(&tree.text("webinfo/settings.json")), file);
    let config = compile(&tree).unwrap();
    assert_eq!(
        effective(&config)["core_mode"],
        ("safe".into(), "settings".into())
    );
    assert_eq!(
        effective(&config)["block_aaaa"],
        ("false".into(), "yaml".into())
    );
}

#[test]
fn startup_with_default_values_records_an_empty_flags_section() {
    let tree = Tree::new("startup-defaults");
    flags_only(&tree, "", "");
    tree.switches(&[(3, "A"), (6, "B")]);
    drop(start(&tree));
    assert_eq!(
        json(&tree.text("webinfo/settings.json")),
        serde_json::json!({"version": 1, "flags": {}})
    );
    // Other settings already in the file are kept as they were.
    let tree = Tree::new("startup-keep");
    flags_only(
        &tree,
        "  - tag: main_proxy\n    type: socks5_proxy\n    args: {addr: \"\"}\n",
        "",
    );
    tree.write(
        "webinfo/settings.json",
        r#"{"version":1,"proxies":{"main_proxy":{"addr":"192.0.2.9:1080"}}}"#,
    );
    tree.switches(&[(7, "B")]);
    drop(start(&tree));
    assert_eq!(
        json(&tree.text("webinfo/settings.json")),
        serde_json::json!({
            "version": 1,
            "proxies": {"main_proxy": {"addr": "192.0.2.9:1080"}},
            "flags": {"block_ads": false}
        })
    );
}

// ----- the HTTP routes and hot apply -------------------------------------------------

#[test]
fn the_flag_routes_change_the_next_query_and_store_only_differences() {
    let tree = Tree::new("http");
    let ports = ports();
    let compat = Stub::start(1, Duration::from_millis(600));
    let safe = Stub::start(2, Duration::ZERO);
    config(&tree, &ports, &compat, &safe);
    let host = start(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(
            json(&tree.text("webinfo/settings.json")),
            serde_json::json!({"version": 1, "flags": {}})
        );
        let listed = json(&api(ports.api, "GET", "/api/v1/flags", "").await.body);
        assert_eq!(listed["migrated"], true);
        assert_eq!(listed["settings_file"], "webinfo/settings.json");
        let flags = listed["flags"].as_array().unwrap();
        assert_eq!(flags.len(), 13);
        assert_eq!(flags[0]["key"], "core_mode", "declaration order");
        assert_eq!(flags[0]["kind"], "choice");
        assert_eq!(flags[0]["section"], "mode");
        assert_eq!(flags[0]["label"], "核心运行模式");
        assert_eq!(flags[0]["options"][1], serde_json::json!({"value": "safe", "label": "安全模式"}));
        assert_eq!(flags[0]["value"], "compat");
        assert_eq!(flags[0]["modified"], false);
        assert_eq!(flags[0]["source"], "yaml");
        let aaaa = flags.iter().find(|flag| flag["key"] == "block_aaaa").unwrap();
        assert_eq!(aaaa["kind"], "bool");
        assert_eq!(aaaa["default"], false);
        assert!(aaaa.get("options").is_none());
        assert_eq!(marker(ports.dns, "a.example", 1).await, Some(1));

        // A query is in flight on the compat upstream while the change applies.
        let slow = tokio::task::spawn_local({
            let dns = ports.dns;
            async move { marker(dns, "slow.example", 1).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let changed = api(ports.api, "POST", "/api/v1/flags/core_mode", r#"{"value":"safe"}"#).await;
        assert_eq!(changed.status, 200, "{}", changed.body);
        let changed = json(&changed.body);
        assert_eq!(changed["value"], "safe");
        assert_eq!(changed["modified"], true);
        assert_eq!(changed["source"], "settings");
        assert_eq!(marker(ports.dns, "b.example", 1).await, Some(2));
        assert_eq!(slow.await.unwrap(), Some(1), "an in-flight query keeps its generation");
        assert_eq!(
            json(&tree.text("webinfo/settings.json")),
            serde_json::json!({"version": 1, "flags": {"core_mode": "safe"}})
        );

        // Bool flags; the same value again changes nothing.
        assert_eq!(marker(ports.dns, "c.example", 28).await, Some(2));
        let on = api(ports.api, "POST", "/api/v1/flags/block_aaaa", r#"{"value":true}"#).await;
        assert_eq!(on.status, 200, "{}", on.body);
        assert_eq!(marker(ports.dns, "c.example", 28).await, None);
        let again = api(ports.api, "POST", "/api/v1/flags/block_aaaa", r#"{"value":true}"#).await;
        assert_eq!(again.status, 200);
        // block_empty and block_blocklist (one old switch) move separately.
        assert_eq!(marker(ports.dns, "blocked.example", 1).await, None);
        assert_eq!(marker(ports.dns, "empty.example", 1).await, None);
        let off = api(ports.api, "POST", "/api/v1/flags/block_empty", r#"{"value":false}"#).await;
        assert_eq!(off.status, 200, "{}", off.body);
        assert_eq!(marker(ports.dns, "empty.example", 1).await, Some(2));
        assert_eq!(marker(ports.dns, "blocked.example", 1).await, None);
        let off = api(ports.api, "POST", "/api/v1/flags/block_blocklist", r#"{"value":false}"#).await;
        assert_eq!(off.status, 200, "{}", off.body);
        assert_eq!(marker(ports.dns, "blocked.example", 1).await, Some(2));
        assert_eq!(
            json(&tree.text("webinfo/settings.json"))["flags"],
            serde_json::json!({"core_mode": "safe", "block_aaaa": true, "block_empty": false, "block_blocklist": false})
        );

        // Errors change nothing.
        for (method, path, body, status) in [
            ("POST", "/api/v1/flags/nothing", r#"{"value":true}"#, 404),
            ("POST", "/api/v1/flags/core_mode", r#"{"value":"leaky"}"#, 400),
            ("POST", "/api/v1/flags/core_mode", r#"{"value":true}"#, 400),
            ("POST", "/api/v1/flags/block_aaaa", r#"{"value":"true"}"#, 400),
            ("POST", "/api/v1/flags/block_aaaa", r#"{"value":true,"extra":1}"#, 400),
            ("POST", "/api/v1/flags/block_aaaa", "not json", 400),
            ("GET", "/api/v1/flags/block_aaaa", "", 405),
            ("PUT", "/api/v1/flags/block_aaaa", r#"{"value":true}"#, 405),
            ("POST", "/api/v1/flags", r#"{"value":true}"#, 405),
        ] {
            let reply = api(ports.api, method, path, body).await;
            assert_eq!(reply.status, status, "{method} {path} {body}: {}", reply.body);
        }
        // The old switch routes are gone.
        assert_eq!(api(ports.api, "GET", "/plugins/core_mode/show", "").await.status, 404);

        // DELETE returns to the default; a default value is not stored.
        let reset = api(ports.api, "DELETE", "/api/v1/flags/core_mode", "").await;
        assert_eq!(reset.status, 200, "{}", reset.body);
        assert_eq!(json(&reset.body)["source"], "yaml");
        assert_eq!(marker(ports.dns, "d.example", 1).await, Some(1));
        for key in ["block_aaaa", "block_empty"] {
            assert_eq!(api(ports.api, "DELETE", &format!("/api/v1/flags/{key}"), "").await.status, 200);
        }
        let back = api(ports.api, "POST", "/api/v1/flags/block_blocklist", r#"{"value":true}"#).await;
        assert_eq!(back.status, 200);
        assert_eq!(
            json(&tree.text("webinfo/settings.json")),
            serde_json::json!({"version": 1, "flags": {}}),
            "an all-default state keeps the (empty) section"
        );
        let effective = json(&api(ports.api, "GET", "/api/v1/settings/effective", "").await.body);
        assert!(effective["items"].as_array().unwrap().iter().any(|item| item["kind"] == "feature_flag"
            && item["object"] == "core_mode"
            && item["value"] == "compat"));
        let capabilities = json(&api(ports.api, "GET", "/api/v1/capabilities", "").await.body);
        assert_eq!(capabilities["ui_operations"]["flags.manage"]["supported"], true);
        assert!(capabilities["ui_operations"].get("switches.manage").is_none());
        assert!(capabilities.get("switches").is_none());
        shutdown.cancel();
        task.await.unwrap();
    });
    assert!(compat.count() >= 2 && safe.count() >= 2);
}

#[test]
fn concurrent_changes_publish_one_state() {
    let tree = Tree::new("concurrent");
    let ports = ports();
    let compat = Stub::start(1, Duration::ZERO);
    let safe = Stub::start(2, Duration::ZERO);
    config(&tree, &ports, &compat, &safe);
    let host = start(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let first = tokio::task::spawn_local(api(
            ports.api,
            "POST",
            "/api/v1/flags/client_routing",
            r#"{"value":"only_listed_proxy"}"#,
        ));
        let second = tokio::task::spawn_local(api(
            ports.api,
            "POST",
            "/api/v1/flags/client_routing",
            r#"{"value":"listed_direct"}"#,
        ));
        let statuses = [first.await.unwrap().status, second.await.unwrap().status];
        assert!(
            statuses.iter().all(|status| [200, 409].contains(status)),
            "{statuses:?}"
        );
        assert!(statuses.contains(&200), "{statuses:?}");
        let listed = json(&api(ports.api, "GET", "/api/v1/flags", "").await.body);
        let live = listed["flags"]
            .as_array()
            .unwrap()
            .iter()
            .find(|flag| flag["key"] == "client_routing")
            .unwrap()["value"]
            .clone();
        assert_eq!(
            json(&tree.text("webinfo/settings.json"))["flags"]["client_routing"],
            live
        );
        if statuses == [200, 409] {
            assert_eq!(live, "only_listed_proxy");
        } else if statuses == [409, 200] {
            assert_eq!(live, "listed_direct");
        }
        shutdown.cancel();
        task.await.unwrap();
    });
}

/// R1: every other settings save keeps the `flags` section, empty or not, so a
/// restart never migrates the old switch files again.
/// `config`, plus a `requery` plugin and `requery_on_change` on `core_mode`
/// (when `with_requery`) or on `core_mode` only.
fn config_requery(tree: &Tree, ports: &Ports, compat: &Stub, safe: &Stub, with_requery: bool) {
    config(tree, ports, compat, safe);
    let mut yaml = tree.text("config.yaml").replacen(
        "      default: compat\n",
        "      default: compat\n      requery_on_change: true\n",
        1,
    );
    if with_requery {
        yaml = yaml.replacen(
            "  - tag: entry\n",
            "  - tag: requery\n    type: requery\n    args: {file: webinfo/requeryconfig.json}\n  - tag: entry\n",
            1,
        );
    }
    tree.write("config.yaml", &yaml);
}

async fn requery_started(port: u16) -> bool {
    let reply = api(port, "GET", "/plugins/requery/status", "").await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    json(&reply.body)["last_run_start_time"]
        .as_str()
        .is_some_and(|time| !time.starts_with("0001-01-01"))
}

#[test]
fn a_flagged_change_starts_a_requery_task_once_it_is_applied() {
    let tree = Tree::new("requery-on-change");
    let ports = ports();
    let compat = Stub::start(1, Duration::ZERO);
    let safe = Stub::start(2, Duration::ZERO);
    config_requery(&tree, &ports, &compat, &safe, true);
    let host = start(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let listed = json(&api(ports.api, "GET", "/api/v1/flags", "").await.body);
        let flags = listed["flags"].as_array().unwrap();
        assert_eq!(flags[0]["requery_on_change"], true);
        assert_eq!(flags[1]["requery_on_change"], false);
        assert!(!requery_started(ports.api).await);

        // An unflagged flag and a value that does not change start nothing.
        let plain = api(
            ports.api,
            "POST",
            "/api/v1/flags/block_aaaa",
            r#"{"value":true}"#,
        )
        .await;
        assert_eq!(plain.status, 200, "{}", plain.body);
        assert_eq!(json(&plain.body)["requery_triggered"], false);
        let unchanged = api(
            ports.api,
            "POST",
            "/api/v1/flags/core_mode",
            r#"{"value":"compat"}"#,
        )
        .await;
        assert_eq!(unchanged.status, 200, "{}", unchanged.body);
        assert_eq!(json(&unchanged.body)["requery_triggered"], false);
        let absent = api(ports.api, "DELETE", "/api/v1/flags/core_mode", "").await;
        assert_eq!(json(&absent.body)["requery_triggered"], false);
        assert!(!requery_started(ports.api).await);

        // A rejected request starts nothing.
        let bad = api(
            ports.api,
            "POST",
            "/api/v1/flags/core_mode",
            r#"{"value":"leaky"}"#,
        )
        .await;
        assert_eq!(bad.status, 400);
        assert!(!requery_started(ports.api).await);

        // A change starts one, on the generation that was just published.
        let changed = api(
            ports.api,
            "POST",
            "/api/v1/flags/core_mode",
            r#"{"value":"safe"}"#,
        )
        .await;
        assert_eq!(changed.status, 200, "{}", changed.body);
        let changed = json(&changed.body);
        assert_eq!(changed["value"], "safe");
        assert_eq!(changed["requery_triggered"], true);
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !requery_started(ports.api).await {
            assert!(std::time::Instant::now() < deadline, "no task started");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        // Back to the default is a change too.
        let reset = api(ports.api, "DELETE", "/api/v1/flags/core_mode", "").await;
        assert_eq!(reset.status, 200, "{}", reset.body);
        assert_eq!(json(&reset.body)["value"], "compat");
        assert!(json(&reset.body)["requery_triggered"].is_boolean());
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_flagged_change_without_a_requery_plugin_is_not_an_error() {
    let tree = Tree::new("requery-absent");
    let ports = ports();
    let compat = Stub::start(1, Duration::ZERO);
    let safe = Stub::start(2, Duration::ZERO);
    config_requery(&tree, &ports, &compat, &safe, false);
    let host = start(&tree);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let changed = api(
            ports.api,
            "POST",
            "/api/v1/flags/core_mode",
            r#"{"value":"safe"}"#,
        )
        .await;
        assert_eq!(changed.status, 200, "{}", changed.body);
        let changed = json(&changed.body);
        assert_eq!(changed["value"], "safe");
        assert_eq!(changed["requery_triggered"], false);
        assert_eq!(marker(ports.dns, "a.example", 1).await, Some(2));
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn other_settings_saves_keep_the_flags_section() {
    for stored in [
        serde_json::json!({}),
        serde_json::json!({"core_mode": "safe"}),
    ] {
        let tree = Tree::new("keep-flags");
        let ports = ports();
        let compat = Stub::start(1, Duration::ZERO);
        let safe = Stub::start(2, Duration::ZERO);
        config(&tree, &ports, &compat, &safe);
        // The old switch files disagree with the stored state.
        tree.switches(&[
            (
                3,
                if stored.as_object().unwrap().is_empty() {
                    "B"
                } else {
                    "A"
                },
            ),
            (6, "A"),
        ]);
        tree.write(
            "webinfo/settings.json",
            &serde_json::json!({"version": 1, "flags": stored}).to_string(),
        );
        let host = start(&tree);
        host.block_on(async {
            let (shutdown, task) = serve(&host).await;
            let saved = api(
                ports.api,
                "POST",
                "/api/v1/overrides",
                r#"{"socks5":"192.0.2.9:1080","ecs":""}"#,
            )
            .await;
            assert_eq!(saved.status, 200, "{}", saved.body);
            assert_eq!(
                json(&tree.text("webinfo/settings.json")),
                serde_json::json!({
                    "version": 1,
                    "proxies": {"main_proxy": {"addr": "192.0.2.9:1080"}},
                    "flags": stored
                })
            );
            shutdown.cancel();
            task.await.unwrap();
        });
        drop(host);
        // A restart does not migrate again: the stored values hold.
        drop(start(&tree));
        let file = json(&tree.text("webinfo/settings.json"));
        assert_eq!(file["flags"], stored);
        let config = compile(&tree).unwrap();
        let expected = if stored.as_object().unwrap().is_empty() {
            "compat"
        } else {
            "safe"
        };
        assert_eq!(value(&config, "core_mode"), expected);
        assert_eq!(value(&config, "block_aaaa"), "false");
    }
}

/// Before the first migration (as during an update trial), the first change
/// stores every legacy value along with it.
#[test]
fn a_change_before_the_migration_stores_the_legacy_values_too() {
    let tree = Tree::new("pre-migration");
    let ports = ports();
    let compat = Stub::start(1, Duration::ZERO);
    let safe = Stub::start(2, Duration::ZERO);
    config(&tree, &ports, &compat, &safe);
    tree.switches(&[(3, "B"), (16, "A")]);
    // `load_and_compile` + `with_options` is the path without the startup migration.
    let compiled = mosdns_native_host::load_and_compile(&tree.path("config.yaml")).unwrap();
    let host =
        HostAssembly::with_options(compiled, mosdns_native_host::HostOptions::default()).unwrap();
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let listed = json(&api(ports.api, "GET", "/api/v1/flags", "").await.body);
        assert_eq!(listed["migrated"], false);
        assert_eq!(listed["flags"][0]["source"], "legacy");
        assert_eq!(marker(ports.dns, "a.example", 1).await, Some(2));
        let changed = api(ports.api, "POST", "/api/v1/flags/block_aaaa", r#"{"value":true}"#).await;
        assert_eq!(changed.status, 200, "{}", changed.body);
        assert_eq!(
            json(&tree.text("webinfo/settings.json")),
            serde_json::json!({"version": 1, "flags": {"core_mode": "safe", "ddns_foreign": true, "block_aaaa": true}})
        );
        assert_eq!(marker(ports.dns, "b.example", 1).await, Some(2));
        shutdown.cancel();
        task.await.unwrap();
    });
}
