//! End-to-end verification of `resp_ip_all`, `ip_set.seed_text`, and the
//! `geosite_cn` DNS pollution detection, cache isolation, lazy refresh
//! learning, route learning, audit `effective_tag`, and requery refresh flow.
//!
//! All fixture IP addresses use RFC 5737 documentation ranges exclusively:
//! - 192.0.2.0/24    (TEST-NET-1): synthetic poisoned answers
//! - 198.51.100.0/24 (TEST-NET-2): synthetic clean domestic answers
//! - 203.0.113.0/24  (TEST-NET-3): synthetic proxy / `FakeIP` answers
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atomdns_native_host::HostAssembly;
use atomdns_upstream_core::TransportCancellation;
use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::rdata::A;
use hickory_proto::rr::{Name, RData, Record, RecordType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const POISON_1: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);
const POISON_2: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 2);
const CLEAN_1: Ipv4Addr = Ipv4Addr::new(198, 51, 100, 1);
const PROXY_1: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 1);
const PROXY_2: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 2);
const PROXY_3: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 3);
const PROXY_9: Ipv4Addr = Ipv4Addr::new(203, 0, 113, 9);

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("poison-detect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, rel: &str, text: &str) {
        let full = self.0.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, text).unwrap();
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

fn make_query(name: &str, qtype: RecordType) -> Vec<u8> {
    let mut msg = Message::new();
    msg.set_id(0x1234)
        .set_message_type(MessageType::Query)
        .set_op_code(OpCode::Query)
        .set_recursion_desired(true);
    let mut q = hickory_proto::op::Query::new();
    q.set_name(Name::from_ascii(name).unwrap())
        .set_query_type(qtype);
    msg.add_query(q);
    msg.to_vec().unwrap()
}

fn make_a_reply(req: &[u8], ips: &[Ipv4Addr], ttl: u32) -> Vec<u8> {
    let query = Message::from_vec(req).unwrap();
    let mut reply = Message::new();
    reply
        .set_id(query.id())
        .set_message_type(MessageType::Response)
        .set_op_code(OpCode::Query)
        .set_recursion_desired(true)
        .set_recursion_available(true)
        .set_response_code(ResponseCode::NoError);
    let q = query.queries()[0].clone();
    let qname = q.name().clone();
    let qtype = q.query_type();
    reply.add_query(q);
    if qtype == RecordType::A {
        for &ip in ips {
            reply.add_answer(Record::from_rdata(qname.clone(), ttl, RData::A(A(ip))));
        }
    }
    reply.to_vec().unwrap()
}

