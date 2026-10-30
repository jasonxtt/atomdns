//! File-backed IP management over real HTTP and DNS; no external peers.
use std::path::PathBuf;
use std::time::Duration;

use atomdns_native_host::HostAssembly;
use atomdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Tree(PathBuf);
impl Tree {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("ip-management-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join(path), text).unwrap();
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn configuration(api: u16, dns: u16, args: &str, extra: &str) -> String {
    format!(
        r"log: {{level: error}}
api: {{http: '127.0.0.1:{api}'}}
plugins:
  - tag: unused_forward
    type: forward
    args: {{upstreams: [{{addr: 'udp://127.0.0.1:15999'}}]}}
  - tag: networks
    type: ip_set
    args: {args}
{extra}
  - tag: hosts
    type: hosts
    args: {{entries: ['example.com 192.0.2.7']}}
  - tag: entry
    type: sequence
    args:
      - matches: client_ip $networks
        exec: reject 3
      - exec: $hosts
      - matches: resp_ip $networks
        exec: reject 5
  - tag: listener
    type: udp_server
    args: {{entry: entry, listen: '127.0.0.1:{dns}'}}
"
    )
}

#[test]
fn ambiguous_sources_are_read_only_with_specific_reasons() {
    let shapes = [
        ("{ips: ['192.0.2.7']}", "", "inline"),
        ("{files: [ips.txt, other.txt]}", "", "2 files"),
        ("{}", "", "0 files"),
        ("{files: [ips.data]}", "", "not a .txt"),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: ip_set\n    args: {files: ['./ips.txt']}\n",
            "shared",
        ),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: domain_set\n    args: {files: [ips.txt]}\n",
            "shared",
        ),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: hosts\n    args: {files: [ips.txt]}\n",
            "shared",
        ),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: redirect\n    args: {files: [ips.txt]}\n",
            "shared",
        ),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: domain_set_light\n    args: {files: [ips.txt]}\n",
            "shared",
        ),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: sequence\n    args: [{matches: 'resp_ip &ips.txt', exec: accept}]\n",
            "shared",
        ),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: si_set\n    args: {local_config: sources.json}\n",
            "shared",
        ),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: ip_set\n    args: {files: [alias.txt]}\n",
            "shared",
        ),
        (
            "{files: [ips.txt]}",
            "  - tag: other\n    type: ip_set\n    args: {files: [hardlink.txt]}\n",
            "shared",
        ),
    ];
    for (index, (args, extra, reason)) in shapes.iter().enumerate() {
        let tree = Tree::new(&format!("shape-{index}"));
        for file in ["ips.txt", "other.txt", "ips.data"] {
            tree.write(file, "# initial\n");
        }
        std::os::unix::fs::symlink("ips.txt", tree.0.join("alias.txt")).unwrap();
        std::fs::hard_link(tree.0.join("ips.txt"), tree.0.join("hardlink.txt")).unwrap();
        tree.write(
            "sources.json",
            r#"[{"name":"reserved","files":"ips.txt","enabled":false}]"#,
        );
        let (api, dns) = (port(), port());
        tree.write("config.yaml", &configuration(api, dns, args, extra));
        let host = HostAssembly::from_config_file_in(&tree.0.join("config.yaml"), None).unwrap();
        host.block_on(async {
            let bound = host.bind_host().await.unwrap();
            let stop = TransportCancellation::new();
            let shutdown = stop.clone();
            let serving =
                tokio::task::spawn_local(async move { bound.serve(shutdown).await.unwrap() });
            let before = http(api, "GET", "networks/show", "").await;
            assert_eq!(before.0, 200, "case {index}");
            let post = http(api, "POST", "networks/post", r#"{"values":["192.0.2.7"]}"#).await;
            assert_eq!(post.0, 400, "case {index}: {}", post.2);
            assert!(post.2.contains(reason), "case {index}: {}", post.2);
            assert_eq!(http(api, "GET", "networks/show", "").await.2, before.2);
            for file in ["ips.txt", "other.txt", "ips.data"] {
                assert_eq!(
                    std::fs::read_to_string(tree.0.join(file)).unwrap(),
                    "# initial\n"
                );
            }
            stop.cancel();
            serving.await.unwrap();
        });
    }
}

