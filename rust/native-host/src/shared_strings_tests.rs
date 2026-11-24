use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use atomdns_dns_core::parse_query;
use atomdns_sequence_core::SharedStr;
use atomdns_upstream_core::TransportCancellation;
use serde_json::Value;

use crate::assembly::{HostAssembly, HostOptions};
use crate::config::compile_yaml;
use crate::execution::{ExecutionRequest, execute_request};
use crate::observer::{CacheStatus, QueryTerminalOutcome, QueryTransport, TerminalObservation};

fn free_tcp_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("probe TCP port");
    let port = listener.local_addr().expect("probe TCP address").port();
    drop(listener);
    port
}

fn free_udp_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("probe UDP port");
    let port = socket.local_addr().expect("probe UDP address").port();
    drop(socket);
    port
}

fn dns_query(id: u16, name: &str) -> Vec<u8> {
    let mut packet = vec![
        u8::try_from(id >> 8).expect("high id"),
        u8::try_from(id & 0xff).expect("low id"),
        0x01,
        0x00,
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
    ];
    for label in name.trim_end_matches('.').split('.') {
        packet.push(u8::try_from(label.len()).expect("label len"));
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
    packet
}

fn answer_a_record(query: &[u8], ip: [u8; 4], ttl: u32) -> Vec<u8> {
    let mut response = query.to_vec();
    response[2] = 0x81;
    response[3] = 0x80;
    response[6] = 0x00;
    response[7] = 0x01;
    response.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    response.extend_from_slice(&ttl.to_be_bytes());
    response.extend_from_slice(&[0x00, 0x04]);
    response.extend_from_slice(&ip);
    response
}

fn spawn_mock_upstream(ip: [u8; 4]) -> (SocketAddr, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind mock upstream");
    let addr = socket.local_addr().expect("mock upstream addr");
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .expect("read timeout");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        let mut buf = [0_u8; 4096];
        while !stop_flag.load(Ordering::Relaxed) {
            if let Ok((len, peer)) = socket.recv_from(&mut buf) {
                let reply = answer_a_record(&buf[..len], ip, 60);
                let _ = socket.send_to(&reply, peer);
            }
        }
    });
    (addr, stop, handle)
}

fn send_udp(target: SocketAddr, query: &[u8]) -> Vec<u8> {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("client udp");
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("client timeout");
    socket.send_to(query, target).expect("client send");
    let mut buf = vec![0_u8; 4096];
    let (len, _) = socket.recv_from(&mut buf).expect("client recv");
    buf.truncate(len);
    buf
}