async fn dns_query(port: u16, name: &str, qtype: RecordType) -> (ResponseCode, Vec<Ipv4Addr>) {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let packet = make_query(name, qtype);
    socket.send_to(&packet, ("127.0.0.1", port)).await.unwrap();
    let mut buf = [0u8; 2048];
    let (n, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();
    let msg = Message::from_vec(&buf[..n]).unwrap();
    let ips = msg
        .answers()
        .iter()
        .filter_map(|rec| match rec.data() {
            RData::A(a) => Some(a.0),
            _ => None,
        })
        .collect();
    (msg.response_code(), ips)
}

async fn http(port: u16, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: local\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(req.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    socket.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_owned())
}

struct MockDns {
    socket: Option<std::net::UdpSocket>,
    port: u16,
    hits: Arc<AtomicUsize>,
    ttl: Arc<AtomicU32>,
    table: Arc<Mutex<HashMap<String, Vec<Ipv4Addr>>>>,
}

impl MockDns {
    fn bind(initial: &[(&str, &[Ipv4Addr])]) -> Self {
        Self::bind_with_ttl(initial, 60)
    }

    fn bind_with_ttl(initial: &[(&str, &[Ipv4Addr])], ttl: u32) -> Self {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let port = socket.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let ttl = Arc::new(AtomicU32::new(ttl));
        let table: Arc<Mutex<HashMap<String, Vec<Ipv4Addr>>>> = Arc::new(Mutex::new(
            initial
                .iter()
                .map(|(k, v)| ((*k).to_ascii_lowercase(), v.to_vec()))
                .collect(),
        ));
        Self {
            socket: Some(socket),
            port,
            hits,
            ttl,
            table,
        }
    }

    fn spawn(&mut self) {
        let std_sock = self.socket.take().unwrap();
        let socket = tokio::net::UdpSocket::from_std(std_sock).unwrap();
        let hits_bg = Arc::clone(&self.hits);
        let ttl_bg = Arc::clone(&self.ttl);
        let table_bg = Arc::clone(&self.table);
        tokio::task::spawn_local(async move {
            let mut buf = [0u8; 2048];
            while let Ok((n, peer)) = socket.recv_from(&mut buf).await {
                hits_bg.fetch_add(1, Ordering::SeqCst);
                let req = &buf[..n];
                let msg = Message::from_vec(req).unwrap();
                let qname = msg.queries()[0]
                    .name()
                    .to_ascii()
                    .trim_end_matches('.')
                    .to_ascii_lowercase();
                let ips = table_bg
                    .lock()
                    .unwrap()
                    .get(&qname)
                    .cloned()
                    .unwrap_or_default();
                let reply = make_a_reply(req, &ips, ttl_bg.load(Ordering::SeqCst));
                let _ = socket.send_to(&reply, peer).await;
            }
        });
    }
}

#[test]
fn resp_ip_all_matches_only_non_empty_all_matching_answers() {
    let mut upstream = MockDns::bind(&[
        ("empty.test", &[]),
        ("single-bad.test", &[POISON_1]),
        ("multi-bad.test", &[POISON_1, POISON_2]),
        ("mixed.test", &[POISON_1, CLEAN_1]),
        ("clean.test", &[CLEAN_1]),
    ]);
    let dns = free_port();
    let tree = Tree::new("resp-ip-all");
    tree.write(
        "config.yaml",
        &format!(
            r"log: {{level: error}}
plugins:
  - tag: poisoned_ip
    type: ip_set
    args:
      ips:
        - '{POISON_1}'
        - '{POISON_2}'
  - tag: up
    type: forward
    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{}'}}]}}
  - tag: entry
    type: sequence
    args:
      - exec: $up
      - matches: 'resp_ip_all $poisoned_ip'
        exec: reject 5
      - matches: '!resp_ip_all $poisoned_ip'
        exec: accept
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: '127.0.0.1:{dns}'}}
",
            upstream.port
        ),
    );
    let host = HostAssembly::from_config_file(&tree.0.join("config.yaml")).unwrap();
    host.block_on(async {
        upstream.spawn();
        let bound = host.bind_host().await.unwrap();
        let stop = TransportCancellation::new();
        let shutdown = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(shutdown).await.unwrap() });

        assert_eq!(
            dns_query(dns, "empty.test", RecordType::A).await.0,
            ResponseCode::NoError,
            "empty answer must not vacuously match resp_ip_all"
        );
        assert_eq!(
            dns_query(dns, "single-bad.test", RecordType::A).await.0,
            ResponseCode::Refused,
            "single poisoned IP matches resp_ip_all"
        );
        assert_eq!(
            dns_query(dns, "multi-bad.test", RecordType::A).await.0,
            ResponseCode::Refused,
            "all poisoned IPs match resp_ip_all"
        );
        assert_eq!(
            dns_query(dns, "mixed.test", RecordType::A).await.0,
            ResponseCode::NoError,
            "mixed clean + poisoned IPs must not match resp_ip_all"
        );
        assert_eq!(
            dns_query(dns, "clean.test", RecordType::A).await.0,
            ResponseCode::NoError,
            "clean IP must not match resp_ip_all"
        );

        stop.cancel();
        serving.await.unwrap();
    });
}

