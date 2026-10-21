//! Concurrent upstream answer selection, against the Go `forward`/`aliapi`
//! `exchange` ranking: an answer carrying an A/AAAA wins at once; otherwise the
//! first NOERROR/NXDOMAIN; otherwise the first other rcode; otherwise the first
//! error. Two scripted UDP upstreams answer after set delays, so the arrival
//! order is fixed whichever one the random start picks first.

use std::fmt::Write;
use std::time::{Duration, Instant};

use mosdns_native_host::{HostAssembly, HostOptions, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;

#[derive(Clone, Copy, Debug)]
enum Reply {
    /// NOERROR with one A record.
    Address,
    /// NOERROR without an address (NODATA).
    NoData,
    NxDomain,
    ServFail,
    Refused,
    /// No datagram at all: the leg ends at its own timeout.
    Silent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Outcome {
    Address,
    Rcode(u8),
}

fn query() -> Vec<u8> {
    let mut wire = vec![0x91, 0x01, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    wire.extend_from_slice(b"\x04race\x07example\x00\x00\x01\x00\x01");
    wire
}

fn reply_wire(query: &[u8], reply: Reply) -> Vec<u8> {
    let mut position = 12;
    while query[position] != 0 {
        position += 1 + usize::from(query[position]);
    }
    let mut wire = query[..position + 5].to_vec();
    wire[2] = 0x81;
    wire[3] = 0x80
        | match reply {
            Reply::NxDomain => 3,
            Reply::ServFail => 2,
            Reply::Refused => 5,
            _ => 0,
        };
    if matches!(reply, Reply::Address) {
        wire[7] = 1;
        wire.extend_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 192, 0, 2, 19]);
    }
    wire
}

/// Runs one query against two upstreams with `(delay in ms, reply)` scripts and
/// returns what the client received and how long that took.
fn race(first: (u64, Reply), second: (u64, Reply), leg_timeout_ms: u32) -> (Outcome, Duration) {
    race_within(first, second, leg_timeout_ms, Duration::from_secs(5))
}

/// Like [`race`], with an explicit request deadline.
fn race_within(
    first: (u64, Reply),
    second: (u64, Reply),
    leg_timeout_ms: u32,
    deadline: Duration,
) -> (Outcome, Duration) {
    let sockets = [first, second].map(|_| std::net::UdpSocket::bind("127.0.0.1:0").unwrap());
    for socket in &sockets {
        socket.set_nonblocking(true).unwrap();
    }
    let mut entries = String::new();
    for (socket, tag) in sockets.iter().zip(["a", "b"]) {
        writeln!(
            entries,
            "        - {{tag: {tag}, addr: 'udp://{}', upstream_query_timeout: {leg_timeout_ms}}}",
            socket.local_addr().unwrap()
        )
        .unwrap();
    }
    let source = format!(
        "log:\n  level: error\nplugins:\n  - tag: upstream\n    type: forward\n    args:\n      concurrent: 2\n      upstreams:\n{entries}  - tag: entry\n    type: sequence\n    args:\n      - exec: '$upstream'\n  - tag: listener\n    type: udp_server\n    args:\n      entry: entry\n      listen: 127.0.0.1:15400\n"
    );
    let host = HostAssembly::with_options(
        compile_yaml(&source).unwrap(),
        HostOptions::with_deadline(deadline),
    )
    .unwrap();
    host.block_on(async {
        let peers = sockets
            .into_iter()
            .zip([first, second])
            .map(|(socket, (delay, reply))| {
                let socket = tokio::net::UdpSocket::from_std(socket).unwrap();
                tokio::task::spawn_local(async move {
                    let mut buffer = [0; 512];
                    let (length, client) = socket.recv_from(&mut buffer).await.unwrap();
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    if !matches!(reply, Reply::Silent) {
                        // The late sender may find its leg already cancelled.
                        let _ = socket
                            .send_to(&reply_wire(&buffer[..length], reply), client)
                            .await;
                    }
                })
            })
            .collect::<Vec<_>>();
        let server = UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let address = server.local_addr().unwrap();
        let shutdown = TransportCancellation::new();
        let service = tokio::task::spawn_local(server.serve(shutdown.clone()));
        let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let started = Instant::now();
        client.send_to(&query(), address).await.unwrap();
        let mut buffer = [0; 512];
        let length = tokio::time::timeout(Duration::from_secs(4), client.recv(&mut buffer))
            .await
            .expect("the client must get an answer")
            .unwrap();
        let elapsed = started.elapsed();
        shutdown.cancel();
        service.await.unwrap().unwrap();
        for peer in peers {
            peer.abort();
            let _ = peer.await;
        }
        let response = &buffer[..length];
        let outcome = if mosdns_dns_core::observe_answer_addresses(response)
            .unwrap()
            .is_empty()
        {
            Outcome::Rcode(response[3] & 0x0f)
        } else {
            Outcome::Address
        };
        (outcome, elapsed)
    })
}

use Reply::{Address, NoData, NxDomain, Refused, ServFail, Silent};

fn check(first: (u64, Reply), second: (u64, Reply), expected: Outcome) {
    // Both orders of the same script: the start leg is random, the result is not.
    for (a, b) in [(first, second), (second, first)] {
        let (outcome, _) = race(a, b, 1500);
        assert_eq!(outcome, expected, "{a:?} against {b:?}");
    }
}

#[test]
fn an_address_beats_every_other_result_whatever_the_arrival_order() {
    check((0, ServFail), (150, Address), Outcome::Address);
    check((0, NoData), (150, Address), Outcome::Address);
    check((0, NxDomain), (150, Address), Outcome::Address);
    check((0, Refused), (150, Address), Outcome::Address);
}

#[test]
fn noerror_or_nxdomain_beats_servfail_and_refused_even_when_later() {
    check((0, ServFail), (150, NoData), Outcome::Rcode(0));
    check((0, ServFail), (150, NxDomain), Outcome::Rcode(3));
    check((0, Refused), (150, NoData), Outcome::Rcode(0));
}

#[test]
fn the_first_arrival_wins_inside_one_rank() {
    check((0, NoData), (150, NxDomain), Outcome::Rcode(0));
    check((0, NxDomain), (150, NoData), Outcome::Rcode(3));
    check((0, Refused), (150, ServFail), Outcome::Rcode(5));
    check((0, ServFail), (150, Refused), Outcome::Rcode(2));
}

#[test]
fn a_silent_leg_does_not_hide_the_other_legs_result() {
    check((0, Silent), (50, ServFail), Outcome::Rcode(2));
    check((0, Silent), (50, NoData), Outcome::Rcode(0));
    check((0, Silent), (50, Address), Outcome::Address);
}

#[test]
fn every_leg_failing_answers_servfail_locally() {
    let (outcome, _) = race((0, Silent), (0, Silent), 300);
    assert_eq!(outcome, Outcome::Rcode(2));
}

#[test]
fn an_address_answers_at_once_and_cancels_the_slower_leg() {
    for (a, b) in [
        ((0, Address), (3000, ServFail)),
        ((3000, ServFail), (0, Address)),
    ] {
        let (outcome, elapsed) = race(a, b, 3500);
        assert_eq!(outcome, Outcome::Address);
        assert!(elapsed < Duration::from_millis(1500), "waited {elapsed:?}");
    }
}

/// Go's `forward` returns an error (so the client gets SERVFAIL) when the request
/// context ends first, even with a NOERROR already collected; `aliapi` and this
/// host return the collected best. Recorded in `research/go-semantics.md`.
#[test]
fn a_leg_silent_to_the_request_deadline_keeps_the_collected_answer() {
    for (a, b) in [((0, NoData), (0, Silent)), ((0, Silent), (0, NoData))] {
        let (outcome, elapsed) = race_within(a, b, 4000, Duration::from_millis(500));
        assert_eq!(outcome, Outcome::Rcode(0), "{a:?} against {b:?}");
        assert!(elapsed < Duration::from_millis(2500), "waited {elapsed:?}");
    }
}
