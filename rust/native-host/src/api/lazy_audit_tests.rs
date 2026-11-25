use std::net::SocketAddr;
use std::time::{Duration, UNIX_EPOCH};

use atomdns_dns_core::observe_response_rcode;

use super::{AuditFilter, CompiledAuditFilter, project_log, reference_audit_record_matches};
use crate::execution::{observe_and_diagnose_wire, reference_observe_and_diagnose_wire};
use crate::observer::{
    AnswerDetailsStatus, AuditAnswer, AuditRecord, AuditRetention, CacheStatus, QueryObserver,
    QueryTerminalOutcome, QueryTransport, ResponseDetails, ResponseFlags, ResponseSource,
    ResponseState, TerminalObservation,
};

struct XorShift64(u64);

impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_usize(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() as usize) % bound
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn hdr(flags_hi: u8, flags_lo: u8, qd: u16, an: u16, ns: u16, ar: u16) -> Vec<u8> {
    let mut b = vec![0x12, 0x34, flags_hi, flags_lo, 0, 0, 0, 0, 0, 0, 0, 0];
    b[4..6].copy_from_slice(&qd.to_be_bytes());
    b[6..8].copy_from_slice(&an.to_be_bytes());
    b[8..10].copy_from_slice(&ns.to_be_bytes());
    b[10..12].copy_from_slice(&ar.to_be_bytes());
    b
}

fn encode_name(labels: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for label in labels {
        out.push(u8::try_from(label.len()).expect("label len <= 63"));
        out.extend_from_slice(label);
    }
    out.push(0);
    out
}

fn question(labels: &[&[u8]], qtype: u16) -> Vec<u8> {
    let mut b = encode_name(labels);
    b.extend_from_slice(&qtype.to_be_bytes());
    b.extend_from_slice(&1_u16.to_be_bytes());
    b
}

fn rr_ptr(owner_ptr: u16, rrtype: u16, ttl: u32, rdata: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(12 + rdata.len());
    b.extend_from_slice(&(0xc000 | owner_ptr).to_be_bytes());
    b.extend_from_slice(&rrtype.to_be_bytes());
    b.extend_from_slice(&1_u16.to_be_bytes());
    b.extend_from_slice(&ttl.to_be_bytes());
    b.extend_from_slice(&u16::try_from(rdata.len()).unwrap().to_be_bytes());
    b.extend_from_slice(rdata);
    b
}

fn opt_rr(ext_rcode: u8) -> Vec<u8> {
    let mut b = vec![
        0x00, 0x00, 0x29, 0x04, 0xd0, ext_rcode, 0x00, 0x80, 0x00, 0x00, 0x00,
    ];
    b[5] = ext_rcode;
    b
}

fn build_resp(
    flags_hi: u8,
    flags_lo: u8,
    qlabels: &[&[u8]],
    qtype: u16,
    answers: &[Vec<u8>],
    authorities: &[Vec<u8>],
    additionals: &[Vec<u8>],
) -> Vec<u8> {
    let mut w = hdr(
        flags_hi,
        flags_lo,
        1,
        answers.len() as u16,
        authorities.len() as u16,
        additionals.len() as u16,
    );
    w.extend(question(qlabels, qtype));
    for sec in [answers, authorities, additionals] {
        for r in sec {
            w.extend_from_slice(r);
        }
    }
    w
}