#[test]
#[allow(clippy::too_many_lines)]
fn ip_set_seed_text_seeds_only_missing_file_on_normal_start() {
    // 1. Invalid seed_text fails compile even when file exists.
    let bad_tree = Tree::new("seed-bad");
    bad_tree.write("rule/poisoned_ip.txt", &format!("{POISON_1}\n"));
    bad_tree.write(
        "config.yaml",
        r"log: {level: error}
plugins:
  - tag: unused_forward
    type: forward
    args: {upstreams: [{addr: 'udp://127.0.0.1:15999'}]}
  - tag: poisoned_ip
    type: ip_set
    args:
      files: ['rule/poisoned_ip.txt']
      seed_text: |
        not-an-ip
  - tag: entry
    type: sequence
    args: [{exec: reject 0}]
  - tag: listener
    type: udp_server
    args: {entry: entry, listen: '127.0.0.1:0'}
",
    );
    assert!(HostAssembly::from_config_file(&bad_tree.0.join("config.yaml")).is_err());

    // 2. Missing file: normal start seeds it once, serves seed IPs, and allows API edits + flag save.
    let tree = Tree::new("seed-missing");
    let (api, dns) = (free_port(), free_port());
    tree.write(
        "config.yaml",
        &format!(
            r"log: {{level: error}}
api: {{http: '127.0.0.1:{api}'}}
plugins:
  - tag: unused_forward
    type: forward
    args: {{upstreams: [{{addr: 'udp://127.0.0.1:15999'}}]}}
  - tag: detect_poison
    type: feature_flag
    args:
      label: 污染识别
      description: test
      section: routing
      kind: bool
      default: false
  - tag: poisoned_ip
    type: ip_set
    args:
      files: ['rule/poisoned_ip.txt']
      seed_text: |
        # default header
        {POISON_1}
        {POISON_2}
  - tag: entry
    type: sequence
    args:
      - matches: flag detect_poison
        exec: reject 0
      - exec: reject 0
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: '127.0.0.1:{dns}'}}
"
        ),
    );
    assert!(!tree.0.join("rule/poisoned_ip.txt").exists());
    let host = HostAssembly::from_config_file(&tree.0.join("config.yaml")).unwrap();
    assert!(
        tree.0.join("rule/poisoned_ip.txt").exists(),
        "normal start should seed missing file"
    );
    let seeded_text = std::fs::read_to_string(tree.0.join("rule/poisoned_ip.txt")).unwrap();
    assert!(seeded_text.contains(&POISON_2.to_string()));

    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let stop = TransportCancellation::new();
        let shutdown = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(shutdown).await.unwrap() });

        let (st, shown) = http(api, "GET", "/plugins/poisoned_ip/show", "").await;
        assert_eq!(st, 200);
        assert!(shown.contains(&format!("{POISON_1}/32")));
        assert!(shown.contains(&format!("{POISON_2}/32")));

        // Flag hot apply must NOT 409 due to seeded file hash mismatch!
        let (st, body) = http(
            api,
            "POST",
            "/api/v1/flags/detect_poison",
            r#"{"value":true}"#,
        )
        .await;
        assert_eq!(st, 200, "hot apply after seed must succeed: {body}");

        // Replacing poisoned_ip with an empty list via API persists empty file.
        let (st, body) = http(api, "POST", "/plugins/poisoned_ip/post", r#"{"values":[]}"#).await;
        assert_eq!(st, 200, "{body}");
        stop.cancel();
        serving.await.unwrap();
    });
    drop(host);

    // 3. Restarting with an existing (even empty) file must NOT overwrite it from seed_text.
    assert_eq!(
        std::fs::read_to_string(tree.0.join("rule/poisoned_ip.txt")).unwrap(),
        ""
    );
    let host2 = HostAssembly::from_config_file(&tree.0.join("config.yaml")).unwrap();
    host2.block_on(async {
        let bound2 = host2.bind_host().await.unwrap();
        let stop2 = TransportCancellation::new();
        let shutdown2 = stop2.clone();
        let serving2 =
            tokio::task::spawn_local(async move { bound2.serve(shutdown2).await.unwrap() });
        let (st, shown2) = http(api, "GET", "/plugins/poisoned_ip/show", "").await;
        assert_eq!(st, 200);
        assert_eq!(shown2, "", "existing empty file must not be overwritten");
        stop2.cancel();
        serving2.await.unwrap();
    });
}

