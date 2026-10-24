//! M2-1: while an online update tries the new program out (the guard's
//! `MOSDNS_UPDATE_GUARD_TXN` is set), the flag values are migrated in memory
//! only and the flag routes refuse changes, so a rollback finds the
//! `settings.json` the old program can read. A binary of its own: the
//! variable is process-wide.

use std::path::PathBuf;

use mosdns_native_host::HostAssembly;
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn api(port: u16, method: &str, path: &str, body: &str) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: native\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("http response");
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        body.to_owned(),
    )
}

#[test]
fn an_update_trial_neither_writes_the_migration_nor_accepts_changes() {
    // SAFETY: set before any other thread of this test binary exists.
    unsafe { std::env::set_var("MOSDNS_UPDATE_GUARD_TXN", "/nonexistent/transaction.json") };
    let root: PathBuf = std::env::temp_dir().join(format!("m2-1-trial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("rule")).unwrap();
    std::fs::write(root.join("rule/switch3.txt"), "B").unwrap();
    let (dns, api_port) = (free_port(), free_port());
    std::fs::write(
        root.join("config.yaml"),
        format!(
            "log: {{level: error}}\napi: {{http: \"127.0.0.1:{api_port}\"}}\nplugins:\n  - tag: core_mode\n    type: feature_flag\n    args:\n      label: 核心运行模式\n      description: 测试\n      section: mode\n      kind: choice\n      options: [{{value: compat, label: 兼容}}, {{value: safe, label: 安全}}]\n      default: compat\n      requery_on_change: true\n  - tag: requery\n    type: requery\n    args: {{file: webinfo/requeryconfig.json}}\n  - tag: fwd\n    type: forward\n    args: {{upstreams: [{{addr: \"udp://127.0.0.1:9\"}}]}}\n  - tag: entry\n    type: sequence\n    args:\n      - matches: flag core_mode=safe\n        exec: reject 3\n      - exec: reject 0\n  - tag: listener\n    type: udp_server\n    args: {{entry: entry, listen: \"127.0.0.1:{dns}\"}}\n"
        ),
    )
    .unwrap();
    let host = HostAssembly::from_config_file(&root.join("config.yaml")).expect("assembles");
    assert!(
        !root.join("webinfo/settings.json").exists(),
        "nothing is written during a trial"
    );
    host.block_on(async {
        let bound = host.bind_host().await.expect("binds");
        let shutdown = TransportCancellation::new();
        let scope = shutdown.clone();
        let task = tokio::task::spawn_local(async move {
            bound.serve(scope).await.expect("clean shutdown");
        });
        let (status, body) = api(api_port, "GET", "/api/v1/flags", "").await;
        assert_eq!(status, 200);
        let listed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(listed["migrated"], false);
        assert_eq!(listed["flags"][0]["value"], "safe");
        assert_eq!(listed["flags"][0]["source"], "legacy");
        for (method, body) in [("POST", r#"{"value":"compat"}"#), ("DELETE", "")] {
            let (status, reply) = api(api_port, method, "/api/v1/flags/core_mode", body).await;
            assert_eq!(status, 503, "{method}: {reply}");
        }
        // Even a request that would change nothing is refused during the trial.
        let (status, _) = api(
            api_port,
            "POST",
            "/api/v1/flags/core_mode",
            r#"{"value":"safe"}"#,
        )
        .await;
        assert_eq!(status, 503);
        // A refused change starts no requery task.
        let (status, body) = api(api_port, "GET", "/plugins/requery/status", "").await;
        assert_eq!(status, 200, "{body}");
        let requery: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(requery["task_state"], "idle", "{body}");
        assert!(
            requery["last_run_start_time"]
                .as_str()
                .is_none_or(|time| time.starts_with("0001-01-01")),
            "{body}"
        );
        shutdown.cancel();
        task.await.unwrap();
    });
    assert!(!root.join("webinfo/settings.json").exists());
    drop(host);
    // The final program (started without the variable) migrates.
    // SAFETY: the host above has stopped; no other thread reads the environment.
    unsafe { std::env::remove_var("MOSDNS_UPDATE_GUARD_TXN") };
    drop(HostAssembly::from_config_file(&root.join("config.yaml")).expect("assembles"));
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("webinfo/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        file,
        serde_json::json!({"version": 1, "flags": {"core_mode": "safe"}})
    );
    let _ = std::fs::remove_dir_all(&root);
}