fn seed_packets() -> Vec<Vec<u8>> {
    let mut seeds = Vec::new();
    let q = [b"www".as_slice(), b"Example".as_slice(), b"org".as_slice()];

    // 1. Single A
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        1,
        &[rr_ptr(12, 1, 300, &[192, 0, 2, 1])],
        &[],
        &[],
    ));
    // 2. Two A
    seeds.push(build_resp(
        0x85,
        0x80,
        &q,
        1,
        &[
            rr_ptr(12, 1, 60, &[192, 0, 2, 1]),
            rr_ptr(12, 1, 120, &[198, 51, 100, 2]),
        ],
        &[],
        &[],
    ));
    // 3. Three A (Packed)
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        1,
        &[
            rr_ptr(12, 1, 10, &[192, 0, 2, 1]),
            rr_ptr(12, 1, 20, &[198, 51, 100, 2]),
            rr_ptr(12, 1, 30, &[203, 0, 113, 3]),
        ],
        &[],
        &[],
    ));
    // 4. Single AAAA
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        28,
        &[rr_ptr(
            12,
            28,
            45,
            &[0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7],
        )],
        &[],
        &[],
    ));
    // 5. IPv4-mapped AAAA
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        28,
        &[rr_ptr(
            12,
            28,
            45,
            &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 192, 0, 2, 9],
        )],
        &[],
        &[],
    ));
    // 6. CNAME chain with compression pointers + 2x A
    let mut cname_chain = build_resp(0x81, 0x80, &q, 1, &[], &[], &[]);
    cname_chain[6..8].copy_from_slice(&4_u16.to_be_bytes());
    let cname1_rdata = [4, b'E', b'd', b'g', b'e', 0xc0, 0x0c];
    let cname1_rr = rr_ptr(12, 5, 90, &cname1_rdata);
    let cname1_target_off = (cname_chain.len() + 12) as u16;
    cname_chain.extend_from_slice(&cname1_rr);
    let cname2_rdata = [
        3,
        b'c',
        b'd',
        b'n',
        (0xc0 | ((cname1_target_off >> 8) as u8)),
        (cname1_target_off & 0xff) as u8,
    ];
    cname_chain.extend_from_slice(&rr_ptr(cname1_target_off, 5, 80, &cname2_rdata));
    cname_chain.extend_from_slice(&rr_ptr(cname1_target_off, 1, 60, &[198, 51, 100, 10]));
    cname_chain.extend_from_slice(&rr_ptr(cname1_target_off, 1, 60, &[203, 0, 113, 11]));
    seeds.push(cname_chain);

    // 7. CNAME with escaped characters (dot, backslash, NUL, space, non-ASCII)
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        5,
        &[rr_ptr(
            12,
            5,
            15,
            &[6, b'a', b'.', b'\\', 0x00, b' ', 0xff, 0],
        )],
        &[],
        &[],
    ));
    // 8. NS + PTR + MX + root name
    let mut mx_rdata = vec![0x00, 0x0a];
    mx_rdata.extend_from_slice(&[4, b'm', b'a', b'i', b'l', 0xc0, 0x0c]);
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        15,
        &[
            rr_ptr(12, 2, 100, &[3, b'n', b's', b'1', 0xc0, 0x0c]),
            rr_ptr(12, 12, 100, &[0]),
            rr_ptr(12, 15, 100, &mx_rdata),
        ],
        &[],
        &[],
    ));
    // 9. TXT (empty, double-empty, multi-chunk, UTF-8 + invalid UTF-8)
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        16,
        &[
            rr_ptr(12, 16, 10, &[]),
            rr_ptr(12, 16, 11, &[0, 0]),
            rr_ptr(
                12,
                16,
                12,
                &[4, b'v', b'=', b'1', b' ', 4, 0xff, 0xc3, 0x28, b'z'],
            ),
        ],
        &[],
        &[],
    ));
    // 10. Unknown/Raw RR (empty + non-empty) + OPT extended RCODE
    seeds.push(build_resp(
        0x83,
        0x80,
        &q,
        65,
        &[
            rr_ptr(12, 65, 50, &[0, 1, 0, 0, 1, 0, 3, 2, b'h', b'2']),
            rr_ptr(12, 65000, 0, &[]),
        ],
        &[],
        &[opt_rr(1)],
    ));
    // 11. Empty NOERROR / NXDOMAIN / OPT BADVERS
    seeds.push(build_resp(0x81, 0x80, &q, 1, &[], &[], &[]));
    seeds.push(build_resp(0x81, 0x83, &q, 1, &[], &[], &[opt_rr(2)]));
    // 12. Multiple OPTs (last wins) + Authority + Additional
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        1,
        &[rr_ptr(12, 1, 30, &[192, 0, 2, 55])],
        &[rr_ptr(12, 2, 30, &[3, b'n', b's', b'1', 0xc0, 0x0c])],
        &[opt_rr(1), opt_rr(3)],
    ));
    // 13. Invalid answer RDATA followed by valid OPT extended RCODE
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        1,
        &[rr_ptr(12, 1, 30, &[192, 0, 2])],
        &[],
        &[opt_rr(1)],
    ));
    // 14. 256-byte max expanded name (255 non-zero label bytes + trailing 0)
    let mut max_name = Vec::with_capacity(256);
    for _ in 0..3 {
        max_name.push(63);
        max_name.extend_from_slice(&[b'x'; 63]);
    }
    max_name.push(62);
    max_name.extend_from_slice(&[b'y'; 62]);
    max_name.push(0);
    seeds.push(build_resp(
        0x81,
        0x80,
        &q,
        5,
        &[rr_ptr(12, 5, 60, &max_name)],
        &[],
        &[],
    ));
    // 15. Self-pointer QNAME + OPT extended RCODE
    let mut self_ptr = hdr(0x81, 0x80, 1, 0, 0, 1);
    self_ptr.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
    self_ptr.extend_from_slice(&opt_rr(1));
    seeds.push(self_ptr);
    // 16. Edge headers: empty, short, QR=0
    seeds.push(Vec::new());
    seeds.push(vec![0x12, 0x34, 0x87]);
    seeds.push(vec![0x12, 0x34, 0x83, 0x85]);
    seeds.push(hdr(0x01, 0x82, 1, 0, 0, 0));

    seeds
}