fn http_get_json(addr: SocketAddr, path: &str) -> Value {
    let mut stream = TcpStream::connect(addr).expect("connect api");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("api timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    let (_, body) = response.split_once("\r\n\r\n").expect("http headers/body");
    serde_json::from_str(body).expect("json body")
}

#[test]
fn shared_audit_strings_survive_miss_and_repeated_cache_hits_and_match_http_json() {
    let (upstream_addr, stop_upstream, upstream_thread) = spawn_mock_upstream([192, 0, 2, 42]);
    let dns_port = free_udp_port();
    let api_port = free_tcp_port();
    let yaml = format!(
        r#"
log: {{ level: error }}
api: {{ http: "127.0.0.1:{api_port}" }}
plugins:
  - tag: cn_rules
    type: domain_set_light
    args:
      exps:
        - "domain:cn.example"
  - tag: mapper
    type: domain_mapper
    args:
      default_tag: "未命中"
      rules:
        - tag: cn_rules
          mark: 1
          output_tag: "中国域名"
  - tag: dns_cache
    type: cache
    args:
      size: 64
  - tag: domestic
    type: forward
    args:
      upstreams:
        - addr: "udp://{upstream_addr}"
  - tag: main_entry
    type: sequence
    args:
      - exec: "$mapper"
      - exec: "$dns_cache"
      - exec: "$domestic"
  - tag: udp_in
    type: udp_server
    args:
      entry: main_entry
      listen: "127.0.0.1:{dns_port}"
      enable_audit: true
"#
    );
    let config = compile_yaml(&yaml).expect("compile yaml");
    let expected_seq_name = config
        .program
        .sequence(config.sequence.sequence)
        .expect("entry sequence")
        .name
        .clone();
    let expected_upstream_id = SharedStr::from(Arc::clone(
        &config.forward_definitions[0].entries[0].response_identity,
    ));
    let expected_peer = config.peer_text(upstream_addr);

    let assembly =
        HostAssembly::with_options(config, HostOptions::default()).expect("host assembly");
    let shutdown = TransportCancellation::new();

    let (records, logs_json) = assembly.block_on(async {
        let bound = assembly.bind_host().await.expect("bind host");
        let dns_addr = bound.dns_addr();
        let api_addr = bound.api_addr().expect("api addr");
        let run_shutdown = shutdown.clone();
        let server_task = tokio::task::spawn_local(async move {
            let _ = bound.serve(run_shutdown).await;
        });

        let client_work = tokio::task::spawn_blocking(move || {
            for (id, name) in [
                (0x1001, "a.cn.example"),
                (0x1002, "a.cn.example"),
                (0x1003, "a.cn.example"),
                (0x1004, "other.example"),
                (0x1005, "other.example"),
            ] {
                let reply = send_udp(dns_addr, &dns_query(id, name));
                assert!(reply.len() > 12, "DNS reply for {name}");
            }
            http_get_json(api_addr, "/api/v2/audit/logs?limit=10")
        });

        let json = client_work.await.expect("client work");
        let records = assembly.audit_snapshot().records;
        shutdown.cancel();
        let _ = server_task.await;
        (records, json)
    });

    stop_upstream.store(true, Ordering::Relaxed);
    let _ = upstream_thread.join();

    assert_eq!(records.len(), 5);
    assert_eq!(records[0].cache_status, CacheStatus::Miss);
    assert_eq!(records[1].cache_status, CacheStatus::Hit);
    assert_eq!(records[2].cache_status, CacheStatus::Hit);
    assert_eq!(records[3].cache_status, CacheStatus::Miss);
    assert_eq!(records[4].cache_status, CacheStatus::Hit);

    // Every query shares the exact configuration-owned / static SharedStr allocations.
    for (idx, record) in records.iter().enumerate() {
        let seq = record.final_sequence.as_ref().expect("final_sequence");
        assert!(
            seq.ptr_eq(&expected_seq_name),
            "record[{idx}] final_sequence"
        );
        let upstream = record.final_upstream.as_ref().expect("final_upstream");
        assert!(
            upstream.ptr_eq(&expected_upstream_id),
            "record[{idx}] final_upstream"
        );
        let selected = record
            .selected_upstream
            .as_ref()
            .expect("selected_upstream");
        assert!(
            selected.ptr_eq(&expected_peer),
            "record[{idx}] selected_upstream"
        );
        let targets = record.upstream_targets.as_ref().expect("upstream_targets");
        assert!(
            targets.ptr_eq(&expected_peer),
            "record[{idx}] upstream_targets"
        );
        let diag_selected = record
            .upstream_diagnostics
            .as_ref()
            .and_then(|d| d.selected.as_ref())
            .expect("upstream_diagnostics.selected");
        assert!(
            diag_selected.entry.ptr_eq(&expected_upstream_id),
            "record[{idx}] diagnostics.selected.entry"
        );
    }

    // Matched domain ("中国域名") shares domain_set and effective_tag across miss + 2 hits.
    let cn_domain_set = records[0].domain_set.as_ref().expect("cn domain_set");
    let cn_effective_tag = records[0].effective_tag.as_ref().expect("cn effective_tag");
    assert_eq!(cn_domain_set.as_str(), "中国域名");
    assert!(cn_domain_set.ptr_eq(cn_effective_tag));
    for idx in [1, 2] {
        assert!(
            records[idx]
                .domain_set
                .as_ref()
                .expect("domain_set")
                .ptr_eq(cn_domain_set),
            "record[{idx}] domain_set shares allocation with miss"
        );
        assert!(
            records[idx]
                .effective_tag
                .as_ref()
                .expect("effective_tag")
                .ptr_eq(cn_effective_tag),
            "record[{idx}] effective_tag shares allocation with miss"
        );
    }

    // Unmatched domain ("未命中") shares default_tag across miss + hit and reuses it as effective_tag.
    let miss_domain_set = records[3].domain_set.as_ref().expect("miss domain_set");
    let miss_effective_tag = records[3]
        .effective_tag
        .as_ref()
        .expect("miss effective_tag");
    assert_eq!(miss_domain_set.as_str(), "未命中");
    assert!(miss_domain_set.ptr_eq(miss_effective_tag));
    assert!(
        records[4]
            .domain_set
            .as_ref()
            .expect("hit miss domain_set")
            .ptr_eq(miss_domain_set)
    );
    assert!(
        records[4]
            .effective_tag
            .as_ref()
            .expect("hit miss effective_tag")
            .ptr_eq(miss_effective_tag)
    );

    // HTTP `/api/v2/audit/logs` projects the exact expected JSON strings.
    let items = logs_json["logs"].as_array().expect("logs array");
    assert_eq!(items.len(), 5);
    let cn_row = items
        .iter()
        .find(|row| row["query_name"] == "a.cn.example")
        .expect("a.cn.example row");
    assert_eq!(cn_row["final_sequence"], "main_entry");
    assert_eq!(cn_row["domain_set"], "中国域名");
    assert_eq!(cn_row["effective_tag"], "中国域名");
    assert_eq!(cn_row["final_upstream"], "domestic");
    assert_eq!(cn_row["selected_upstream"], upstream_addr.to_string());
    assert_eq!(cn_row["upstream_targets"], upstream_addr.to_string());
    assert_eq!(
        cn_row["upstream_diagnostics"]["selected"]["entry"],
        "domestic"
    );

    let miss_row = items
        .iter()
        .find(|row| row["query_name"] == "other.example")
        .expect("other.example row");
    assert_eq!(miss_row["final_sequence"], "main_entry");
    assert_eq!(miss_row["domain_set"], "未命中");
    assert_eq!(miss_row["effective_tag"], "未命中");
    assert_eq!(miss_row["final_upstream"], "domestic");
}

#[test]
fn hot_reload_isolates_generations_and_releases_old_shared_strings_after_snapshot_drop() {
    let (upstream_addr, stop_upstream, upstream_thread) = spawn_mock_upstream([198, 51, 100, 7]);
    let yaml = format!(
        r#"
log: {{ level: error }}
plugins:
  - tag: gen_custom_set
    type: domain_set
    args:
      exps:
        - "domain:gen.example"
  - tag: domestic
    type: forward
    args:
      upstreams:
        - addr: "udp://{upstream_addr}"
  - tag: main_entry
    type: sequence
    args:
      - matches: qname $gen_custom_set
        exec: "$domestic"
  - tag: udp_in
    type: udp_server
    args:
      entry: main_entry
      listen: "127.0.0.1:15353"
      enable_audit: true
"#
    );

    let gen1 = compile_yaml(&yaml).expect("gen1");
    let gen2 = compile_yaml(&yaml).expect("gen2");

    // Two generations never share pointers even when names are identical.
    let gen1_seq = gen1
        .program
        .sequence(gen1.sequence.sequence)
        .expect("gen1 seq")
        .name
        .clone();
    let gen2_seq = gen2
        .program
        .sequence(gen2.sequence.sequence)
        .expect("gen2 seq")
        .name
        .clone();
    assert_eq!(gen1_seq, gen2_seq);
    assert!(!gen1_seq.ptr_eq(&gen2_seq));

    let gen1_upstream = SharedStr::from(Arc::clone(
        &gen1.forward_definitions[0].entries[0].response_identity,
    ));
    let gen2_upstream = SharedStr::from(Arc::clone(
        &gen2.forward_definitions[0].entries[0].response_identity,
    ));
    assert_eq!(gen1_upstream, gen2_upstream);
    assert!(!gen1_upstream.ptr_eq(&gen2_upstream));

    let weak_seq = gen1_seq.downgrade();
    let weak_upstream = gen1_upstream.downgrade();
    let weak_peer = gen1.peer_text(upstream_addr).downgrade();
    drop(gen1_seq);
    drop(gen1_upstream);

    let assembly1 = HostAssembly::from_config(gen1).expect("assembly1");
    let observer = assembly1.observer_handle();
    let raw = dns_query(0x2001, "sub.gen.example");
    let (header, question) = parse_query(&raw).expect("parse query");

    let (weak_domain_set, weak_rule_source) = assembly1.block_on(async {
        let forwards = assembly1.forwards_handle();
        let caches = assembly1.cache_handle();
        let cancellation = TransportCancellation::new();
        let mut admitted = observer.admit(
            "192.0.2.10:53000".parse().expect("client"),
            QueryTransport::Udp,
            &question,
            cancellation.clone(),
        );
        let result = execute_request(
            ExecutionRequest {
                client: atomdns_sequence_core::ClientContext::default(),
                config: assembly1.config(),
                cache: &caches,
                options: &HostOptions::default(),
                raw: &raw,
                header,
                question: question.clone(),
            },
            &forwards,
            cancellation,
            admitted.execution_checkpoint(),
        )
        .await;
        let weak_domain_set = result.domain_set.as_ref().expect("domain_set").downgrade();
        let weak_rule_source = result
            .matched_rule_source
            .as_ref()
            .expect("matched_rule_source")
            .downgrade();
        admitted.capture_execution(TerminalObservation {
            outcome: QueryTerminalOutcome::SendSucceeded,
            response: result.response,
            response_details: result.response_details,
            cache_status: result.cache_status,
            final_sequence: result.final_sequence,
            matched_group: result.matched_group,
            domain_set: result.domain_set,
            effective_tag: result.effective_tag,
            matched_rule_source: result.matched_rule_source,
            final_upstream: result.final_upstream,
            upstream_targets: result.upstream_targets,
            selected_upstream: result.selected_upstream,
            upstream_attempts: result.upstream_attempts,
            upstream_diagnostics: result.upstream_diagnostics,
            failure_provenance: result.failure_provenance,
            elapsed: Instant::now().elapsed(),
        });
        admitted.finish(QueryTerminalOutcome::SendSucceeded);
        (weak_domain_set, weak_rule_source)
    });

    stop_upstream.store(true, Ordering::Relaxed);
    let _ = upstream_thread.join();

    let snapshot = observer.audit_read_snapshot().expect("read snapshot");
    assert_eq!(snapshot.records.len(), 1);

    // Drop the old generation's HostAssembly/CompiledConfig; audit ring + snapshot keep strings alive.
    drop(assembly1);
    assert!(weak_seq.upgrade().is_some());
    assert!(weak_upstream.upgrade().is_some());
    assert!(weak_peer.upgrade().is_some());
    assert!(weak_domain_set.upgrade().is_some());
    assert!(weak_rule_source.upgrade().is_some());

    // Clearing the audit ring still leaves strings alive until the read snapshot drops.
    observer.clear_audit();
    assert!(weak_seq.upgrade().is_some());
    assert!(weak_upstream.upgrade().is_some());
    assert!(weak_peer.upgrade().is_some());
    assert!(weak_domain_set.upgrade().is_some());
    assert!(weak_rule_source.upgrade().is_some());

    // Dropping the last read snapshot releases every old-generation SharedStr immediately.
    drop(snapshot);
    assert!(weak_seq.upgrade().is_none());
    assert!(weak_upstream.upgrade().is_none());
    assert!(weak_peer.upgrade().is_none());
    assert!(weak_domain_set.upgrade().is_none());
    assert!(weak_rule_source.upgrade().is_none());

    // Keep gen2 alive through the assertion to prove it is independent.
    assert_eq!(gen2.sequence.tag, "main_entry");
}
