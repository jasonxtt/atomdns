//! `route_learner` / `route_learn`: learning by condition, the merge with a
//! `domain_mapper`, per-entry expiry, the state file, the management routes,
//! the hand-over to a new generation and the `requery` flush rule. Real UDP
//! queries and the real management listener; the upstream is a loopback mock.
//! No test reaches the public network.

#![allow(clippy::too_many_lines)]

use std::net::UdpSocket as StdUdpSocket;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mosdns_native_host::{BoundHost, CompiledConfig, HostAssembly, HostOptions};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ----- fixtures -------------------------------------------------------------

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("route-learner-{name}-{}", std::process::id()));
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

/// A loopback upstream that answers by the first label of the name:
/// `direct*` A -> 192.0.2.7, `proxy*` A -> 198.51.100.7, `servfail*` -> rcode 2,
/// `nx*` -> rcode 3, `v6yes*` AAAA -> `2001:db8::1`, anything else A ->
/// 203.0.113.9 and AAAA -> an empty NOERROR.
struct Upstream {
    address: String,
    seen: Arc<Mutex<Vec<(String, u16)>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

fn question(packet: &[u8]) -> (String, u16, usize) {
    let mut position = 12;
    let mut labels = Vec::new();
    while packet[position] != 0 {
        let length = usize::from(packet[position]);
        labels.push(
            String::from_utf8_lossy(&packet[position + 1..position + 1 + length]).into_owned(),
        );
        position += 1 + length;
    }
    let end = position + 5;
    let qtype = u16::from_be_bytes([packet[end - 4], packet[end - 3]]);
    (labels.join("."), qtype, end)
}

impl Upstream {
    fn start() -> Self {
        let socket = StdUdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let address = socket.local_addr().unwrap().to_string();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, stopped) = (Arc::clone(&seen), Arc::clone(&stop));
        let thread = thread::spawn(move || {
            let mut packet = vec![0_u8; 4096];
            while !stopped.load(Ordering::SeqCst) {
                let Ok((length, client)) = socket.recv_from(&mut packet) else {
                    continue;
                };
                let query = &packet[..length];
                let (name, qtype, end) = question(query);
                record.lock().unwrap().push((name.clone(), qtype));
                let answer: Option<Vec<u8>> = if name.starts_with("direct") && qtype == 1 {
                    Some(vec![192, 0, 2, 7])
                } else if name.starts_with("proxy") && qtype == 1 {
                    Some(vec![198, 51, 100, 7])
                } else if name.starts_with("v6yes") && qtype == 28 {
                    let mut data = vec![0x20, 0x01, 0x0d, 0xb8];
                    data.extend([0; 11]);
                    data.push(1);
                    Some(data)
                } else if qtype == 1 {
                    Some(vec![203, 0, 113, 9])
                } else {
                    None
                };
                let rcode: u8 = if name.starts_with("servfail") {
                    2
                } else if name.starts_with("nx") {
                    3
                } else {
                    0
                };
                let answer = answer.filter(|_| rcode == 0);
                let mut reply = vec![
                    query[0],
                    query[1],
                    0x81,
                    0x80 | rcode,
                    0,
                    1,
                    0,
                    u8::from(answer.is_some()),
                    0,
                    0,
                    0,
                    0,
                ];
                reply.extend_from_slice(&query[12..end]);
                if let Some(data) = answer {
                    reply.extend_from_slice(&[0xc0, 0x0c]);
                    reply.extend_from_slice(&qtype.to_be_bytes());
                    reply.extend_from_slice(&[0, 1, 0, 0, 0, 60]);
                    reply.extend_from_slice(&u16::try_from(data.len()).unwrap().to_be_bytes());
                    reply.extend_from_slice(&data);
                }
                let _ = socket.send_to(&reply, client);
            }
        });
        Self {
            address,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    fn count(&self, name: &str, qtype: u16) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, t)| n == name && *t == qtype)
            .count()
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

const CATEGORIES: &str = r#"      categories:
        - {tag: my_sv4list, mark: 4, label: "SPECIAL", source: "r_special", ttl: 1000}
        - {tag: my_realiplist, mark: 11, label: "DIRECT", source: "r_direct", ttl: 100}
        - {tag: my_fakeiplist, mark: 12, label: "PROXY", source: "r_proxy", ttl: 100}
        - {tag: my_nov6list, mark: 3, label: "NOV6", ttl: 60}
        - {tag: my_nov4list, mark: 2, label: "NOV4", ttl: 60}
"#;

/// The host. Learned marks answer: proxy (12) -> rcode 12, direct (11) ->
/// rcode 11, special (4) -> rcode 4, no-V6 (3) on AAAA -> rcode 6; otherwise
/// the upstream is asked and the rules learn from its answer.
fn config(tree: &Tree, ports: &Ports, upstream: &Upstream, extra: &str, learner_args: &str) {
    let yaml = format!(
        r#"log: {{level: error}}
api: {{http: "127.0.0.1:{api}"}}
plugins:
  - tag: fwd
    type: forward
    args: {{upstreams: [{{addr: "udp://{upstream}"}}]}}
  - tag: cache
    type: cache
    args: {{dump_file: "cache/c.gz"}}
  - {{tag: wl, type: domain_set_light, args: {{files: ["rule/wl.txt"]}}}}
  - tag: unified
    type: domain_mapper
    args:
      default_mark: 17
      default_tag: "UNMATCHED"
      rules:
        - {{tag: wl, mark: 8, output_tag: "STATIC"}}
  - tag: route_learner
    type: route_learner
    args:
      target_mapper: unified
      file: gen/route_learner.json
      write_interval: 3600
{learner_args}{CATEGORIES}
  - {{tag: learn_direct, type: route_learn, args: {{learner: route_learner, learn: [my_realiplist], when: ["qtype 1", "resp_ip 192.0.2.0/24"]}}}}
  - {{tag: learn_proxy, type: route_learn, args: {{learner: route_learner, learn: [my_fakeiplist, my_sv4list], when: ["qtype 1", "resp_ip 198.51.100.0/24"]}}}}
  - {{tag: learn_nov6, type: route_learn, args: {{learner: route_learner, learn: my_nov6list, when: ["qtype 28", "rcode 0 3", "!resp_ip 2000::/3"]}}}}
{extra}  - tag: entry
    type: sequence
    args:
      - exec: $unified
      - matches: fast_mark 12
        exec: reject 12
      - matches: fast_mark 11
        exec: reject 11
      - matches: fast_mark 4
        exec: reject 4
      - matches: [fast_mark 3, "qtype 28"]
        exec: reject 6
      - exec: $cache
      - exec: $fwd
      - exec: $learn_direct
      - exec: $learn_proxy
      - exec: $learn_nov6
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: "127.0.0.1:{dns}"}}
"#,
        api = ports.api,
        dns = ports.dns,
        upstream = upstream.address,
    );
    tree.write("config.yaml", &yaml);
    tree.write("rule/wl.txt", "domain:static.test\n");
    std::fs::create_dir_all(tree.path("cache")).unwrap();
}