fn template_record(has_wire: bool, rcode: u16, details: ResponseDetails) -> AuditRecord {
    AuditRecord {
        timestamp: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        client_addr: "192.0.2.10:53000".parse::<SocketAddr>().unwrap(),
        transport: QueryTransport::Udp,
        qname: "www.example.org.".to_owned(),
        qtype: 1,
        qclass: 1,
        trace_id: "n-11111111111111111111111111111111-0000000000000001".into(),
        elapsed: Duration::from_micros(42),
        terminal_outcome: QueryTerminalOutcome::SendSucceeded,
        response: if !has_wire {
            ResponseState::NoResponse
        } else {
            ResponseState::Dns {
                rcode,
                source: ResponseSource::Cache,
            }
        },
        response_details: details,
        cache_status: CacheStatus::Hit,
        final_sequence: Some("main".into()),
        matched_group: None,
        domain_set: Some("白名单".into()),
        effective_tag: Some("白名单".into()),
        matched_rule_source: Some("provider:cn".into()),
        final_upstream: Some("domestic".into()),
        upstream_targets: Some("198.51.100.53:53".into()),
        selected_upstream: Some("198.51.100.53:53".into()),
        upstream_attempts: Vec::new(),
        upstream_diagnostics: None,
        failure_provenance: None,
    }
}

