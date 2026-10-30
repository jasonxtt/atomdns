//! F-1 system operations, against the real `atomdns` binary: the in-place
//! restart (same process id, saved state loaded again, failure paths), the saved
//! WebUI port taking effect at the next start and the process metrics. The
//! appearance routes were removed in the UI redo (2026-10-23). No public network:
//! the upstream is a local UDP stub.
#![cfg(unix)]
#![allow(clippy::pedantic)]

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn dns_query(name: &str, id: u16) -> Vec<u8> {
    let mut wire = vec![
        (id >> 8) as u8,
        id as u8,
        0x01,
        0x00,
        0,
        1,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    for label in name.split('.') {
        wire.push(label.len() as u8);
        wire.extend_from_slice(label.as_bytes());
    }
    wire.extend_from_slice(&[0, 0, 1, 0, 1]);
    wire
}

/// The query echoed back as an answer carrying 192.0.2.1 (TTL 60).
fn dns_answer(query: &[u8]) -> Vec<u8> {
    let mut wire = query.to_vec();
    wire[2] = 0x81;
    wire[3] = 0x80;
    wire[7] = 1;
    wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 1]);
    wire
}

struct Stub {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Stub {
    fn start() -> (Self, u16) {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            while !flag.load(Ordering::SeqCst) {
                if let Ok((length, peer)) = socket.recv_from(&mut buffer) {
                    let _ = socket.send_to(&dns_answer(&buffer[..length]), peer);
                }
            }
        });
        (
            Self {
                stop,
                handle: Some(handle),
            },
            port,
        )
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn ask(dns: u16, name: &str) -> Option<Vec<u8>> {
    let socket = UdpSocket::bind("127.0.0.1:0").ok()?;
    socket
        .set_read_timeout(Some(Duration::from_millis(800)))
        .ok()?;
    socket
        .send_to(&dns_query(name, 0x4242), ("127.0.0.1", dns))
        .ok()?;
    let mut buffer = [0_u8; 512];
    let (length, _) = socket.recv_from(&mut buffer).ok()?;
    Some(buffer[..length].to_vec())
}

fn http(
    port: u16,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> Option<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(500),
    )
    .ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(content_type) = content_type {
        head.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).ok()?;
    stream.write_all(body).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    let status = std::str::from_utf8(&response[..split])
        .ok()?
        .split(' ')
        .nth(1)?
        .parse()
        .ok()?;
    Some((status, response[split + 4..].to_vec()))
}

fn json(reply: &(u16, Vec<u8>)) -> serde_json::Value {
    serde_json::from_slice(&reply.1)
        .unwrap_or_else(|_| panic!("JSON: {:?}", String::from_utf8_lossy(&reply.1)))
}

struct Host {
    root: PathBuf,
    child: Child,
    stderr: Receiver<String>,
    api: u16,
    dns: u16,
    stub: Stub,
}

/// Spawns `command`, retrying a bounded number of times while the freshly copied program is
/// "Text file busy": another test thread forking at that moment can hold the copy's write
/// handle for an instant. Any other error, or the same one after the last try, panics.
fn spawn_retrying_busy(command: &mut Command) -> Child {
    for _ in 0..50 {
        match command.spawn() {
            Ok(child) => return child,
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("spawn failed: {error}"),
        }
    }
    command.spawn().unwrap()
}

impl Host {
    /// Starts a copy of the binary (so the file can be changed under it) with a cache that is saved at stop.
    fn start(name: &str) -> Self {
        Self::start_with(name, true)
    }

    /// `dump_directory = false` makes the cache's final save fail.
    fn start_with(name: &str, dump_directory: bool) -> Self {
        common::ports::retry_addr_in_use(|| Self::try_start_with(name, dump_directory))
    }

    fn try_start_with(name: &str, dump_directory: bool) -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!("f1-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // The cache's final save needs its directory (a failed save is an exit-2 stop).
        std::fs::create_dir_all(&root).unwrap();
        if dump_directory {
            std::fs::create_dir_all(root.join("dump")).unwrap();
        }
        let exe = root.join("atomdns");
        std::fs::copy(env!("CARGO_BIN_EXE_atomdns"), &exe).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (stub, upstream) = Stub::start();
        let (api, dns) = (free_port(), free_udp_port());
        std::fs::write(
            root.join("config.yaml"),
            format!(
                "log: {{level: error}}\napi: {{http: '127.0.0.1:{api}'}}\nplugins:\n  - tag: cache_main\n    type: cache\n    args: {{dump_file: dump/cache.gz}}\n  - tag: upstream\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{upstream}'}}]}}\n  - tag: main\n    type: sequence\n    args: [{{exec: $cache_main}}, {{exec: $upstream}}]\n  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{dns}', entry: main, enable_audit: false}}\n"
            ),
        )
        .unwrap();
        let mut command = Command::new(&exe);
        command
            .args(["start", "-d"])
            .arg(&root)
            .arg("-c")
            .arg(root.join("config.yaml"))
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = spawn_retrying_busy(&mut command);
        let (sender, stderr) = channel();
        let pipe = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut host = Self {
            root,
            child,
            stderr,
            api,
            dns,
            stub,
        };
        common::ports::wait_ready(
            &mut host.child,
            &host.stderr,
            Duration::from_secs(20),
            || {
                http(api, "GET", "/api/v1/system/health", None, b"")
                    .is_some_and(|reply| reply.0 == 200)
            },
        )?;
        Ok(host)
    }