#[test]
#[allow(clippy::too_many_lines)]
fn geosite_cn_poison_detection_cache_learning_and_requery() {
    let mut domestic = MockDns::bind(&[
        ("bad.cn", &[POISON_1]),
        ("stillbad.cn", &[POISON_2]),
        ("mixed.cn", &[POISON_1, CLEAN_1]),
        ("white.cn", &[POISON_1]),
        ("clean.cn", &[CLEAN_1]),
    ]);
    let mut nocnfake = MockDns::bind(&[
        ("bad.cn", &[PROXY_1]),
        ("stillbad.cn", &[PROXY_2]),
        ("white.cn", &[PROXY_9]),
    ]);

    let (api, dns, req_dns) = (free_port(), free_port(), free_port());
    let tree = Tree::new("e2e");
    tree.write("rule/whitelist.txt", "full:white.cn\n");
    tree.write(
        "rule/geosite_cn.txt",
        "full:bad.cn\nfull:stillbad.cn\nfull:mixed.cn\nfull:white.cn\nfull:clean.cn\n",
    );
    tree.write("rule/poisoned_ip.txt", &format!("{POISON_1}\n{POISON_2}\n"));
    tree.write(
        "config.yaml",
        &format!(
            r"log: {{level: error}}
api: {{http: '127.0.0.1:{api}'}}
plugins:
  - tag: detect_poison
    type: feature_flag
    args:
      label: 污染识别
      description: test
      section: routing
      kind: bool
      default: false
  - tag: lazy_cache
    type: feature_flag
    args:
      label: 分组缓存
      description: test
      section: cache
      kind: bool
      default: true
  - tag: whitelist
    type: domain_set_light
    args: {{files: ['rule/whitelist.txt']}}
  - tag: geosite_cn
    type: domain_set_light
    args: {{files: ['rule/geosite_cn.txt']}}
  - tag: poisoned_ip
    type: ip_set
    args: {{files: ['rule/poisoned_ip.txt']}}
  - tag: unified_matcher1
    type: domain_mapper
    args:
      default_mark: 17
      default_tag: 未命中
      rules:
        - {{tag: whitelist, mark: 8, output_tag: 白名单}}
        - {{tag: geosite_cn, mark: 16, output_tag: 中国域名}}
  - tag: route_learner
    type: route_learner
    args:
      target_mapper: unified_matcher1
      file: gen/route_learner.json
      write_interval: 3600
      categories:
        - {{tag: my_sv4list, mark: 4, label: 特殊域名, source: my_sv4rule, ttl: 691200}}
        - {{tag: my_fakeiplist, mark: 12, label: 记忆代理, source: my_fakeiprule, ttl: 604800}}
  - tag: learn_poisoned
    type: route_learn
    args: {{label: 国内应答命中污染 IP, learner: route_learner, learn: [my_sv4list, my_fakeiplist]}}
  - tag: cache_cn
    type: cache
    args: {{size: 1024, lazy_cache_ttl: 0}}
  - tag: domestic
    type: forward
    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{}'}}]}}
  - tag: nocnfake
    type: forward
    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{}'}}]}}
  - tag: sequence_local_exit
    type: sequence
    args:
      - exec: flow_setter sequence=sequence_local_exit upstream=domestic
      - matches: flag lazy_cache
        exec: $cache_cn
      - exec: $domestic
      - exec: exit
  - tag: sequence_local_poison_check
    type: sequence
    args:
      - exec: flow_setter sequence=sequence_local_poison_check upstream=domestic
      - matches: flag lazy_cache
        exec: $cache_cn
      - exec: $domestic
      - matches:
        - qtype 1
        - 'resp_ip_all $poisoned_ip'
        exec:
          - $learn_poisoned
          - fast_mark 22
          - drop_resp
  - tag: sequence_poison_fakeip_exit
    type: sequence
    args:
      - exec: flow_setter sequence=sequence_poison_fakeip_exit upstream=nocnfake
      - exec: tag_setter 污染转代理
      - exec: $nocnfake
      - exec: exit
  - tag: sequence_fakeip_exit
    type: sequence
    args:
      - exec: flow_setter sequence=sequence_proxy_fakeip_exit upstream=nocnfake
      - exec: $nocnfake
      - exec: exit
  - tag: sequence_local_geosite_cn_exit
    type: sequence
    args:
      - exec: $sequence_local_poison_check
      - matches: fast_mark 22
        exec: $sequence_poison_fakeip_exit
      - matches:
        - qtype 1
        - 'resp_ip_all $poisoned_ip'
        exec:
          - drop_resp
          - $learn_poisoned
          - $sequence_poison_fakeip_exit
      - exec: exit
  - tag: sequence_listed
    type: sequence
    args:
      - matches: fast_mark 8
        exec: $sequence_local_exit
      - matches: fast_mark 8
        exec: exit
      - matches: [fast_mark 16, fast_mark 4, flag detect_poison]
        exec: $sequence_fakeip_exit
      - matches: [fast_mark 16, flag detect_poison]
        exec: $sequence_local_geosite_cn_exit
      - matches: [fast_mark 16, '!flag detect_poison']
        exec: $sequence_local_exit
      - matches: fast_mark 16
        exec: exit
  - tag: sequence_main
    type: sequence
    args:
      - exec: $unified_matcher1
      - exec: $sequence_listed
  - tag: sequence_requery
    type: sequence
    args:
      - exec: $unified_matcher1
      - matches: fast_mark 8
        exec: $sequence_local_exit
      - matches: fast_mark 8
        exec: exit
      - matches: [fast_mark 16, flag detect_poison]
        exec: $sequence_local_geosite_cn_exit
      - matches: fast_mark 16
        exec: exit
  - tag: udp_main
    type: udp_server
    args: {{entry: sequence_main, listen: '127.0.0.1:{dns}', enable_audit: true}}
  - tag: udp_req
    type: udp_server
    args: {{entry: sequence_requery, listen: '127.0.0.1:{req_dns}', enable_audit: false}}
",
            domestic.port, nocnfake.port
        ),
    );

    let host = HostAssembly::from_config_file(&tree.0.join("config.yaml")).unwrap();
    host.block_on(async {
        domestic.spawn();
        nocnfake.spawn();
        let bound = host.bind_host().await.unwrap();
        let stop = TransportCancellation::new();
        let shutdown = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(shutdown).await.unwrap() });

        // 1. Default detect_poison=false: bad.cn returns domestic POISON_1, nothing learned.
        let (rc, ips) = dns_query(dns, "bad.cn", RecordType::A).await;
        assert_eq!(rc, ResponseCode::NoError);
        assert_eq!(ips, vec![POISON_1]);
        let (_, sv4) = http(api, "GET", "/plugins/my_sv4list/show", "").await;
        assert_eq!(sv4, "");

        // 2. Enable detect_poison=true.
        let (st, _) = http(
            api,
            "POST",
            "/api/v1/flags/detect_poison",
            r#"{"value":true}"#,
        )
        .await;
        assert_eq!(st, 200);

        // Even BEFORE flushing cache_cn (bad.cn has stale POISON_1 cached from step 1),
        // the fallback resp_ip_all check catches it, learns it, and returns FakeIP!
        let (_, ips) = dns_query(dns, "bad.cn", RecordType::A).await;
        assert_eq!(ips, vec![PROXY_1]);

        // Flush cache_cn and test fresh miss on stillbad.cn: must NOT store anything in cache_cn!
        let (st, _) = http(api, "GET", "/plugins/cache_cn/flush", "").await;
        assert_eq!(st, 200);
        let dom_before = domestic.hits.load(Ordering::SeqCst);
        let (_, ips) = dns_query(dns, "stillbad.cn", RecordType::A).await;
        assert_eq!(ips, vec![PROXY_2]);
        assert_eq!(domestic.hits.load(Ordering::SeqCst), dom_before + 1);
        let (_, cache_cn_show) = http(api, "GET", "/plugins/cache_cn/show", "").await;
        assert!(
            !cache_cn_show.contains("stillbad.cn"),
            "poisoned domain must not be stored in cache_cn: {cache_cn_show}"
        );

        // Next A and AAAA queries for stillbad.cn must hit fast_mark 4 shortcut and NOT touch domestic.
        let dom_before = domestic.hits.load(Ordering::SeqCst);
        let (_, ips) = dns_query(dns, "stillbad.cn", RecordType::A).await;
        assert_eq!(ips, vec![PROXY_2]);
        let _ = dns_query(dns, "stillbad.cn", RecordType::AAAA).await;
        assert_eq!(
            domestic.hits.load(Ordering::SeqCst),
            dom_before,
            "learned poisoned domain must bypass domestic DNS for A and AAAA"
        );

        // Verify audit effective_tag: first hit = 污染转代理, subsequent hit = 记忆代理.
        let (st, logs_json) = http(
            api,
            "GET",
            "/api/v2/audit/logs?domain=stillbad.cn&limit=10",
            "",
        )
        .await;
        assert_eq!(st, 200, "{logs_json}");
        let logs: serde_json::Value = serde_json::from_str(&logs_json).unwrap();
        let tags: Vec<String> = logs["logs"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["effective_tag"].as_str().map(ToOwned::to_owned))
            .collect();
        assert!(
            tags.iter().any(|t| t == "污染转代理"),
            "first poison hit should audit as 污染转代理, got {tags:?} ({logs_json})"
        );
        assert!(
            tags.iter().any(|t| t == "记忆代理"),
            "subsequent hit should audit as 记忆代理, got {tags:?} ({logs_json})"
        );

        // Clean geosite_cn domain must keep its normal 中国域名 effective_tag.
        let (_, ips) = dns_query(dns, "clean.cn", RecordType::A).await;
        assert_eq!(ips, vec![CLEAN_1]);
        let (st, clean_logs_json) =
            http(api, "GET", "/api/v2/audit/logs?domain=clean.cn&limit=5", "").await;
        assert_eq!(st, 200);
        let clean_logs: serde_json::Value = serde_json::from_str(&clean_logs_json).unwrap();
        assert_eq!(
            clean_logs["logs"][0]["effective_tag"], "中国域名",
            "clean geosite_cn domain must keep 中国域名 effective_tag: {clean_logs_json}"
        );

        // Mixed IPs (one poisoned, one clean) must stay direct and NOT be learned.
        let (_, ips) = dns_query(dns, "mixed.cn", RecordType::A).await;
        assert_eq!(ips, vec![POISON_1, CLEAN_1]);
        // Whitelisted domain (also in geosite_cn) must stay direct even if POISON_1!
        let (_, ips) = dns_query(dns, "white.cn", RecordType::A).await;
        assert_eq!(ips, vec![POISON_1]);

        let (_, sv4) = http(api, "GET", "/plugins/my_sv4list/show", "").await;
        assert!(sv4.contains("bad.cn"));
        assert!(sv4.contains("stillbad.cn"));
        assert!(!sv4.contains("mixed.cn"));
        assert!(!sv4.contains("white.cn"));

        // 3. Requery refresh: bad.cn recovers to CLEAN_1, stillbad.cn stays poisoned.
        domestic
            .table
            .lock()
            .unwrap()
            .insert("bad.cn".to_owned(), vec![CLEAN_1]);
        let (st, _) = http(api, "GET", "/plugins/cache_cn/flush", "").await;
        assert_eq!(st, 200);
        let (_, before_json) = http(
            api,
            "GET",
            "/plugins/route_learner/entries?category=my_sv4list",
            "",
        )
        .await;
        let before_entries: serde_json::Value = serde_json::from_str(&before_json).unwrap();
        let find_last = |v: &serde_json::Value, name: &str| {
            v["entries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["name"] == name)
                .unwrap()["last_confirmed"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        let bad_last_before = find_last(&before_entries, "bad.cn");
        let still_last_before = find_last(&before_entries, "stillbad.cn");

        tokio::time::sleep(Duration::from_millis(1100)).await;
        let (_, ips) = dns_query(req_dns, "bad.cn", RecordType::A).await;
        assert_eq!(
            ips,
            vec![CLEAN_1],
            "requery must re-probe domestic and return clean IP when recovered"
        );
        let (_, ips) = dns_query(req_dns, "stillbad.cn", RecordType::A).await;
        assert_eq!(
            ips,
            vec![PROXY_2],
            "requery must keep proxying still-poisoned domain"
        );

        let (_, after_json) = http(
            api,
            "GET",
            "/plugins/route_learner/entries?category=my_sv4list",
            "",
        )
        .await;
        let after_entries: serde_json::Value = serde_json::from_str(&after_json).unwrap();
        assert_eq!(
            find_last(&after_entries, "bad.cn"),
            bad_last_before,
            "recovered domain must NOT refresh last_confirmed (expires naturally)"
        );
        assert!(
            find_last(&after_entries, "stillbad.cn") > still_last_before,
            "still-poisoned domain must refresh last_confirmed"
        );

        stop.cancel();
        serving.await.unwrap();
    });
}

#[test]
#[allow(clippy::too_many_lines)]
fn lazy_refresh_detects_poison_and_learns_proxy_without_full_cache() {
    // Regression for clean -> lazy -> poisoned with global cache disabled:
    // the background lazy refresh of `cache_cn` captures `sequence_local_poison_check`'s
    // successor, so `$learn_poisoned` must fire inside it while keeping `cache_cn`
    // free of poisoned/proxy answers.
    let mut domestic = MockDns::bind_with_ttl(&[("lazy.cn", &[CLEAN_1])], 1);
    let mut nocnfake = MockDns::bind(&[("lazy.cn", &[PROXY_3])]);
    let (api, dns) = (free_port(), free_port());
    let tree = Tree::new("lazy-poison");
    tree.write("rule/geosite_cn.txt", "full:lazy.cn\n");
    tree.write("rule/poisoned_ip.txt", &format!("{POISON_1}\n"));
    tree.write(
        "config.yaml",
        &format!(
            r"log: {{level: error}}
api: {{http: '127.0.0.1:{api}'}}
plugins:
  - tag: detect_poison
    type: feature_flag
    args:
      label: 污染识别
      description: test
      section: routing
      kind: bool
      default: true
  - tag: lazy_cache
    type: feature_flag
    args:
      label: 分组缓存
      description: test
      section: cache
      kind: bool
      default: true
  - tag: geosite_cn
    type: domain_set_light
    args: {{files: ['rule/geosite_cn.txt']}}
  - tag: poisoned_ip
    type: ip_set
    args: {{files: ['rule/poisoned_ip.txt']}}
  - tag: unified_matcher1
    type: domain_mapper
    args:
      default_mark: 17
      default_tag: 未命中
      rules:
        - {{tag: geosite_cn, mark: 16, output_tag: 中国域名}}
  - tag: route_learner
    type: route_learner
    args:
      target_mapper: unified_matcher1
      file: gen/route_learner.json
      write_interval: 3600
      categories:
        - {{tag: my_sv4list, mark: 4, label: 特殊域名, source: my_sv4rule, ttl: 691200}}
        - {{tag: my_fakeiplist, mark: 12, label: 记忆代理, source: my_fakeiprule, ttl: 604800}}
  - tag: learn_poisoned
    type: route_learn
    args: {{label: 国内应答命中污染 IP, learner: route_learner, learn: [my_sv4list, my_fakeiplist]}}
  - tag: cache_cn
    type: cache
    args: {{size: 1024, lazy_cache_ttl: 60}}
  - tag: domestic
    type: forward
    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{}'}}]}}
  - tag: nocnfake
    type: forward
    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{}'}}]}}
  - tag: sequence_local_poison_check
    type: sequence
    args:
      - exec: flow_setter sequence=sequence_local_poison_check upstream=domestic
      - matches: flag lazy_cache
        exec: $cache_cn
      - exec: $domestic
      - matches:
        - qtype 1
        - 'resp_ip_all $poisoned_ip'
        exec:
          - $learn_poisoned
          - fast_mark 22
          - drop_resp
  - tag: sequence_poison_fakeip_exit
    type: sequence
    args:
      - exec: flow_setter sequence=sequence_poison_fakeip_exit upstream=nocnfake
      - exec: tag_setter 污染转代理
      - exec: $nocnfake
      - exec: exit
  - tag: sequence_fakeip_exit
    type: sequence
    args:
      - exec: flow_setter sequence=sequence_proxy_fakeip_exit upstream=nocnfake
      - exec: $nocnfake
      - exec: exit
  - tag: sequence_local_geosite_cn_exit
    type: sequence
    args:
      - exec: $sequence_local_poison_check
      - matches: fast_mark 22
        exec: $sequence_poison_fakeip_exit
      - matches:
        - qtype 1
        - 'resp_ip_all $poisoned_ip'
        exec:
          - drop_resp
          - $learn_poisoned
          - $sequence_poison_fakeip_exit
      - exec: exit
  - tag: sequence_main
    type: sequence
    args:
      - exec: $unified_matcher1
      - matches: [fast_mark 16, fast_mark 4, flag detect_poison]
        exec: $sequence_fakeip_exit
      - matches: [fast_mark 16, flag detect_poison]
        exec: $sequence_local_geosite_cn_exit
      - matches: fast_mark 16
        exec: exit
  - tag: udp_main
    type: udp_server
    args: {{entry: sequence_main, listen: '127.0.0.1:{dns}'}}
",
            domestic.port, nocnfake.port
        ),
    );

    let host = HostAssembly::from_config_file(&tree.0.join("config.yaml")).unwrap();
    host.block_on(async {
        domestic.spawn();
        nocnfake.spawn();
        let bound = host.bind_host().await.unwrap();
        let stop = TransportCancellation::new();
        let shutdown = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(shutdown).await.unwrap() });

        // 1. Prime cache_cn with clean 1s-TTL domestic answer.
        let (_, ips) = dns_query(dns, "lazy.cn", RecordType::A).await;
        assert_eq!(ips, vec![CLEAN_1]);

        // 2. Domestic starts returning poisoned IP; wait past the 1s DNS TTL into lazy window.
        domestic
            .table
            .lock()
            .unwrap()
            .insert("lazy.cn".to_owned(), vec![POISON_1]);
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // 3. Lazy hit returns stale CLEAN_1 and fires background lazy refresh.
        let (_, ips) = dns_query(dns, "lazy.cn", RecordType::A).await;
        assert_eq!(ips, vec![CLEAN_1]);

        // 4. Wait for background lazy refresh to detect POISON_1 and learn both categories.
        let mut learned = false;
        for _ in 0..40 {
            let (_, sv4) = http(api, "GET", "/plugins/my_sv4list/show", "").await;
            let (_, fake) = http(api, "GET", "/plugins/my_fakeiplist/show", "").await;
            if sv4.contains("lazy.cn") && fake.contains("lazy.cn") {
                learned = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            learned,
            "background lazy refresh must learn lazy.cn into my_sv4list and my_fakeiplist"
        );

        // 5. cache_cn must NOT have been overwritten with POISON_1 or PROXY_3.
        let (_, cache_cn_show) = http(api, "GET", "/plugins/cache_cn/show", "").await;
        assert!(
            !cache_cn_show.contains(&POISON_1.to_string())
                && !cache_cn_show.contains(&PROXY_3.to_string()),
            "cache_cn must not hold poisoned or proxy IP after lazy refresh: {cache_cn_show}"
        );

        // 6. Next query immediately takes the learned proxy route instead of stale CLEAN_1.
        let (_, ips) = dns_query(dns, "lazy.cn", RecordType::A).await;
        assert_eq!(
            ips,
            vec![PROXY_3],
            "subsequent query after lazy refresh must route to proxy"
        );

        stop.cancel();
        serving.await.unwrap();
    });
}
