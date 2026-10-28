// Included only inside observer::tests of a disposable diagnostic tree.
// It never runs a server and never changes the product's sharing hasher.
#[test]
fn memory_validation_synthetic_retention() {
    use std::hash::{Hash, Hasher};
    let capacity: usize = std::env::var("RTDNS_MEMORY_AUDIT_CAPACITY")
        .unwrap_or_else(|_| "100000".into())
        .parse()
        .unwrap();
    assert!(matches!(capacity, 0 | 100_000 | 400_000));
    let profile = std::env::var("RTDNS_MEMORY_AUDIT_PROFILE").unwrap_or_else(|_| "shared".into());
    assert!(matches!(profile.as_str(), "shared" | "ttl" | "unique"));
    let observer = observer(true, capacity);
    let mut digest = std::collections::hash_map::DefaultHasher::new();
    for i in 0..capacity + 300 {
        let variant = match profile.as_str() {
            "shared" => 0,
            "ttl" => i % 13,
            _ => i,
        };
        let ttl = if profile == "shared" {
            300
        } else {
            (i % 301) as u32
        };
        let event = observation(Duration::from_micros(((i * 7919) % 10000) as u64));
        let mut record = audit_record(event.clone(), &format!("q{}.memory.example.", i % 9973));
        record.timestamp = SystemTime::UNIX_EPOCH + Duration::from_millis(i as u64);
        record.trace_id = format!("n-11111111111111111111111111111111-{i:016x}");
        record.qtype = if i % 2 == 0 { 1 } else { 28 };
        record.client_addr = format!("192.0.2.{}:{}", 1 + i % 64, 10000 + i % 50000)
            .parse()
            .unwrap();
        record.domain_set = Some(format!("route-{}", i % 7));
        record.matched_rule_source = Some(format!("provider-{}", i % 3));
        record.response_details.flags.ra = true;
        record.response_details.answers = vec![
            super::AuditAnswer {
                rrtype: 1,
                ttl,
                data: format!("192.0.2.{}", 1 + variant % 250),
            },
            super::AuditAnswer {
                rrtype: 5,
                ttl,
                data: format!("target{variant}.memory.example."),
            },
            super::AuditAnswer {
                rrtype: 16,
                ttl,
                data: format!("fixture payload {variant}"),
            },
        ];
        record.hash(&mut digest);
        observer.admit_query();
        observer.record_terminal(event, |_| record);
    }
    let state = observer.lock();
    assert_eq!(state.audit_records.len(), capacity);
    eprintln!(
        "SYNTHETIC_INPUT {{\"capacity\":{capacity},\"profile\":\"{profile}\",\"records\":{},\"input_checksum\":\"{:016x}\",\"sharing_hash\":\"DefaultHasher diagnostic only\"}}",
        capacity + 300,
        digest.finish()
    );
    super::retention::audit_memory_report(&state);
}
