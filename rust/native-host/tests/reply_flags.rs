use std::time::Duration;

use hickory_proto::op::{Message, Query};
use hickory_proto::rr::{Name, RecordType};
use mosdns_native_host::{HostAssembly, HostOptions, TcpServer, UdpServer, compile_yaml};
use mosdns_upstream_core::TransportCancellation;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

/// Every locally constructed reply path, with the rcode it must produce.
const PATHS: [(&str, u16); 6] = [
    ("empty", 5),
    ("reject", 3),
    ("hosts", 0),
    ("ecs_reject", 5),
    ("redirect_reject", 3),
    ("budget", 2),
];

fn config(path: &str, tcp: bool) -> String {
    let rules = match path {
        "empty" => "[{exec: 'mark 1'}]",
        "reject" => "[{exec: 'reject 3'}]",
        "hosts" => "[{exec: '$local'}]",
        "ecs_reject" => "[{exec: '$ecs'}, {exec: 'reject 5'}]",
        "redirect_reject" => "[{exec: '$rewrite'}, {exec: 'reject 3'}]",
        "budget" => "[{exec: '$ecs'}, {exec: 'sleep 1000'}]",
        _ => panic!("unknown test path"),
    };
    let transport = if tcp { "tcp" } else { "udp" };
    format!(
        "log: {{level: error}}
plugins:
  - tag: up
    type: forward
    args: {{upstreams: [{{addr: 'udp://192.0.2.53'}}]}}
  - tag: ecs
    type: ecs_handler
    args: {{preset: '192.0.2.1'}}
  - tag: rewrite
    type: redirect
    args: {{rules: ['flags.example target.example']}}
  - tag: local
    type: hosts
    args: {{entries: ['flags.example 192.0.2.1']}}
  - tag: main
    type: sequence
    args: {rules}
  - tag: dns
    type: {transport}_server
    args: {{entry: main, listen: '127.0.0.1:19911'}}
"
    )
}

fn query(rd: bool, cd: bool) -> Message {
    let mut query = Message::new();
    query
        .set_id(0x4321)
        .set_recursion_desired(rd)
        .set_checking_disabled(cd)
        .add_query(Query::query(
            Name::from_ascii("flags.example.").unwrap(),
            RecordType::A,
        ));
    query
}

async fn read_tcp(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let size = stream.read_u16().await?;
    let mut wire = vec![0; usize::from(size)];
    stream.read_exact(&mut wire).await?;
    Ok(wire)
}

fn run(path: &str, rcode: u16, tcp: bool, rd: bool, cd: bool) {
    let host = HostAssembly::with_options(
        compile_yaml(&config(path, tcp)).unwrap(),
        HostOptions::with_deadline(Duration::from_millis(80)),
    )
    .unwrap();
    host.block_on(async {
        let stop = TransportCancellation::new();
        let shutdown = stop.clone();
        let (address, serving) = if tcp {
            let server = TcpServer::bind(&host, "127.0.0.1:0".parse().unwrap())
                .await
                .unwrap();
            let address = server.local_addr().unwrap();
            (
                address,
                tokio::task::spawn_local(async move { server.serve(shutdown).await.unwrap() }),
            )
        } else {
            let server = UdpServer::bind(&host, "127.0.0.1:0".parse().unwrap())
                .await
                .unwrap();
            let address = server.local_addr().unwrap();
            (
                address,
                tokio::task::spawn_local(async move { server.serve(shutdown).await.unwrap() }),
            )
        };
        let request = query(rd, cd);
        let wire = request.to_vec().unwrap();
        let response = tokio::time::timeout(Duration::from_millis(600), async {
            if tcp {
                let mut stream = TcpStream::connect(address).await.unwrap();
                stream
                    .write_u16(u16::try_from(wire.len()).unwrap())
                    .await
                    .unwrap();
                stream.write_all(&wire).await.unwrap();
                read_tcp(&mut stream).await.unwrap()
            } else {
                let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                udp.send_to(&wire, address).await.unwrap();
                let mut packet = vec![0; 4096];
                let len = udp.recv(&mut packet).await.unwrap();
                packet.truncate(len);
                packet
            }
        })
        .await
        .expect("live request must answer");
        stop.cancel();
        serving.await.unwrap();

        let label = format!("{path} tcp={tcp} rd={rd} cd={cd}");
        let reply = Message::from_vec(&response).unwrap();
        assert_eq!(u16::from(reply.response_code()), rcode, "{label}");
        assert_eq!(reply.id(), request.id(), "{label}");
        assert_eq!(reply.queries(), request.queries(), "{label}");
        assert_eq!(reply.recursion_desired(), rd, "{label}: RD");
        assert_eq!(reply.checking_disabled(), cd, "{label}: CD");
        assert!(reply.recursion_available(), "{label}: RA");
        assert!(!reply.authentic_data(), "{label}: AD");
        // The raw bits, not only the decoded view.
        assert_eq!(response[2] & 0x81, if rd { 0x81 } else { 0x80 }, "{label}");
        assert_eq!(response[3] & 0x90, if cd { 0x90 } else { 0x80 }, "{label}");
    });
}

#[test]
fn local_replies_copy_rd_and_cd_over_udp() {
    for (path, rcode) in PATHS {
        for (rd, cd) in [(true, true), (false, false), (true, false), (false, true)] {
            run(path, rcode, false, rd, cd);
        }
    }
}

#[test]
fn local_replies_copy_rd_and_cd_over_tcp() {
    for (path, rcode) in PATHS {
        for (rd, cd) in [(true, true), (false, false), (true, false), (false, true)] {
            run(path, rcode, true, rd, cd);
        }
    }
}