fn compile(tree: &Tree) -> CompiledConfig {
    mosdns_native_host::load_and_compile(&tree.path("config.yaml")).expect("compiles")
}

fn assemble(config: CompiledConfig) -> HostAssembly {
    HostAssembly::with_options(config, HostOptions::default()).expect("assembles")
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
    headers: Vec<(String, String)>,
    body: String,
}

impl Http {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
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
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    Http {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn wire(name: &str, qtype: u16) -> Vec<u8> {
    let mut out = vec![0, 7, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.trim_end_matches('.').split('.') {
        out.push(u8::try_from(label.len()).unwrap());
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out.extend(qtype.to_be_bytes());
    out.extend([0, 1]);
    out
}

async fn rcode_of(port: u16, name: &str, qtype: u16) -> u8 {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket
        .send_to(&wire(name, qtype), ("127.0.0.1", port))
        .await
        .unwrap();
    let mut packet = vec![0; 1500];
    let (length, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut packet))
        .await
        .expect("response in time")
        .unwrap();
    assert!(length >= 12);
    packet[3] & 15
}

async fn a(port: u16, name: &str) -> u8 {
    rcode_of(port, name, 1).await
}

async fn aaaa(port: u16, name: &str) -> u8 {
    rcode_of(port, name, 28).await
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

// ----- tests ------------------------------------------------------------------

#[test]
fn rules_learn_by_condition_and_the_next_query_follows_the_learned_route() {
    let tree = Tree::new("rules");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        // direct: learned from a 192.0.2.0/24 answer; the next query is routed
        // by the learned mark and never reaches the upstream again.
        assert_eq!(a(ports.dns, "direct1.test").await, 0);
        assert_eq!(a(ports.dns, "Direct1.Test").await, 11);
        assert_eq!(upstream.count("direct1.test", 1), 1);
        // A name whose answer matches no rule is not learned.
        assert_eq!(a(ports.dns, "plain.test").await, 0);
        assert_eq!(a(ports.dns, "plain.test").await, 0);
        // One rule can learn into several categories; the proxy mark wins in
        // the sequence.
        assert_eq!(a(ports.dns, "proxy1.test").await, 0);
        assert_eq!(a(ports.dns, "proxy1.test").await, 12);
        let special = api(ports.api, "GET", "/plugins/my_sv4list/show", "").await;
        assert!(special.body.contains("proxy1.test"), "{}", special.body);
        // no-V6: an empty NOERROR and an NXDOMAIN answer to AAAA are learned,
        // a SERVFAIL one is not (the rule asks for rcode 0 or 3), and an
        // answer with a global IPv6 address is not.
        assert_eq!(aaaa(ports.dns, "v6none.test").await, 0);
        assert_eq!(aaaa(ports.dns, "v6none.test").await, 6);
        assert_eq!(aaaa(ports.dns, "nx1.test").await, 3);
        assert_eq!(aaaa(ports.dns, "nx1.test").await, 6);
        assert_eq!(aaaa(ports.dns, "servfail1.test").await, 2);
        assert_eq!(aaaa(ports.dns, "servfail1.test").await, 2);
        assert_eq!(aaaa(ports.dns, "v6yes1.test").await, 0);
        assert_eq!(aaaa(ports.dns, "v6yes1.test").await, 0);
        // The learned no-V6 mark does not affect an A query for the same name.
        assert_eq!(a(ports.dns, "v6none.test").await, 0);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_warm_response_keeps_the_current_learned_label_in_the_same_generation() {
    let tree = Tree::new("warm-label");
    let ports = ports();
    let upstream = Upstream::start();
    config(
        &tree,
        &ports,
        &upstream,
        "  - tag: empty_mapper\n    type: domain_mapper\n    args: {rules: []}\n",
        "",
    );
    let yaml = tree
        .text("config.yaml")
        .replace(
            "      - matches: fast_mark 11\n        exec: reject 11\n",
            "",
        )
        .replace(
            "entry: entry, listen:",
            "enable_audit: true, entry: entry, listen:",
        )
        .replace(
            "      - exec: $unified\n",
            "      - exec: $unified\n      - exec: $empty_mapper\n",
        );
    tree.write("config.yaml", &yaml);
    let compiled = compile(&tree);
    let mapper = compiled.domain_mappers[0].clone();
    let host = assemble(compiled);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(a(ports.dns, "direct.static.test").await, 0);
        assert_eq!(
            mapper
                .evaluate(Some("direct.static.test."))
                .domain_set
                .as_deref(),
            Some("DIRECT|STATIC")
        );
        assert_eq!(a(ports.dns, "direct.static.test").await, 0);
        assert_eq!(upstream.count("direct.static.test", 1), 1);
        let audit = host.audit_snapshot();
        let warm = audit
            .records
            .iter()
            .find(|row| row.cache_status == mosdns_native_host::CacheStatus::Hit)
            .expect("warm audit row");
        assert_eq!(warm.matched_rule_source.as_deref(), Some("r_direct|wl"));
        assert_eq!(warm.domain_set.as_deref(), Some("DIRECT|STATIC"));
        assert_eq!(warm.effective_tag.as_deref(), Some("STATIC"));
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn cached_learning_labels_follow_expiry_and_flush_in_every_execution_path() {
    for mode in ["ordinary", "branch", "direct", "nested", "empty"] {
        let tree = Tree::new(&format!("labels-{mode}"));
        let ports = ports();
        let upstream = Upstream::start();
        config(&tree, &ports, &upstream, "", "");
        let yaml = tree.text("config.yaml");
        let yaml = if mode == "direct" {
            yaml.replace("label: \"DIRECT\"", "label: \" DIRECT \"")
                .replace(
                    "default_tag: \"UNMATCHED\"",
                    "default_tag: \"UNMATCHED|FALLBACK\"",
                )
        } else if mode == "empty" {
            yaml.replace("default_tag: \"UNMATCHED\"", "default_tag: \"\"")
        } else {
            yaml
        };
        let prefix = yaml.split("  - tag: entry\n").next().unwrap();
        let successor =
            "      - exec: $fwd\n      - exec: $learn_direct\n      - exec: $learn_proxy\n";
        let (extra, entry) = match mode {
            "ordinary" | "empty" => (String::new(), format!("      - exec: $cache\n{successor}")),
            "nested" => (
                "  - tag: outer\n    type: cache\n    args: {size: 16}\n".to_owned(),
                format!("      - exec: $outer\n      - exec: $cache\n{successor}"),
            ),
            "branch" => (
                format!(
                    "  - tag: cached\n    type: sequence\n    args:\n      - exec: $cache\n{successor}  - tag: fb\n    type: fallback\n    args: {{primary: $cached, secondary: $fwd}}\n"
                ),
                "      - exec: $fb\n".to_owned(),
            ),
            _ => (
                "  - tag: fb\n    type: fallback\n    args: {primary: $cache, secondary: $fwd}\n"
                    .to_owned(),
                format!("      - exec: $fb\n{successor}"),
            ),
        };
        tree.write("config.yaml", &format!("{prefix}{extra}  - tag: entry\n    type: sequence\n    args:\n      - exec: $unified\n{entry}  - tag: listener\n    type: udp_server\n    args: {{enable_audit: true, entry: entry, listen: '127.0.0.1:{dns}'}}\n", dns=ports.dns));
        let compiled = compile(&tree);
        let learner = Rc::clone(&compiled.local.learners[0].runtime);
        let host = assemble(compiled);
        host.block_on(async {
            let (shutdown, task) = serve(&host).await;
            let default_label = if mode == "direct" {
                Some("UNMATCHED|FALLBACK")
            } else if mode == "empty" {
                None
            } else {
                Some("UNMATCHED")
            };
            for (name, learned, cold, source, category, seconds) in [
                (
                    "direct.static.test",
                    "DIRECT|STATIC",
                    Some("STATIC"),
                    Some("wl"),
                    "my_realiplist",
                    101,
                ),
                (
                    "direct.example",
                    "DIRECT",
                    default_label,
                    None,
                    "my_realiplist",
                    101,
                ),
                (
                    "proxy.static.test",
                    "SPECIAL|PROXY|STATIC",
                    Some("STATIC"),
                    Some("wl"),
                    "my_fakeiplist",
                    1001,
                ),
            ] {
                // Cold query learns; the next query is a real warm hit.
                assert_eq!(a(ports.dns, name).await, 0);
                let original_effective = host
                    .audit_snapshot()
                    .records
                    .last()
                    .unwrap()
                    .effective_tag
                    .clone();
                assert_eq!(a(ports.dns, name).await, 0);
                assert_eq!(
                    host.audit_snapshot()
                        .records
                        .last()
                        .unwrap()
                        .domain_set
                        .as_deref(),
                    Some(learned),
                    "{mode} {name}"
                );
                let entries = json(
                    &api(
                        ports.api,
                        "GET",
                        &format!("/plugins/route_learner/entries?category={category}"),
                        "",
                    )
                    .await
                    .body,
                );
                let row = entries["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["name"] == name)
                    .unwrap();
                assert_eq!(
                    row["hits"], 1,
                    "cache restoration must not look up the learner twice"
                );
                if mode == "nested" {
                    // An outer miss around an inner hit must not persist the
                    // query-local annotation as a new effective route.
                    api(ports.api, "GET", "/plugins/outer/flush", "").await;
                    assert_eq!(a(ports.dns, name).await, 0);
                    assert_eq!(a(ports.dns, name).await, 0);
                    assert_eq!(
                        host.audit_snapshot().records.last().unwrap().effective_tag,
                        original_effective
                    );
                }
                // Refill the cache while the learning label is already live,
                // so expiry must remove a label actually stored in the cache.
                flush_label_caches(ports.api, mode == "nested").await;
                assert_eq!(a(ports.dns, name).await, 0);
                let cached_effective = host
                    .audit_snapshot()
                    .records
                    .last()
                    .unwrap()
                    .effective_tag
                    .clone();
                let count = upstream.count(name, 1);
                learner.advance_clock(seconds);
                assert_eq!(a(ports.dns, name).await, 0);
                let audit = host.audit_snapshot();
                let expired = audit.records.last().unwrap();
                assert_eq!(expired.domain_set.as_deref(), cold, "expired {mode} {name}");
                assert_eq!(expired.matched_rule_source.as_deref(), source);
                assert_eq!(expired.effective_tag, cached_effective);
                assert_eq!(
                    upstream.count(name, 1),
                    count,
                    "expiry retained the cached response"
                );
                // Learn again, refill with the learned prefix, then flush the
                // category without flushing the response cache.
                flush_label_caches(ports.api, mode == "nested").await;
                assert_eq!(a(ports.dns, name).await, 0);
                flush_label_caches(ports.api, mode == "nested").await;
                assert_eq!(a(ports.dns, name).await, 0);
                for tag in [category, "my_sv4list"] {
                    assert_eq!(
                        api(ports.api, "GET", &format!("/plugins/{tag}/flush"), "")
                            .await
                            .status,
                        200
                    );
                }
                let count = upstream.count(name, 1);
                assert_eq!(a(ports.dns, name).await, 0);
                let audit = host.audit_snapshot();
                let flushed = audit.records.last().unwrap();
                assert_eq!(flushed.domain_set.as_deref(), cold, "flushed {mode} {name}");
                assert_eq!(flushed.matched_rule_source.as_deref(), source);
                assert_eq!(upstream.count(name, 1), count);
            }
            shutdown.cancel();
            task.await.unwrap();
        });
    }
}

async fn flush_label_caches(api_port: u16, nested: bool) {
    api(api_port, "GET", "/plugins/cache/flush", "").await;
    if nested {
        api(api_port, "GET", "/plugins/outer/flush", "").await;
    }
}

#[test]
fn the_mapper_merges_learned_facts_first_and_never_applies_its_default() {
    let tree = Tree::new("merge");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    let compiled = compile(&tree);
    let mapper = compiled.domain_mappers[0].clone();
    let learner = Rc::clone(&compiled.local.learners[0].runtime);
    let host = assemble(compiled);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        // Learned only.
        a(ports.dns, "direct2.test").await;
        let only = mapper.evaluate(Some("Direct2.Test."));
        assert!(only.matched);
        assert_eq!(only.fast_flags, 1 << 11);
        assert_eq!(only.domain_set.as_deref(), Some("DIRECT"));
        assert_eq!(only.matched_rule_source.as_deref(), Some("r_direct"));
        // Learned in two categories: the order of the categories, not of the
        // learning, decides the order of the labels.
        a(ports.dns, "proxy2.test").await;
        let two = mapper.evaluate(Some("proxy2.test."));
        assert_eq!(two.fast_flags, (1 << 4) | (1 << 12));
        assert_eq!(two.domain_set.as_deref(), Some("SPECIAL|PROXY"));
        // Learned and in a static list: the learned label comes first, the
        // marks are or-ed.
        a(ports.dns, "direct3.static.test").await;
        let both = mapper.evaluate(Some("direct3.static.test."));
        assert_eq!(both.fast_flags, (1 << 11) | (1 << 8));
        assert_eq!(both.domain_set.as_deref(), Some("DIRECT|STATIC"));
        assert_eq!(both.matched_rule_source.as_deref(), Some("r_direct|wl"));
        // Static only, and nothing at all.
        let fixed = mapper.evaluate(Some("other.static.test."));
        assert_eq!(fixed.domain_set.as_deref(), Some("STATIC"));
        let none = mapper.evaluate(Some("nothing.test."));
        assert!(!none.matched);
        assert_eq!(none.fast_flags, 1 << 17);
        assert_eq!(none.domain_set.as_deref(), Some("UNMATCHED"));
        // Expiry is per entry and per category: direct (ttl 100) is gone after
        // 101 s, special (ttl 1000) is still there.
        learner.advance_clock(101);
        let later = mapper.evaluate(Some("proxy2.test."));
        assert_eq!(later.fast_flags, 1 << 4);
        assert_eq!(later.domain_set.as_deref(), Some("SPECIAL"));
        let gone = mapper.evaluate(Some("direct2.test."));
        assert!(!gone.matched);
        // An expired name is learned afresh by the next query's rule (once
        // the cached answer is out of the way: a cache hit ends the flow
        // before the rules).
        api(ports.api, "GET", "/plugins/cache/flush", "").await;
        assert_eq!(a(ports.dns, "direct2.test").await, 0);
        assert_eq!(a(ports.dns, "direct2.test").await, 11);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn show_save_flush_entries_and_status() {
    let tree = Tree::new("api");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    let compiled = compile(&tree);
    let learner = Rc::clone(&compiled.local.learners[0].runtime);
    let host = assemble(compiled);
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        for name in ["direct-a.test", "direct-b.test", "direct-c.test"] {
            a(ports.dns, name).await;
        }
        // Hits count the queries the learned route answered.
        for _ in 0..3 {
            a(ports.dns, "direct-b.test").await;
        }
        a(ports.dns, "direct-c.test").await;

        let shown = api(ports.api, "GET", "/plugins/my_realiplist/show", "").await;
        assert_eq!(shown.status, 200);
        assert_eq!(
            shown.header("Content-Type"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(shown.header("X-Total-Count"), Some("3"));
        assert_eq!(
            shown.header("Access-Control-Expose-Headers"),
            Some("X-Total-Count")
        );
        let lines: Vec<&str> = shown.body.lines().collect();
        assert_eq!(lines.len(), 3, "{}", shown.body);
        // By hits, then name; the layout of a statistics line.
        let parsed: Vec<(String, String, String)> = lines
            .iter()
            .map(|line| {
                let fields: Vec<&str> = line.split(' ').collect();
                assert_eq!(fields.len(), 3, "{line}");
                assert_eq!(fields[0].len(), 10, "{line}");
                (
                    fields[0].to_owned(),
                    fields[1].to_owned(),
                    fields[2].to_owned(),
                )
            })
            .collect();
        assert_eq!(parsed[0].0, "0000000003");
        assert_eq!(parsed[0].2, "direct-b.test");
        assert_eq!(parsed[1].0, "0000000001");
        assert_eq!(parsed[1].2, "direct-c.test");
        assert_eq!(parsed[2].2, "direct-a.test");
        assert_eq!(parsed[0].1.len(), 10);
        let window = api(
            ports.api,
            "GET",
            "/plugins/my_realiplist/show?q=DIRECT-&limit=1&offset=1",
            "",
        )
        .await;
        assert_eq!(window.header("X-Total-Count"), Some("3"));
        assert_eq!(window.body.lines().count(), 1);
        assert!(window.body.contains("direct-c.test"));
        assert_eq!(
            api(ports.api, "GET", "/plugins/my_realiplist/show?q=zzz", "")
                .await
                .header("X-Total-Count"),
            Some("0")
        );

        // The detail route.
        let detail = api(
            ports.api,
            "GET",
            "/plugins/route_learner/entries?category=my_realiplist&sort=hits&limit=2",
            "",
        )
        .await;
        assert_eq!(detail.status, 200);
        assert_eq!(detail.header("X-Total-Count"), Some("3"));
        let detail = json(&detail.body);
        assert_eq!(detail["total"], 3);
        let first = &detail["entries"][0];
        assert_eq!(first["name"], "direct-b.test");
        assert_eq!(first["category"], "my_realiplist");
        assert_eq!(first["conclusion"], "DIRECT");
        assert_eq!(first["mark"], 11);
        assert_eq!(first["reason"], "learn_direct");
        assert_eq!(first["hits"], 3);
        assert!(first["first_confirmed"].as_str().unwrap().ends_with('Z'));
        assert!(first["expires_at"].as_str().unwrap() > first["last_confirmed"].as_str().unwrap());
        assert_eq!(detail["entries"].as_array().unwrap().len(), 2);
        assert_eq!(
            api(
                ports.api,
                "GET",
                "/plugins/route_learner/entries?category=nope",
                ""
            )
            .await
            .status,
            404
        );
        let status = json(
            &api(ports.api, "GET", "/plugins/route_learner/status", "")
                .await
                .body,
        );
        let direct = status["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["tag"] == "my_realiplist")
            .unwrap();
        assert_eq!(direct["entries"], 3);
        assert_eq!(direct["ttl_seconds"], 100);
        assert_eq!(status["counters"]["learned"], 3);

        // Methods and actions.
        assert_eq!(
            api(ports.api, "POST", "/plugins/my_realiplist/show", "")
                .await
                .status,
            405
        );
        assert_eq!(
            api(ports.api, "POST", "/plugins/route_learner/entries", "")
                .await
                .status,
            405
        );
        assert_eq!(
            api(ports.api, "GET", "/plugins/my_realiplist/nosuch", "")
                .await
                .status,
            404
        );
        assert_eq!(
            api(ports.api, "GET", "/plugins/my_realiplist/restartall", "")
                .await
                .status,
            501
        );

        // `save` writes the file once; a second save with nothing new changes
        // nothing.
        let saved = api(ports.api, "GET", "/plugins/my_realiplist/save", "").await;
        assert_eq!(
            (saved.status, saved.body.as_str()),
            (200, "domain_output saved")
        );
        let file = json(&tree.text("gen/route_learner.json"));
        assert_eq!(file["format"], 1);
        let rows = file["categories"]["my_realiplist"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0], "direct-a.test");
        assert_eq!(rows[1][3], 3, "the hit count is written");
        assert_eq!(rows[1][4], "learn_direct");
        let before = tree.text("gen/route_learner.json");
        assert_eq!(
            api(ports.api, "GET", "/plugins/my_realiplist/save", "")
                .await
                .status,
            200
        );
        assert_eq!(tree.text("gen/route_learner.json"), before);

        // Expired names are neither listed nor counted.
        learner.advance_clock(101);
        assert_eq!(
            api(ports.api, "GET", "/plugins/my_realiplist/show", "")
                .await
                .header("X-Total-Count"),
            Some("0")
        );
        // `flush` clears one category only and writes at once.
        a(ports.dns, "direct-d.test").await;
        a(ports.dns, "proxy-d.test").await;
        let flushed = api(ports.api, "GET", "/plugins/my_realiplist/flush", "").await;
        assert_eq!(
            (flushed.status, flushed.body.as_str()),
            (200, "domain_output flushed")
        );
        let file = json(&tree.text("gen/route_learner.json"));
        assert_eq!(
            file["categories"]["my_realiplist"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            file["categories"]["my_fakeiplist"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            a(ports.dns, "direct-d.test").await,
            0,
            "forgotten, asked again"
        );
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn closing_the_host_writes_the_state_and_a_restart_routes_by_it_at_once() {
    let tree = Tree::new("restart");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(a(ports.dns, "direct9.test").await, 0);
        assert_eq!(aaaa(ports.dns, "v6none9.test").await, 0);
        // Nothing was written yet: the interval is an hour and nothing asked.
        assert_eq!(tree.text("gen/route_learner.json"), "");
        shutdown.cancel();
        task.await.unwrap();
    });
    let file = json(&tree.text("gen/route_learner.json"));
    assert_eq!(file["categories"]["my_realiplist"][0][0], "direct9.test");
    assert_eq!(file["categories"]["my_nov6list"][0][0], "v6none9.test");

    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let before = upstream.count("direct9.test", 1);
        assert_eq!(a(ports.dns, "direct9.test").await, 11);
        assert_eq!(aaaa(ports.dns, "v6none9.test").await, 6);
        assert_eq!(upstream.count("direct9.test", 1), before);
        shutdown.cancel();
        task.await.unwrap();
    });
    // A damaged file starts empty instead of failing the start.
    tree.write("gen/route_learner.json", "{broken");
    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(a(ports.dns, "direct9.test").await, 0);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_hot_apply_hands_the_live_state_over_and_the_switches_still_work() {
    let tree = Tree::new("handover");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        assert_eq!(a(ports.dns, "direct5.test").await, 0);
        assert_eq!(aaaa(ports.dns, "v6none5.test").await, 0);
        let page = json(
            &api(ports.api, "GET", "/api/v1/domain-generation", "")
                .await
                .body,
        );
        assert_eq!(page["remember_direct"], true);

        // Turning `no_v4` off is a settings apply: a new generation. What was
        // learned survives it in memory: the state file cannot be written or
        // read here (a directory stands in its place), so a hand-over through
        // the file would lose it. The flush that follows the apply does need
        // the file and reports it.
        std::fs::create_dir_all(tree.path("gen/route_learner.json")).unwrap();
        let reply = api(
            ports.api,
            "POST",
            "/api/v1/domain-generation",
            r#"{"no_v4":false}"#,
        )
        .await;
        assert_eq!(reply.status, 500, "{}", reply.body);
        assert!(
            reply.body.contains("settings were applied"),
            "{}",
            reply.body
        );
        std::fs::remove_dir_all(tree.path("gen/route_learner.json")).unwrap();
        assert_eq!(a(ports.dns, "direct5.test").await, 11);
        assert_eq!(aaaa(ports.dns, "v6none5.test").await, 6);
        let status = json(
            &api(ports.api, "GET", "/plugins/route_learner/status", "")
                .await
                .body,
        );
        assert_eq!(status["counters"]["learned"], 2);

        // `remember_direct` off: the category is flushed and stops learning
        // and answering; the others are untouched.
        let reply = json(
            &api(
                ports.api,
                "POST",
                "/api/v1/domain-generation",
                r#"{"remember_direct":false}"#,
            )
            .await
            .body,
        );
        assert_eq!(reply["flush_targets"], serde_json::json!(["my_realiplist"]));
        assert_eq!(a(ports.dns, "direct5.test").await, 0);
        assert_eq!(
            a(ports.dns, "direct5.test").await,
            0,
            "not learned while off"
        );
        assert_eq!(aaaa(ports.dns, "v6none5.test").await, 6);
        let effective = json(
            &api(ports.api, "GET", "/api/v1/settings/effective", "")
                .await
                .body,
        );
        assert!(
            effective.to_string().contains("my_realiplist"),
            "{effective}"
        );
        api(
            ports.api,
            "POST",
            "/api/v1/domain-generation",
            r#"{"remember_direct":true}"#,
        )
        .await;
        assert_eq!(a(ports.dns, "direct6.test").await, 0);
        assert_eq!(a(ports.dns, "direct6.test").await, 11);
        // The master switch gates every category.
        let master = json(
            &api(
                ports.api,
                "POST",
                "/api/v1/domain-generation",
                r#"{"enabled":false}"#,
            )
            .await
            .body,
        );
        assert_eq!(master["flush_targets"].as_array().unwrap().len(), 5);
        assert_eq!(aaaa(ports.dns, "v6none5.test").await, 0);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn bad_configurations_are_refused_with_the_path() {
    for (what, patch, expected) in [
        (
            "unknown mapper",
            ("target_mapper: unified", "target_mapper: nosuch"),
            "target_mapper",
        ),
        ("ttl too small", ("ttl: 1000}", "ttl: 5}"), "ttl"),
        ("mark too big", ("mark: 4,", "mark: 64,"), "mark"),
        (
            "duplicate category",
            ("tag: my_nov4list", "tag: my_nov6list"),
            "duplicate category tag",
        ),
        (
            "category clashes with a plugin",
            ("tag: my_nov4list", "tag: wl"),
            "already a plugin",
        ),
        (
            "unknown category in a rule",
            ("learn: [my_realiplist]", "learn: [nosuch]"),
            "not a category",
        ),
        (
            "unknown learner in a rule",
            (
                "learner: route_learner, learn: [my_realiplist]",
                "learner: nosuch, learn: [my_realiplist]",
            ),
            "not a route_learner",
        ),
        (
            "bad condition",
            (
                "\"qtype 1\", \"resp_ip 192.0.2.0/24\"",
                "\"nosuchmatcher 1\"",
            ),
            "nosuchmatcher",
        ),
        (
            "unknown argument",
            (
                "write_interval: 3600",
                "write_interval: 3600\n      bogus: 1",
            ),
            "bogus",
        ),
        (
            "pipe in a label",
            ("label: \"SPECIAL\"", "label: \"A|B\""),
            "label",
        ),
    ] {
        let tree = Tree::new("bad");
        let ports = ports();
        let upstream = Upstream::start();
        config(&tree, &ports, &upstream, "", "");
        let yaml = tree.text("config.yaml");
        assert!(
            yaml.contains(patch.0),
            "{what}: fixture lacks {:?}",
            patch.0
        );
        tree.write("config.yaml", &yaml.replacen(patch.0, patch.1, 1));
        let error = mosdns_native_host::load_and_compile(&tree.path("config.yaml"))
            .err()
            .unwrap_or_else(|| panic!("{what} must fail"))
            .to_string();
        assert!(error.contains(expected), "{what}: {error}");
    }
    // A mapper takes one learner.
    let tree = Tree::new("two");
    let ports = ports();
    let upstream = Upstream::start();
    config(
        &tree,
        &ports,
        &upstream,
        "  - tag: second\n    type: route_learner\n    args:\n      target_mapper: unified\n      categories:\n        - {tag: other, mark: 20, label: OTHER, ttl: 100}\n",
        "",
    );
    let error = mosdns_native_host::load_and_compile(&tree.path("config.yaml"))
        .err()
        .expect("second learner")
        .to_string();
    assert!(error.contains("already has a route_learner"), "{error}");
}

/// The `requery` flush rule: the caches are still flushed, the URLs that
/// flush a learned category are skipped, anything else is called.
#[test]
fn requery_flushes_the_caches_but_never_a_learned_category() {
    let tree = Tree::new("requery");
    let ports = ports();
    let upstream = Upstream::start();
    config(
        &tree,
        &ports,
        &upstream,
        "  - tag: requery\n    type: requery\n    args: {file: webinfo/requeryconfig.json}\n  - tag: top_domains\n    type: domain_output\n    args: {file_stat: gen/top.txt, dump_interval: 3600}\n",
        "",
    );
    let api_port = ports.api;
    tree.write(
        "webinfo/requeryconfig.json",
        &format!(
            r#"{{
  "domain_processing": {{"source_files": [{{"alias": "stats", "path": "gen/top.txt"}}]}},
  "url_actions": {{
    "save_rules": [],
    "flush_rules": [
      "http://127.0.0.1:{api_port}/plugins/cache/flush",
      "http://127.0.0.1:{api_port}/plugins/my_realiplist/flush",
      "http://localhost:{api_port}/plugins/my_nov6list/flush",
      "http://127.0.0.1:{api_port}/plugins/top_domains/flush"
    ]
  }},
  "scheduler": {{"enabled": false, "start_datetime": "", "interval_minutes": 0}},
  "execution_settings": {{"queries_per_second": 200, "resolver_address": "127.0.0.1:{dns}", "url_call_delay_ms": 5, "date_range_days": 30}},
  "status": {{"task_state": "idle", "progress": {{"processed": 0, "total": 0}}}}
}}"#,
            dns = ports.dns
        ),
    );
    tree.write("gen/top.txt", "0000000001 9999-12-31 plain-r.test\n");
    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        // A cached name, a learned direct name and a learned no-V6 name.
        a(ports.dns, "direct7.test").await;
        aaaa(ports.dns, "v6none7.test").await;
        a(ports.dns, "plain-r.test").await;
        let cached = upstream.count("plain-r.test", 1);
        assert_eq!(a(ports.dns, "plain-r.test").await, 0);
        assert_eq!(
            upstream.count("plain-r.test", 1),
            cached,
            "served from the cache"
        );

        let started = api(ports.api, "POST", "/plugins/requery/trigger", "").await;
        assert_eq!(started.status, 200, "{}", started.body);
        let deadline = Instant::now() + Duration::from_secs(10);
        let done = loop {
            let status = json(
                &api(ports.api, "GET", "/plugins/requery/status", "")
                    .await
                    .body,
            );
            if status["task_state"] == "idle" {
                break status;
            }
            assert!(status["task_state"] != "failed", "{status}");
            assert!(Instant::now() < deadline, "task did not finish: {status}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(done["skipped_flush_urls"], 2, "{done}");
        // The cache was flushed: the re-query reached the upstream again.
        assert!(upstream.count("plain-r.test", 1) > cached);
        // The learned names were not touched.
        assert_eq!(a(ports.dns, "direct7.test").await, 11);
        assert_eq!(aaaa(ports.dns, "v6none7.test").await, 6);
        // The ranking's flush URL was called: only the re-queried name is left.
        let top = tree.text("gen/top.txt");
        assert!(!top.contains("direct7.test"), "{top}");
        shutdown.cancel();
        task.await.unwrap();
    });
}

// ----- tunable numbers (cache and learner settings) ----------------------------------

fn settings_file(tree: &Tree) -> serde_json::Value {
    json(&tree.text("webinfo/settings.json"))
}

#[test]
fn the_cache_numbers_are_tuned_through_the_api_and_a_change_replaces_the_cache() {
    let tree = Tree::new("cache-tuning");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let listed = json(&api(ports.api, "GET", "/api/v1/cache/settings", "").await.body);
        assert_eq!(listed["settings_file"], "webinfo/settings.json");
        let caches = listed["caches"].as_array().unwrap();
        assert_eq!(caches.len(), 1);
        assert_eq!(caches[0]["tag"], "cache");
        assert_eq!(
            caches[0]["size"],
            serde_json::json!({"value": 1024, "default": 1024, "modified": false})
        );
        assert_eq!(caches[0]["lazy_cache_ttl"]["value"], 0);
        assert_eq!(caches[0]["dump_interval"]["value"], 600);

        // The second query is a cache hit.
        assert_eq!(a(ports.dns, "plain.test").await, 0);
        assert_eq!(a(ports.dns, "plain.test").await, 0);
        assert_eq!(upstream.count("plain.test", 1), 1);

        let changed = api(
            ports.api,
            "POST",
            "/api/v1/cache/settings/cache",
            r#"{"size":5000,"lazy_cache_ttl":604800}"#,
        )
        .await;
        assert_eq!(changed.status, 200, "{}", changed.body);
        let changed = json(&changed.body);
        assert_eq!(changed["reset"], serde_json::json!(["cache"]));
        assert_eq!(changed["cache"]["size"]["value"], 5000);
        assert_eq!(changed["cache"]["size"]["modified"], true);
        assert_eq!(changed["cache"]["dump_interval"]["modified"], false);
        assert_eq!(
            settings_file(&tree),
            serde_json::json!({"version": 1, "caches": {"cache": {"size": 5000, "lazy_cache_ttl": 604_800}}})
        );
        let effective = json(&api(ports.api, "GET", "/api/v1/settings/effective", "").await.body);
        assert!(
            effective["items"].as_array().unwrap().iter().any(|item| item["kind"] == "cache"
                && item["field"] == "size"
                && item["value"] == "5000"
                && item["source"] == "settings"),
            "{effective}"
        );
        // The cache is a new, empty one.
        assert_eq!(a(ports.dns, "plain.test").await, 0);
        assert_eq!(upstream.count("plain.test", 1), 2);

        // The same values again change nothing; a value equal to the YAML one
        // is not stored.
        let again = json(
            &api(
                ports.api,
                "POST",
                "/api/v1/cache/settings/cache",
                r#"{"size":5000}"#,
            )
            .await
            .body,
        );
        assert_eq!(again["reset"], serde_json::json!([]));
        assert_eq!(a(ports.dns, "plain.test").await, 0);
        assert_eq!(upstream.count("plain.test", 1), 2, "still cached");
        let back = json(
            &api(
                ports.api,
                "POST",
                "/api/v1/cache/settings/cache",
                r#"{"size":1024}"#,
            )
            .await
            .body,
        );
        assert_eq!(back["reset"], serde_json::json!(["cache"]));
        assert_eq!(
            settings_file(&tree),
            serde_json::json!({"version": 1, "caches": {"cache": {"lazy_cache_ttl": 604_800}}})
        );

        // Bad requests change nothing.
        for (method, path, body, status) in [
            ("POST", "/api/v1/cache/settings/cache", r#"{"size":10}"#, 400),
            ("POST", "/api/v1/cache/settings/cache", r#"{"size":"big"}"#, 400),
            ("POST", "/api/v1/cache/settings/cache", r#"{"size":1.5}"#, 400),
            ("POST", "/api/v1/cache/settings/cache", r#"{"dump_interval":5}"#, 400),
            ("POST", "/api/v1/cache/settings/cache", r#"{"bogus":1}"#, 400),
            ("POST", "/api/v1/cache/settings/cache", "{}", 400),
            ("POST", "/api/v1/cache/settings/cache", "not json", 400),
            ("POST", "/api/v1/cache/settings/nothing", r#"{"size":5000}"#, 404),
            ("GET", "/api/v1/cache/settings/cache", "", 405),
            ("PUT", "/api/v1/cache/settings", "", 405),
        ] {
            let reply = api(ports.api, method, path, body).await;
            assert_eq!(reply.status, status, "{method} {path} {body}: {}", reply.body);
        }
        assert_eq!(
            settings_file(&tree),
            serde_json::json!({"version": 1, "caches": {"cache": {"lazy_cache_ttl": 604_800}}})
        );

        // `null` restores one field; DELETE restores them all.
        let nulled = json(
            &api(
                ports.api,
                "POST",
                "/api/v1/cache/settings/cache",
                r#"{"lazy_cache_ttl":null}"#,
            )
            .await
            .body,
        );
        assert_eq!(nulled["cache"]["lazy_cache_ttl"]["modified"], false);
        assert_eq!(settings_file(&tree), serde_json::json!({"version": 1}));
        api(
            ports.api,
            "POST",
            "/api/v1/cache/settings/cache",
            r#"{"dump_interval":3600}"#,
        )
        .await;
        let deleted = json(&api(ports.api, "DELETE", "/api/v1/cache/settings/cache", "").await.body);
        assert_eq!(deleted["cache"]["dump_interval"]["value"], 600);
        assert_eq!(deleted["reset"], serde_json::json!(["cache"]));
        assert_eq!(settings_file(&tree), serde_json::json!({"version": 1}));
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn the_learner_numbers_are_tuned_through_the_api_and_the_learned_names_stay() {
    let tree = Tree::new("learner-tuning");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let listed = json(&api(ports.api, "GET", "/api/v1/learning/settings", "").await.body);
        assert_eq!(
            listed["write_interval"],
            serde_json::json!({"value": 3600, "default": 3600, "modified": false})
        );
        let categories = listed["categories"].as_array().unwrap();
        assert_eq!(categories.len(), 5);
        assert_eq!(categories[0]["tag"], "my_sv4list", "declaration order");
        assert_eq!(categories[0]["ttl"]["value"], 1000);
        assert_eq!(categories[1]["tag"], "my_realiplist");
        assert_eq!(categories[1]["ttl"]["default"], 100);
        assert_eq!(categories[1]["max_entries"]["value"], 20000);

        for name in ["direct1.test", "direct2.test", "direct3.test"] {
            assert_eq!(a(ports.dns, name).await, 0);
        }
        assert_eq!(a(ports.dns, "direct1.test").await, 11);

        let changed = api(
            ports.api,
            "POST",
            "/api/v1/learning/settings",
            r#"{"write_interval":600,"categories":{"my_realiplist":{"ttl":7200,"max_entries":50},"my_nov6list":{"ttl":120}}}"#,
        )
        .await;
        assert_eq!(changed.status, 200, "{}", changed.body);
        let changed = json(&changed.body);
        assert_eq!(changed["write_interval"]["value"], 600);
        assert_eq!(changed["categories"][1]["ttl"]["modified"], true);
        assert_eq!(changed["categories"][2]["ttl"]["modified"], false);
        assert_eq!(
            settings_file(&tree),
            serde_json::json!({"version": 1, "learning": {
                "write_interval": 600,
                "categories": {
                    "my_realiplist": {"ttl": 7200, "max_entries": 50},
                    "my_nov6list": {"ttl": 120}
                }
            }})
        );
        let status = json(&api(ports.api, "GET", "/plugins/route_learner/status", "").await.body);
        assert_eq!(status["write_interval_seconds"], 600);
        let direct = status["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["tag"] == "my_realiplist")
            .unwrap();
        assert_eq!(direct["ttl_seconds"], 7200);
        assert_eq!(direct["max_entries"], 50);
        assert_eq!(direct["entries"], 3, "what was learned stays");
        assert_eq!(a(ports.dns, "direct2.test").await, 11);

        // Lowering the capacity keeps that many names.
        api(
            ports.api,
            "POST",
            "/api/v1/learning/settings",
            r#"{"categories":{"my_realiplist":{"max_entries":2}}}"#,
        )
        .await;
        let status = json(&api(ports.api, "GET", "/plugins/route_learner/status", "").await.body);
        let direct = status["categories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|category| category["tag"] == "my_realiplist")
            .unwrap();
        assert_eq!(direct["entries"], 2);
        assert_eq!(status["counters"]["evicted"], 1);

        // Values equal to the YAML ones are not stored; null restores.
        let restored = api(
            ports.api,
            "POST",
            "/api/v1/learning/settings",
            r#"{"write_interval":3600,"categories":{"my_realiplist":{"ttl":100,"max_entries":null},"my_nov6list":{"ttl":null}}}"#,
        )
        .await;
        assert_eq!(restored.status, 200, "{}", restored.body);
        assert_eq!(settings_file(&tree), serde_json::json!({"version": 1}));

        // Bad requests change nothing.
        api(
            ports.api,
            "POST",
            "/api/v1/learning/settings",
            r#"{"write_interval":900}"#,
        )
        .await;
        let kept = settings_file(&tree);
        for (method, body, status) in [
            ("POST", r#"{"write_interval":10}"#, 400),
            ("POST", r#"{"categories":{"my_realiplist":{"ttl":10}}}"#, 400),
            ("POST", r#"{"categories":{"my_realiplist":{"max_entries":0}}}"#, 400),
            ("POST", r#"{"categories":{"my_realiplist":{"max_entries":262145}}}"#, 400),
            ("POST", r#"{"categories":{"my_realiplist":{"mark":3}}}"#, 400),
            ("POST", r#"{"categories":{"my_realiplist":5}}"#, 400),
            ("POST", r#"{"categories":{"nothing":{"ttl":100}}}"#, 404),
            ("POST", r#"{"bogus":1}"#, 400),
            ("POST", "{}", 400),
            ("PUT", "{}", 405),
        ] {
            let reply = api(ports.api, method, "/api/v1/learning/settings", body).await;
            assert_eq!(reply.status, status, "{method} {body}: {}", reply.body);
        }
        assert_eq!(settings_file(&tree), kept);
        let deleted = api(ports.api, "DELETE", "/api/v1/learning/settings", "").await;
        assert_eq!(deleted.status, 200);
        assert_eq!(settings_file(&tree), serde_json::json!({"version": 1}));
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn caches_and_learning_rules_carry_display_labels() {
    let tree = Tree::new("labels");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    // The cache and the rule that learns direct names get a label; the other rules do not.
    let yaml = tree
        .text("config.yaml")
        .replace(
            r#"args: {dump_file: "cache/c.gz"}"#,
            r#"args: {dump_file: "cache/c.gz", label: "全局缓存·兼容模式"}"#,
        )
        .replace(
            "{tag: learn_direct, type: route_learn, args: {learner: route_learner,",
            r#"{tag: learn_direct, type: route_learn, args: {label: "探测到国内 IP", learner: route_learner,"#,
        );
    tree.write("config.yaml", &yaml);
    let host = assemble(compile(&tree));
    host.block_on(async {
        let (shutdown, task) = serve(&host).await;
        let inventory = json(
            &api(ports.api, "GET", "/api/v1/cache/inventory", "")
                .await
                .body,
        );
        assert_eq!(
            inventory["caches"][0],
            serde_json::json!({"tag": "cache", "label": "全局缓存·兼容模式"})
        );
        let listed = json(
            &api(ports.api, "GET", "/api/v1/cache/settings", "")
                .await
                .body,
        );
        assert_eq!(listed["caches"][0]["label"], "全局缓存·兼容模式");

        assert_eq!(a(ports.dns, "direct1.test").await, 0);
        assert_eq!(a(ports.dns, "proxy1.test").await, 0);
        let entries = json(
            &api(
                ports.api,
                "GET",
                "/plugins/route_learner/entries?limit=50",
                "",
            )
            .await
            .body,
        );
        let find = |name: &str| {
            entries["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["name"] == name)
                .unwrap_or_else(|| panic!("{name}: {entries}"))
                .clone()
        };
        assert_eq!(find("direct1.test")["reason"], "learn_direct");
        assert_eq!(find("direct1.test")["reason_label"], "探测到国内 IP");
        assert_eq!(find("proxy1.test")["reason"], "learn_proxy");
        assert_eq!(find("proxy1.test")["reason_label"], "");

        // Renaming a cache never replaces it: the second query is still a hit.
        assert_eq!(a(ports.dns, "plain.test").await, 0);
        assert_eq!(a(ports.dns, "plain.test").await, 0);
        assert_eq!(upstream.count("plain.test", 1), 1);
        shutdown.cancel();
        task.await.unwrap();
    });
}

#[test]
fn a_bad_label_is_a_load_error_with_its_path() {
    let tree = Tree::new("bad-label");
    let ports = ports();
    let upstream = Upstream::start();
    config(&tree, &ports, &upstream, "", "");
    let base = tree.text("config.yaml");
    for (from, to, path) in [
        (
            r#"args: {dump_file: "cache/c.gz"}"#,
            r#"args: {dump_file: "cache/c.gz", label: ""}"#,
            "cache",
        ),
        (
            r#"args: {dump_file: "cache/c.gz"}"#,
            r#"args: {dump_file: "cache/c.gz", label: 5}"#,
            "cache",
        ),
        (
            "{tag: learn_direct, type: route_learn, args: {learner: route_learner,",
            &format!(
                "{{tag: learn_direct, type: route_learn, args: {{label: \"{}\", learner: route_learner,",
                "长".repeat(65)
            ),
            "route_learn",
        ),
    ] {
        tree.write("config.yaml", &base.replace(from, to));
        let error = mosdns_native_host::load_and_compile(&tree.path("config.yaml"))
            .err()
            .unwrap_or_else(|| panic!("{path}: a bad label must fail"))
            .to_string();
        assert!(error.contains("label"), "{path}: {error}");
    }
}