    fn wait_ready(&self, api: u16) {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if http(api, "GET", "/api/v1/system/health", None, b"")
                .is_some_and(|reply| reply.0 == 200)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "host not ready; stderr: {:?}",
            self.stderr.try_iter().collect::<Vec<_>>()
        );
    }

    fn api(&self, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
        http(self.api, method, path, None, body)
            .unwrap_or_else(|| panic!("{method} {path}: no answer"))
    }

    /// Waits for a stderr line containing `text`.
    fn wait_line(&self, text: &str) {
        let end = Instant::now() + Duration::from_secs(20);
        while Instant::now() < end {
            if let Ok(line) = self.stderr.recv_timeout(Duration::from_millis(100)) {
                if line.contains(text) {
                    return;
                }
            }
        }
        panic!("no stderr line containing {text:?}");
    }

    fn exe(&self) -> PathBuf {
        self.root.join("atomdns")
    }

    fn terminate(&mut self) -> std::process::ExitStatus {
        assert!(
            Command::new("kill")
                .args(["-TERM", &self.child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < end, "no exit after SIGTERM");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stub.stop();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn process_start_ticks(pid: u32) -> String {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    stat.rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_owned()
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out: {what}");
}

#[test]
fn restart_keeps_the_process_id_and_serves_again_from_saved_state() {
    let mut host = Host::start("restart");
    let pid = host.child.id();
    let started = process_start_ticks(pid);
    // Warm the cache, and state a setting that must survive.
    assert!(ask(host.dns, "kept.example.com").is_some());
    let capacity = host.api("POST", "/api/v1/audit/capacity", br#"{"capacity": 1234}"#);
    assert_eq!(capacity.0, 200, "{}", String::from_utf8_lossy(&capacity.1));
    // Capabilities and metrics are real.
    let capabilities = json(&host.api("GET", "/api/v1/capabilities", b""));
    for id in ["system.restart", "system.webui_port", "metrics.process"] {
        assert_eq!(capabilities["ui_operations"][id]["supported"], true, "{id}");
    }
    assert!(
        capabilities["ui_operations"]
            .get("appearance.server")
            .is_none(),
        "the appearance operation is gone"
    );
    assert_eq!(
        host.api("GET", "/api/v1/appearance/button-color", b"").0,
        404,
        "the appearance routes are gone"
    );
    let metrics = String::from_utf8(host.api("GET", "/metrics", b"").1).unwrap();
    for name in [
        "process_cpu_seconds_total",
        "process_start_time_seconds",
        "process_resident_memory_bytes",
        "process_open_fds",
        "atomdns_process_threads",
    ] {
        assert!(
            metrics
                .lines()
                .any(|line| line.starts_with(&format!("{name} "))),
            "{name}"
        );
    }
    assert!(
        !metrics.contains("go_goroutines") && !metrics.contains("go_info"),
        "no Go runtime figures"
    );
    // Restart. The upstream is gone afterwards: an answer proves the saved cache was loaded.
    host.stub.stop();
    let reply = host.api("POST", "/api/v1/system/restart", br#"{"delay_ms": 100}"#);
    assert_eq!(
        (reply.0, json(&reply)),
        (
            200,
            serde_json::json!({"status": "scheduled", "delay_ms": 100})
        )
    );
    host.wait_line("restarting in place");
    wait_until("DNS served again", || {
        ask(host.dns, "kept.example.com").is_some_and(|answer| answer.len() > 12 && answer[7] == 1)
    });
    host.wait_ready(host.api);
    assert_eq!(host.child.id(), pid);
    assert!(
        host.child.try_wait().unwrap().is_none(),
        "the same process is still running"
    );
    assert_eq!(
        process_start_ticks(pid),
        started,
        "the process image was replaced, not the process"
    );
    assert!(
        host.root.join("dump/cache.gz").metadata().unwrap().len() > 0,
        "the final save happened"
    );
    assert_eq!(
        json(&host.api("GET", "/api/v1/audit/capacity", b""))["capacity"],
        1234,
        "settings are kept"
    );
    // A termination signal after a restart still stops the process normally.
    assert!(host.terminate().success());
}

#[test]
fn a_failed_exec_keeps_the_same_process_serving() {
    let mut host = Host::start("exec-fails");
    let pid = host.child.id();
    assert!(ask(host.dns, "still.example.com").is_some());
    let reply = host.api("POST", "/api/v1/system/restart", br#"{"delay_ms": 1500}"#);
    assert_eq!(reply.0, 200);
    // The pre-flight passed; now the file stops being executable before the exec happens.
    std::fs::set_permissions(host.exe(), std::fs::Permissions::from_mode(0o644)).unwrap();
    host.wait_line("restart failed");
    wait_until("DNS served again in the same process", || {
        ask(host.dns, "still.example.com").is_some()
    });
    host.wait_ready(host.api);
    assert_eq!(host.child.id(), pid);
    assert!(host.child.try_wait().unwrap().is_none());
    // A request made while the file is not executable is refused up front and stops nothing.
    let refused = host.api("POST", "/api/v1/system/restart", b"{}");
    assert_eq!(refused.0, 500);
    assert!(
        json(&refused)["error"]
            .as_str()
            .unwrap()
            .contains("not executable")
    );
    assert!(ask(host.dns, "still.example.com").is_some());
    // The restart works again once the file is executable.
    std::fs::set_permissions(host.exe(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        host.api("POST", "/api/v1/system/restart", br#"{"delay_ms": 50}"#)
            .0,
        200
    );
    host.wait_line("restarting in place");
    host.wait_ready(host.api);
    assert_eq!(host.child.id(), pid);
    assert!(host.terminate().success());
}

#[test]
fn a_saved_webui_port_takes_effect_at_the_next_start_and_beats_the_yaml_port() {
    common::ports::retry_addr_in_use(saved_webui_port_scenario);
}

fn saved_webui_port_scenario() -> Result<(), String> {
    let mut host = Host::start("webui-port");
    let before = json(&host.api("GET", "/api/v1/system/webui-port", b""));
    assert_eq!(before["active_port"], host.api);
    assert_eq!(before["pending_restart"], false);
    let target = free_port();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = taken.local_addr().unwrap().port();
    let refused = host.api(
        "POST",
        "/api/v1/system/webui-port",
        format!("{{\"port\": {busy}}}").as_bytes(),
    );
    assert_eq!(refused.0, 409);
    assert_eq!(
        host.api("POST", "/api/v1/system/webui-port", br#"{"port": 70000}"#)
            .0,
        400
    );
    let saved = host.api(
        "POST",
        "/api/v1/system/webui-port",
        format!("{{\"port\": {target}}}").as_bytes(),
    );
    assert_eq!(saved.0, 200);
    assert_eq!(json(&saved)["pending_restart"], true);
    assert_eq!(json(&saved)["message"], "WebUI 端口已保存，重启后生效");
    // Saving does not switch the listener.
    assert_eq!(host.api("GET", "/api/v1/system/webui-port", b"").0, 200);
    assert_eq!(
        json(&host.api("GET", "/api/v1/system/webui-port", b""))["port"],
        target
    );
    assert!(http(target, "GET", "/api/v1/system/health", None, b"").is_none());
    assert_eq!(
        host.api("POST", "/api/v1/system/restart", br#"{"delay_ms": 50}"#)
            .0,
        200
    );
    host.wait_line("restarting in place");
    common::ports::wait_ready(
        &mut host.child,
        &host.stderr,
        Duration::from_secs(20),
        || {
            http(target, "GET", "/api/v1/system/health", None, b"")
                .is_some_and(|reply| reply.0 == 200)
        },
    )?;
    let after = http(target, "GET", "/api/v1/system/webui-port", None, b"").unwrap();
    assert_eq!(json(&after)["active_port"], target);
    assert_eq!(json(&after)["pending_restart"], false);
    assert!(
        http(host.api, "GET", "/api/v1/system/health", None, b"").is_none(),
        "the old port is closed"
    );
    assert!(
        std::fs::read_to_string(host.root.join("config.yaml"))
            .unwrap()
            .contains(&host.api.to_string()),
        "YAML untouched"
    );
    drop(taken);
    assert!(host.terminate().success());
    Ok(())
}

#[test]
fn a_restart_goes_ahead_even_when_the_final_save_fails() {
    let mut host = Host::start_with("save-fails", false);
    let pid = host.child.id();
    assert!(ask(host.dns, "unsaved.example.com").is_some());
    assert_eq!(
        host.api("POST", "/api/v1/system/restart", br#"{"delay_ms": 50}"#)
            .0,
        200
    );
    host.wait_line("restarting anyway");
    host.wait_line("restarting in place");
    host.wait_ready(host.api);
    assert_eq!(host.child.id(), pid);
    assert!(ask(host.dns, "unsaved.example.com").is_some_and(|answer| answer.len() > 12));
    // Without a restart request the same failure is still the exit-2 stop of the reference behaviour.
    assert_eq!(host.terminate().code(), Some(2));
}