#[test]
fn post_survives_an_actual_process_restart() {
    struct Process(std::process::Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let tree = Tree::new("process");
    let (api, dns) = (port(), port());
    tree.write("ips.txt", "198.51.100.9\n");
    tree.write(
        "config.yaml",
        &configuration(api, dns, "{files: [ips.txt]}", ""),
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for restart in 0..2 {
            let mut process = Process(
                std::process::Command::new(env!("CARGO_BIN_EXE_atomdns"))
                    .args(["start", "-d"])
                    .arg(&tree.0)
                    .args(["-c", "config.yaml"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    assert!(
                        process.0.try_wait().unwrap().is_none(),
                        "child exited before ready"
                    );
                    if tokio::net::TcpStream::connect(("127.0.0.1", api))
                        .await
                        .is_ok()
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            if restart == 0 {
                assert_eq!(
                    http(api, "POST", "networks/post", r#"{"values":["192.0.2.7"]}"#)
                        .await
                        .0,
                    200
                );
            }
            assert_eq!(
                http(api, "GET", "networks/show", "").await.2,
                "192.0.2.7/32\n"
            );
            assert_eq!(rcode(dns).await, 5);
            drop(process); // abrupt stop: persistence must already be complete
        }
    });
}

#[test]
fn a_rule_source_added_after_startup_makes_ip_post_read_only() {
    let tree = Tree::new("live-owner");
    let (api, dns) = (port(), port());
    tree.write("ips.txt", "192.0.2.7\n");
    tree.write("sources.json", "[]");
    tree.write(
        "config.yaml",
        &configuration(
            api,
            dns,
            "{files: [ips.txt]}",
            "  - tag: other\n    type: si_set\n    args: {local_config: sources.json}\n",
        ),
    );
    let host = HostAssembly::from_config_file_in(&tree.0.join("config.yaml"), None).unwrap();
    host.block_on(async {
        let bound = host.bind_host().await.unwrap();
        let stop = TransportCancellation::new();
        let shutdown = stop.clone();
        let serving = tokio::task::spawn_local(async move { bound.serve(shutdown).await.unwrap() });
        let added = http(
            api,
            "PUT",
            "other/config/reserved",
            r#"{"files":"ips.txt","enabled":false}"#,
        )
        .await;
        assert_eq!(added.0, 201, "{}", added.2);
        let refused = http(
            api,
            "POST",
            "networks/post",
            r#"{"values":["198.51.100.7"]}"#,
        )
        .await;
        assert_eq!(refused.0, 400, "{}", refused.2);
        assert!(refused.2.contains("shared"));
        assert_eq!(
            http(api, "GET", "networks/show", "").await.2,
            "192.0.2.7/32\n"
        );
        assert_eq!(
            std::fs::read_to_string(tree.0.join("ips.txt")).unwrap(),
            "192.0.2.7\n"
        );
        stop.cancel();
        serving.await.unwrap();
    });
}
async fn http(port: u16, method: &str, action: &str, body: &str) -> (u16, String, String) {
    let target = if action.starts_with('/') {
        action.to_owned()
    } else {
        format!("/plugins/{action}")
    };
    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    socket.write_all(format!("{method} {target} HTTP/1.1\r\nHost: local\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    socket.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        head.to_owned(),
        body.to_owned(),
    )
}
async fn rcode(port: u16) -> u8 {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut packet = vec![0, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    packet.extend_from_slice(b"\x07example\x03com\0\0\x01\0\x01");
    socket.send_to(&packet, ("127.0.0.1", port)).await.unwrap();
    let mut reply = [0; 1500];
    tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut reply))
        .await
        .unwrap()
        .unwrap();
    reply[3] & 15
}
#[test]
fn show_post_changes_both_query_matchers_and_retains_the_file() {
    let tree = Tree::new("wire");
    let (api, dns) = (port(), port());
    tree.write("ips.txt", "198.51.100.9\n");
    tree.write(
        "config.yaml",
        &configuration(api, dns, "{files: [ips.txt]}", ""),
    );
    for restart in 0..2 {
        let host = HostAssembly::from_config_file_in(&tree.0.join("config.yaml"), None).unwrap();
        host.block_on(async {
            let bound = host.bind_host().await.unwrap();
            let stop = TransportCancellation::new();
            let shutdown = stop.clone();
            let serving =
                tokio::task::spawn_local(async move { bound.serve(shutdown).await.unwrap() });
            let show = http(api, "GET", "networks/show", "").await;
            assert_eq!(show.0, 200);
            let capabilities = http(api, "GET", "/api/v1/capabilities", "").await;
            let capabilities: serde_json::Value = serde_json::from_str(&capabilities.2).unwrap();
            assert_eq!(capabilities["ui_operations"]["rules.local.manage"]["supported"], true);
            assert!(
                show.1
                    .to_ascii_lowercase()
                    .contains("content-type: text/plain; charset=utf-8")
            );
            if restart == 0 {
                assert_eq!(show.2, "198.51.100.9/32\n");
                assert_eq!(rcode(dns).await, 0);
                assert_eq!(http(api, "POST", "networks/post", "{").await.0, 400);
                assert_eq!(http(api, "GET", "missing/show", "").await.0, 404);
                assert_eq!(http(api, "GET", "hosts/show", "").await.0, 404);
                let post = http(
                    api,
                    "POST",
                    "networks/post",
                    r#"{"values":["127.0.0.1", "bad", "2001:db8::1/32", "::ffff:127.0.0.1", "2001:db8:1::/48"]}"#,
                )
                .await;
                assert_eq!(post.0, 200, "{}", post.2);
                assert_eq!(post.2, "ip_set replaced with 2 entries");
                assert_eq!(
                    http(api, "GET", "networks/show", "").await.2,
                    "127.0.0.1/32\n2001:db8::/32\n"
                );
                assert_eq!(rcode(dns).await, 3, "client_ip sees the new list");
                assert_eq!(
                    http(api, "POST", "networks/post", r#"{"values":["192.0.2.7"]}"#)
                        .await
                        .0,
                    200
                );
            }
            assert_eq!(
                rcode(dns).await,
                5,
                "resp_ip sees the replacement, including after reload"
            );
            assert_eq!(
                std::fs::read_to_string(tree.0.join("ips.txt")).unwrap(),
                "192.0.2.7/32\n"
            );
            stop.cancel();
            serving.await.unwrap();
        });
    }
}