#[test]
fn single_pass_wire_diagnosis_matches_reference_oracle_across_100k_mutated_packets() {
    const SEED: u64 = 0x5EED_5005_2026_1125;
    const MUTATED_CASES: usize = 100_000;

    let seeds = seed_packets();
    let mut rng = XorShift64(SEED);
    let mut retention = AuditRetention::default();

    let mut verify = |case_id: usize, wire: &[u8]| {
        let (ref_rcode, ref_details) = reference_observe_and_diagnose_wire(wire);
        let (new_rcode, audit_response) = observe_and_diagnose_wire(wire);
        let fast_rcode = if wire.is_empty() {
            0
        } else {
            observe_response_rcode(wire)
        };

        assert_eq!(
            new_rcode,
            ref_rcode,
            "rcode mismatch at case={case_id} seed={SEED:#x} wire={}",
            hex(wire)
        );
        assert_eq!(
            fast_rcode,
            ref_rcode,
            "observe_response_rcode mismatch at case={case_id} seed={SEED:#x} wire={}",
            hex(wire)
        );

        let has_wire = !wire.is_empty();
        let base_record = template_record(has_wire, new_rcode, ResponseDetails::no_response());
        let retained = retention.retain_with_response(base_record, audit_response);
        let expanded = retained.expand();
        let ref_record = template_record(has_wire, ref_rcode, ref_details.clone());

        assert_eq!(
            expanded.response_details,
            ref_details,
            "ResponseDetails mismatch at case={case_id} seed={SEED:#x} wire={}",
            hex(wire)
        );

        let new_json = serde_json::to_string(&project_log(&expanded)).expect("new json");
        let ref_json = serde_json::to_string(&project_log(&ref_record)).expect("ref json");
        assert_eq!(
            new_json,
            ref_json,
            "API JSON mismatch at case={case_id} seed={SEED:#x} wire={}",
            hex(wire)
        );
    };

    for (idx, seed) in seeds.iter().enumerate() {
        verify(idx, seed);
    }

    for i in 0..MUTATED_CASES {
        let mut wire = seeds[rng.next_usize(seeds.len())].clone();
        let mutation_count = 1 + rng.next_usize(3);
        for _ in 0..mutation_count {
            match rng.next_usize(9) {
                // Truncate at arbitrary boundary
                0 if !wire.is_empty() => {
                    let cut = rng.next_usize(wire.len() + 1);
                    wire.truncate(cut);
                }
                // Flip 1 byte
                1 if !wire.is_empty() => {
                    let pos = rng.next_usize(wire.len());
                    wire[pos] ^= (1_u8 << rng.next_usize(8)).max(1);
                }
                // Overwrite arbitrary byte
                2 if !wire.is_empty() => {
                    let pos = rng.next_usize(wire.len());
                    wire[pos] = (rng.next_u64() & 0xff) as u8;
                }
                // Mutate header flags / QR / RCODE
                3 if wire.len() >= 4 => {
                    let pos = 2 + rng.next_usize(2);
                    wire[pos] = (rng.next_u64() & 0xff) as u8;
                }
                // Mutate section counts (qd/an/ns/ar)
                4 if wire.len() >= 12 => {
                    let field = 4 + rng.next_usize(4) * 2;
                    let val = [0_u16, 1, 2, 3, 5, 16, 0x00ff][rng.next_usize(7)];
                    wire[field..field + 2].copy_from_slice(&val.to_be_bytes());
                }
                // Inject compression pointer (self-loop, mutual loop, or OOB)
                5 if wire.len() >= 14 => {
                    let pos = 12 + rng.next_usize(wire.len() - 13);
                    match rng.next_usize(3) {
                        0 => {
                            // Self-pointer to `pos`
                            wire[pos] = 0xc0 | ((pos >> 8) as u8 & 0x3f);
                            wire[pos + 1] = (pos & 0xff) as u8;
                        }
                        1 => {
                            // Pointer to offset 12 (QNAME)
                            wire[pos] = 0xc0;
                            wire[pos + 1] = 0x0c;
                        }
                        _ => {
                            // Out-of-bounds pointer
                            wire[pos] = 0xc0;
                            wire[pos + 1] = 0xff;
                        }
                    }
                }
                // Append an OPT record with non-zero extended RCODE and bump arcount
                6 if wire.len() >= 12 => {
                    let ar = u16::from_be_bytes([wire[10], wire[11]]).saturating_add(1);
                    wire[10..12].copy_from_slice(&ar.to_be_bytes());
                    wire.extend_from_slice(&opt_rr(1 + (rng.next_usize(15) as u8)));
                }
                // Append a malformed A/TXT/CNAME answer and bump ancount
                7 if wire.len() >= 12 => {
                    let an = u16::from_be_bytes([wire[6], wire[7]]).saturating_add(1);
                    wire[6..8].copy_from_slice(&an.to_be_bytes());
                    let bad = match rng.next_usize(4) {
                        0 => rr_ptr(12, 1, 30, &[192, 0, 2]),        // short A
                        1 => rr_ptr(12, 28, 30, &[0; 15]),           // short AAAA
                        2 => rr_ptr(12, 16, 30, &[10, b'a', b'b']),  // overflowing TXT
                        _ => rr_ptr(12, 5, 30, &[0xc0, 0x0c, 0xaa]), // trailing CNAME byte
                    };
                    wire.extend_from_slice(&bad);
                }
                // Replace with short random bytes
                _ => {
                    let len = rng.next_usize(32);
                    wire.clear();
                    for _ in 0..len {
                        wire.push((rng.next_u64() & 0xff) as u8);
                    }
                }
            }
        }
        verify(seeds.len() + i, &wire);
    }
}

#[test]
fn compiled_audit_filter_matches_reference_across_10k_diverse_records() {
    const RECORD_COUNT: usize = 10_000;
    let observer = QueryObserver::new(
        true,
        ["domestic".to_owned(), "foreign".to_owned()],
        RECORD_COUNT,
    );
    let q = [b"host".as_slice(), b"example".as_slice(), b"com".as_slice()];

    let mut cname_packet = build_resp(0x81, 0x80, &q, 1, &[], &[], &[]);
    cname_packet[6..8].copy_from_slice(&3_u16.to_be_bytes());
    cname_packet.extend_from_slice(&rr_ptr(
        12,
        5,
        90,
        &[4, b'E', b'd', b'g', b'e', 3, b'c', b'd', b'n', 0xc0, 0x0c],
    ));
    cname_packet.extend_from_slice(&rr_ptr(12, 1, 60, &[198, 51, 100, 10]));
    cname_packet.extend_from_slice(&rr_ptr(12, 1, 60, &[203, 0, 113, 20]));

    let escaped_cname_packet = build_resp(
        0x81,
        0x80,
        &q,
        5,
        &[rr_ptr(12, 5, 30, &[5, b'a', b'.', b'b', b'\\', b'c', 0])],
        &[],
        &[],
    );
    let txt_ip_packet = build_resp(
        0x81,
        0x80,
        &q,
        16,
        &[rr_ptr(
            12,
            16,
            60,
            &[
                13, b'1', b'9', b'8', b'.', b'5', b'1', b'.', b'1', b'0', b'0', b'.', b'4', b'2',
            ],
        )],
        &[],
        &[],
    );
    let txt_unicode_packet = build_resp(
        0x81,
        0x80,
        &q,
        16,
        &[rr_ptr(
            12,
            16,
            60,
            &[6, 0xe6, 0xb5, 0x8b, 0xe8, 0xaf, 0x95, 2, 0xff, b'x'],
        )],
        &[],
        &[],
    );
    let raw_packet = build_resp(
        0x81,
        0x80,
        &q,
        65000,
        &[rr_ptr(12, 65000, 0, &[0xde, 0xad])],
        &[],
        &[],
    );
    let bad_packet = build_resp(
        0x81,
        0x80,
        &q,
        1,
        &[rr_ptr(12, 1, 60, &[192, 0, 2])],
        &[],
        &[],
    );

    for i in 0..RECORD_COUNT {
        let client: SocketAddr = match i % 4 {
            0 => format!("192.0.2.{}:53000", (i % 250) + 1).parse().unwrap(),
            1 => format!("[2001:db8::{:x}]:53001", (i % 250) + 1)
                .parse()
                .unwrap(),
            2 => format!("[::ffff:192.0.2.{}]:53002", (i % 250) + 1)
                .parse()
                .unwrap(),
            _ => "198.51.100.9:53003".parse().unwrap(),
        };
        let qname = match i % 5 {
            0 => ".".to_owned(),
            1 => format!("Site{i}.Example.COM."),
            _ => format!("host-{}.example.com.", i % 200),
        };
        let domain_set = match i % 4 {
            0 => Some("白名单".into()),
            1 => Some("代理域名".into()),
            2 => Some("白名单|订阅代理".into()),
            _ => None,
        };
        let effective_tag = match i % 3 {
            0 => Some("白名单".into()),
            1 => Some("代理域名".into()),
            _ => None,
        };

        let make_base = |rcode: u16, details: ResponseDetails| AuditRecord {
            timestamp: UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i as u64),
            client_addr: client,
            transport: QueryTransport::Udp,
            qname: qname.clone(),
            qtype: 1,
            qclass: 1,
            trace_id: format!("n-11111111111111111111111111111111-{i:016x}").into(),
            elapsed: Duration::from_micros(100),
            terminal_outcome: QueryTerminalOutcome::SendSucceeded,
            response: ResponseState::Dns {
                rcode,
                source: ResponseSource::Cache,
            },
            response_details: details,
            cache_status: CacheStatus::Hit,
            final_sequence: Some("main".into()),
            matched_group: None,
            domain_set: domain_set.clone(),
            effective_tag: effective_tag.clone(),
            matched_rule_source: Some("provider:测试".into()),
            final_upstream: Some("domestic".into()),
            upstream_targets: Some("198.51.100.53:53".into()),
            selected_upstream: Some(if i % 2 == 0 {
                "198.51.100.53:53".into()
            } else {
                "[2001:db8::53]:853".into()
            }),
            upstream_attempts: Vec::new(),
            upstream_diagnostics: None,
            failure_provenance: None,
        };

        observer.admit_query();
        if i % 12 == 11 {
            // Exercise Verbatim fallback with synthetic ResponseDetails.
            let synthetic = ResponseDetails {
                rcode: 0,
                flags: ResponseFlags {
                    aa: false,
                    tc: true,
                    ra: true,
                },
                answers: vec![
                    AuditAnswer {
                        rrtype: 1,
                        ttl: 60,
                        data: "192.0.2.88".to_owned(),
                    },
                    AuditAnswer {
                        rrtype: 5,
                        ttl: 30,
                        data: "Synthetic.Alias.Example.".to_owned(),
                    },
                    AuditAnswer {
                        rrtype: 65000,
                        ttl: 0,
                        data: "\\000 arbitrary 原始".to_owned(),
                    },
                ],
                answer_details_status: AnswerDetailsStatus::DecodeError,
                answer_decode_error: Some("truncated answer".to_owned()),
            };
            observer.record_terminal(TerminalObservation::default(), |_| make_base(0, synthetic));
            continue;
        }

        let wire: Vec<u8> = match i % 12 {
            0 => build_resp(
                0x81,
                0x80,
                &q,
                1,
                &[rr_ptr(
                    12,
                    1,
                    (i % 300) as u32,
                    &[198, 51, 100, ((i % 250) + 1) as u8],
                )],
                &[],
                &[],
            ),
            1 => build_resp(
                0x81,
                0x80,
                &q,
                1,
                &[
                    rr_ptr(12, 1, 60, &[192, 0, 2, ((i % 250) + 1) as u8]),
                    rr_ptr(12, 1, 90, &[203, 0, 113, ((i % 250) + 1) as u8]),
                ],
                &[],
                &[],
            ),
            2 => build_resp(
                0x81,
                0x80,
                &q,
                28,
                &[rr_ptr(
                    12,
                    28,
                    45,
                    &[
                        0x20,
                        0x01,
                        0x0d,
                        0xb8,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        0,
                        ((i % 250) + 1) as u8,
                    ],
                )],
                &[],
                &[],
            ),
            3 => build_resp(
                0x81,
                0x80,
                &q,
                28,
                &[rr_ptr(
                    12,
                    28,
                    45,
                    &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 198, 51, 100, 7],
                )],
                &[],
                &[],
            ),
            4 => build_resp(
                0x81,
                0x80,
                &q,
                1,
                &[
                    rr_ptr(12, 1, 10, &[192, 0, 2, 1]),
                    rr_ptr(12, 1, 20, &[198, 51, 100, 2]),
                    rr_ptr(12, 1, 30, &[203, 0, 113, 3]),
                ],
                &[],
                &[],
            ),
            5 => cname_packet.clone(),
            6 => escaped_cname_packet.clone(),
            7 => txt_ip_packet.clone(),
            8 => txt_unicode_packet.clone(),
            9 => raw_packet.clone(),
            10 => build_resp(0x81, 0x83, &q, 1, &[], &[], &[]),
            _ => bad_packet.clone(),
        };
        let (rcode, response) = observe_and_diagnose_wire(&wire);
        observer.record_terminal_compact(TerminalObservation::default(), |_| {
            (make_base(rcode, ResponseDetails::no_response()), response)
        });
    }

    let snapshot = observer.audit_read_snapshot().expect("read snapshot");
    assert_eq!(snapshot.records.len(), RECORD_COUNT);

    let filters = vec![
        // answer_ip variations
        AuditFilter {
            answer_ip: Some("198.51.100.7".into()),
            ..Default::default()
        },
        AuditFilter {
            answer_ip: Some("::ffff:198.51.100.7".into()),
            ..Default::default()
        },
        AuditFilter {
            answer_ip: Some("[2001:db8::7]".into()),
            ..Default::default()
        },
        AuditFilter {
            answer_ip: Some("2001:DB8::7".into()),
            ..Default::default()
        },
        AuditFilter {
            answer_ip: Some("198.51.100.10:53".into()),
            ..Default::default()
        },
        AuditFilter {
            answer_ip: Some("192.0.2.88".into()),
            ..Default::default()
        },
        AuditFilter {
            answer_ip: Some("203.0.113.254".into()),
            ..Default::default()
        },
        AuditFilter {
            answer_ip: Some("not-an-ip".into()),
            ..Default::default()
        },
        // cname (case-sensitive)
        AuditFilter {
            cname: Some("Edge.cdn".into()),
            ..Default::default()
        },
        AuditFilter {
            cname: Some("edge.cdn".into()),
            ..Default::default()
        },
        AuditFilter {
            cname: Some("a\\.b".into()),
            ..Default::default()
        },
        AuditFilter {
            cname: Some("Synthetic.Alias".into()),
            ..Default::default()
        },
        // q exact=false
        AuditFilter {
            q: Some("198.51.100.7".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("[198.51.100.7]".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("198.51.100.7:53".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("::ffff:198.51.100.7".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("2001:DB8::7".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("198.51.100".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("2001:db8".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("2001:DB8".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("ffff".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("198.51.100.42".into()),
            ..Default::default()
        }, // TXT containing IP
        AuditFilter {
            q: Some("edge.cdn".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("白名单".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("原始".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("测试".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("\\# 2 dead".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("00000000000000ff".into()),
            ..Default::default()
        },
        AuditFilter {
            q: Some("nonexistent-keyword".into()),
            ..Default::default()
        },
        // q exact=true
        AuditFilter {
            q: Some("198.51.100.7".into()),
            exact: true,
            ..Default::default()
        },
        AuditFilter {
            q: Some("2001:DB8::7".into()),
            exact: true,
            ..Default::default()
        },
        AuditFilter {
            q: Some("2001:DB8".into()),
            exact: true,
            ..Default::default()
        },
        AuditFilter {
            q: Some("Edge.cdn.host.example.com.".into()),
            exact: true,
            ..Default::default()
        },
        AuditFilter {
            q: Some("edge.cdn.host.example.com.".into()),
            exact: true,
            ..Default::default()
        },
        AuditFilter {
            q: Some("198.51.100.42".into()),
            exact: true,
            ..Default::default()
        },
        AuditFilter {
            q: Some(".".into()),
            exact: true,
            ..Default::default()
        },
        // Combined filters
        AuditFilter {
            domain_set: Some("白名单".into()),
            answer_ip: Some("198.51.100.7".into()),
            ..Default::default()
        },
        AuditFilter {
            client_ips: vec!["192.0.2.1".into(), "invalid".into()],
            q: Some("example.com".into()),
            ..Default::default()
        },
        AuditFilter {
            client_ips: vec!["not-an-ip".into()],
            ..Default::default()
        },
        AuditFilter {
            domain: Some("host-".into()),
            cname: Some("Edge.cdn".into()),
            ..Default::default()
        },
        AuditFilter {
            exact_domain: Some("host-5.example.com".into()),
            q: Some("203.0.113.20".into()),
            ..Default::default()
        },
    ];

    let mut scratch = String::new();
    for (filter_index, filter) in filters.iter().enumerate() {
        let compiled = CompiledAuditFilter::compile(filter);
        for (rec_index, record) in snapshot.records.iter().enumerate() {
            let expected = reference_audit_record_matches(record, filter);
            let actual = compiled.matches(record, &mut scratch);
            assert_eq!(
                actual, expected,
                "filter mismatch at filter_index={filter_index} filter={filter:?} rec_index={rec_index}"
            );
        }
    }
}

#[test]
fn all_answer_rrtypes_truncation_and_decode_errors_project_identically_to_json() {
    let q = [b"full".as_slice(), b"example".as_slice(), b"org".as_slice()];
    let mut mx = vec![0x00, 0x14];
    mx.extend_from_slice(&[4, b'm', b'a', b'i', b'l', 0xc0, 0x0c]);

    let full_wire = build_resp(
        0x85,
        0x80,
        &q,
        1,
        &[
            rr_ptr(12, 1, 60, &[192, 0, 2, 1]),
            rr_ptr(
                12,
                28,
                61,
                &[0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            ),
            rr_ptr(12, 2, 61, &[2, b'n', b's', 0xc0, 0x0c]),
            rr_ptr(12, 5, 62, &[5, b'a', b'.', b'\\', 0x00, b' ', 0xc0, 0x0c]),
            rr_ptr(12, 12, 62, &[0]),
            rr_ptr(12, 15, 63, &mx),
            rr_ptr(12, 16, 64, &[3, b't', b'x', b't', 2, 0xff, b'2']),
            rr_ptr(12, 16, 64, &[]),
            rr_ptr(12, 16, 64, &[0, 0]),
            rr_ptr(12, 65000, 65, &[0xca, 0xfe]),
            rr_ptr(12, 65001, 66, &[]),
        ],
        &[],
        &[],
    );

    // Oversized response truncated for 512-byte UDP client budget.
    let many_a: Vec<Vec<u8>> = (0..64)
        .map(|idx| rr_ptr(12, 1, 120, &[198, 51, 100, (idx + 1) as u8]))
        .collect();
    let oversized = build_resp(0x81, 0x80, &q, 1, &many_a, &[], &[]);
    let mut query = hdr(0x01, 0x00, 1, 0, 0, 0);
    query.extend(question(&q, 1));
    let (truncated_wire, records_removed) =
        crate::udp_response::prepare_response(oversized, 512, &query).expect("truncate UDP");
    assert!(records_removed);

    let decode_error_wires: [(&str, Vec<u8>); 4] = [
        ("truncated_message", vec![0x12, 0x34, 0x81, 0x80]),
        ("invalid_response", hdr(0x01, 0x80, 1, 0, 0, 0)),
        (
            "bad_name",
            build_resp(
                0x81,
                0x80,
                &q,
                5,
                &[rr_ptr(12, 5, 10, &[0xc0, 0xff])],
                &[],
                &[],
            ),
        ),
        (
            "invalid_rdata",
            build_resp(
                0x81,
                0x80,
                &q,
                1,
                &[rr_ptr(12, 1, 10, &[192, 0, 2])],
                &[],
                &[],
            ),
        ),
    ];

    let mut retention = AuditRetention::default();

    // Independent hardcoded expected-output assertions (not derived from shared helpers).
    let (full_rcode, full_resp) = observe_and_diagnose_wire(&full_wire);
    assert_eq!(full_rcode, 0);
    let full_retained = retention.retain_with_response(
        template_record(true, full_rcode, ResponseDetails::no_response()),
        full_resp,
    );
    let full_json: serde_json::Value =
        serde_json::to_value(project_log(&full_retained.expand())).unwrap();
    assert_eq!(full_json["response_code"], "NOERROR");
    assert_eq!(
        full_json["response_flags"],
        serde_json::json!({"AA": true, "TC": false, "RA": true})
    );
    assert_eq!(full_json["answer_details_status"], "raw_rdata");
    assert!(full_json.get("answer_decode_error").is_none());
    assert_eq!(
        full_json["answers"],
        serde_json::json!([
            {"type": "A", "ttl": 60, "data": "192.0.2.1"},
            {"type": "AAAA", "ttl": 61, "data": "2001:db8::1"},
            {"type": "NS", "ttl": 61, "data": "ns.full.example.org."},
            {"type": "CNAME", "ttl": 62, "data": "a\\.\\\\\\000\\032.full.example.org."},
            {"type": "PTR", "ttl": 62, "data": "."},
            {"type": "MX", "ttl": 63, "data": "mail.full.example.org."},
            {"type": "TXT", "ttl": 64, "data": "txt \u{fffd}2"},
            {"type": "TXT", "ttl": 64, "data": ""},
            {"type": "TXT", "ttl": 64, "data": " "},
            {"type": "TYPE65000", "ttl": 65, "data": "\\# 2 cafe"},
            {"type": "TYPE65001", "ttl": 66, "data": "\\# 0 "},
        ])
    );

    let (trunc_rcode, trunc_resp) = observe_and_diagnose_wire(&truncated_wire);
    let trunc_retained = retention.retain_with_response(
        template_record(true, trunc_rcode, ResponseDetails::no_response()),
        trunc_resp,
    );
    let trunc_json: serde_json::Value =
        serde_json::to_value(project_log(&trunc_retained.expand())).unwrap();
    assert_eq!(trunc_json["response_flags"]["TC"], true);
    assert_eq!(trunc_json["answer_details_status"], "complete");
    let trunc_len = trunc_json["answers"].as_array().unwrap().len();
    assert!(trunc_len > 0 && trunc_len < 64);

    for (expected_err, wire) in &decode_error_wires {
        let (rcode, resp) = observe_and_diagnose_wire(wire);
        let retained = retention.retain_with_response(
            template_record(true, rcode, ResponseDetails::no_response()),
            resp,
        );
        let val: serde_json::Value = serde_json::to_value(project_log(&retained.expand())).unwrap();
        assert_eq!(val["answer_details_status"], "decode_error");
        assert_eq!(val["answer_decode_error"], *expected_err);
        assert_eq!(val["answers"], serde_json::json!([]));
    }

    for wire in std::iter::once(full_wire)
        .chain(std::iter::once(truncated_wire))
        .chain(decode_error_wires.into_iter().map(|(_, w)| w))
    {
        let (ref_rcode, ref_details) = reference_observe_and_diagnose_wire(&wire);
        let (new_rcode, audit_response) = observe_and_diagnose_wire(&wire);
        assert_eq!(new_rcode, ref_rcode);
        let has_wire = !wire.is_empty();
        let retained = retention.retain_with_response(
            template_record(has_wire, new_rcode, ResponseDetails::no_response()),
            audit_response,
        );
        let new_json = serde_json::to_string(&project_log(&retained.expand())).unwrap();
        let ref_json = serde_json::to_string(&project_log(&template_record(
            has_wire,
            ref_rcode,
            ref_details,
        )))
        .unwrap();
        assert_eq!(new_json, ref_json);
    }
}
