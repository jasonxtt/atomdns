use atomdns_dns_core::{QueryHeader, QuestionInfo};
use atomdns_sequence_core::{
    DnsResponseInspector, ExecutionState, OwnedResponseWire, ResponseError, ResponseState,
    RoutingLabelOverlay, RoutingState, SharedStr, SynthesizedResponse,
};

fn state() -> ExecutionState {
    ExecutionState::new(
        QueryHeader {
            id: 0x1234,
            qr: false,
            opcode: 0,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
            rd: false,
            cd: false,
        },
        QuestionInfo {
            qname_wire: vec![7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0],
            qtype: 1,
            qclass: 1,
        },
    )
}

fn complete_response_wire() -> Vec<u8> {
    let mut wire = vec![
        0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01,
    ];
    wire.extend_from_slice(&[7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0, 0, 1, 0, 1]);
    wire.extend_from_slice(&[
        0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 192, 0, 2, 1,
    ]);
    wire.extend_from_slice(&[
        0xc0, 0x0c, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x78, 0x00, 0x02, 0xc0, 0x0c,
    ]);
    wire.extend_from_slice(&[
        0x00, 0x00, 0x29, 0x04, 0xd0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    wire
}

#[test]
fn cached_default_is_preserved_without_a_live_learning_prefix() {
    let mut state = state();
    let replaced_default = SharedStr::from("DEFAULT");
    state.routing.domain_set = Some("NEW_STATIC".into());
    state.domain_set_overlay = Some(RoutingLabelOverlay {
        prefix_len: 0,
        dynamic_label_count: 1,
        replaced: vec![SharedStr::from("LEARNED"), replaced_default.clone()].into(),
    });
    state.restore_cached_domain_set("DEFAULT");
    let restored = state.routing.domain_set.as_ref().expect("restored");
    assert_eq!(restored.as_str(), "DEFAULT");
    assert!(restored.ptr_eq(&replaced_default));
}

#[test]
fn restore_cached_domain_set_preserves_shared_allocation_across_cache_and_learning_paths() {
    // 1. No overlay, identical text keeps the existing SharedStr allocation.
    let mut plain = state();
    let existing = SharedStr::from("中国域名");
    plain.routing.domain_set = Some(existing.clone());
    plain.restore_cached_domain_set("中国域名");
    assert!(
        plain
            .routing
            .domain_set
            .as_ref()
            .expect("domain_set")
            .ptr_eq(&existing)
    );
    assert_eq!(plain.routing.effective_domain_set, None);

    // 2. Overlay with prefix_len == 0, identical text keeps the existing SharedStr.
    let mut with_overlay = state();
    let static_label = SharedStr::from("中国域名");
    let learned_label = SharedStr::from("记忆无V4");
    let default_label = SharedStr::from("未命中");
    with_overlay.routing.domain_set = Some(static_label.clone());
    with_overlay.domain_set_overlay = Some(RoutingLabelOverlay {
        prefix_len: 0,
        dynamic_label_count: 1,
        replaced: vec![learned_label.clone(), default_label.clone()].into(),
    });
    with_overlay.restore_cached_domain_set("中国域名");
    assert!(
        with_overlay
            .routing
            .domain_set
            .as_ref()
            .expect("domain_set")
            .ptr_eq(&static_label)
    );
    assert_eq!(with_overlay.routing.effective_domain_set, None);

    // 3. Live learning prefix superseding cached default keeps prefix SharedStr and
    //    reuses replaced default SharedStr for effective_domain_set.
    let mut learned = state();
    learned.routing.domain_set = Some(learned_label.clone());
    learned.domain_set_overlay = Some(RoutingLabelOverlay {
        prefix_len: learned_label.len(),
        dynamic_label_count: 1,
        replaced: vec![learned_label.clone(), default_label.clone()].into(),
    });
    learned.restore_cached_domain_set("未命中");
    assert!(
        learned
            .routing
            .domain_set
            .as_ref()
            .expect("domain_set")
            .ptr_eq(&learned_label)
    );
    assert!(
        learned
            .routing
            .effective_domain_set
            .as_ref()
            .expect("effective_domain_set")
            .ptr_eq(&default_label)
    );

    // 4. Expired learning-only cached label keeps current mapper default and
    //    records effective_domain_set using overlay.replaced.
    let mut expired = state();
    expired.routing.domain_set = Some(default_label.clone());
    expired.domain_set_overlay = Some(RoutingLabelOverlay {
        prefix_len: 0,
        dynamic_label_count: 1,
        replaced: vec![learned_label.clone(), default_label.clone()].into(),
    });
    expired.restore_cached_domain_set("记忆无V4");
    assert!(
        expired
            .routing
            .domain_set
            .as_ref()
            .expect("domain_set")
            .ptr_eq(&default_label)
    );
    assert!(
        expired
            .routing
            .effective_domain_set
            .as_ref()
            .expect("effective_domain_set")
            .ptr_eq(&learned_label)
    );

    // 5. Live learning prefix combined with non-superseded cached static label.
    let mut combined = state();
    combined.routing.domain_set = Some(learned_label.clone());
    combined.domain_set_overlay = Some(RoutingLabelOverlay {
        prefix_len: learned_label.len(),
        dynamic_label_count: 1,
        replaced: vec![learned_label, default_label].into(),
    });
    combined.restore_cached_domain_set("中国域名");
    assert_eq!(
        combined.routing.domain_set.as_deref(),
        Some("记忆无V4|中国域名")
    );
    assert_eq!(
        combined.routing.effective_domain_set.as_deref(),
        Some("中国域名")
    );
}

#[test]
fn state_snapshot_is_typed_owned_and_deterministic() {
    let mut state = state();
    state.marks.extend([49, 7, 7]);
    state.fast_flags = 1 << 48;
    state.routing = RoutingState {
        domain_set: Some("domain".into()),
        effective_domain_set: None,
        effective_tag: None,
        matched_group: Some("group".into()),
        final_sequence: Some("sequence".into()),
        final_upstream: Some("upstream".into()),
        final_upstream_targets: Some("target-a,target-b".into()),
        selected_upstream: Some("selected".into()),
        matched_rule_source: Some("source".into()),
    };

    let snapshot = state.snapshot();
    assert_eq!(snapshot.query.header.id, 0x1234);
    assert_eq!(
        snapshot.query.question.qname_wire,
        vec![7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0]
    );
    assert_eq!(snapshot.marks, vec![7, 49]);
    assert_eq!(snapshot.fast_flags, 1 << 48);
    assert_eq!(snapshot.routing, state.routing);
    assert_eq!(snapshot.response, ResponseState::None);
}

#[test]
fn response_setters_replace_and_clear_each_response_form() {
    let wire = complete_response_wire();
    let mut state = state();

    state.set_raw_response(wire.clone());
    assert_eq!(
        state.response,
        ResponseState::Raw(OwnedResponseWire(wire.clone()))
    );

    state.set_response(ResponseState::None);
    assert_eq!(state.response, ResponseState::None);

    state
        .set_synthesized_response(15)
        .expect("valid synthesized response");
    assert_eq!(state.response.synthesized_rcode(), Some(15));

    state.set_raw_response(wire.clone());
    assert_eq!(
        state.response,
        ResponseState::Raw(OwnedResponseWire(wire.clone()))
    );

    state.set_response(ResponseState::Synthesized(
        SynthesizedResponse::new(0x0fff).expect("valid configured RCODE"),
    ));
    assert_eq!(state.response.synthesized_rcode(), Some(0x0fff));

    state.set_response(ResponseState::None);
    assert_eq!(state.response, ResponseState::None);
}

#[test]
fn synthesized_rcode_boundaries_are_validated_without_partial_transition() {
    let mut state = state();
    for rcode in [0, 15, 0x0fff] {
        state
            .set_synthesized_response(rcode)
            .expect("configured RCODE is in range");
        assert_eq!(state.response.synthesized_rcode(), Some(rcode));
    }

    let previous = state.response.clone();
    assert_eq!(
        state.set_synthesized_response(0x1000),
        Err(ResponseError::InvalidRcode(0x1000))
    );
    assert_eq!(state.response, previous);
}

#[test]
fn raw_inspection_is_ttl_only_non_consuming_and_preserves_complete_wire() {
    let wire = complete_response_wire();
    let mut state = state();
    state.set_raw_response(wire.clone());

    let inspection = state
        .inspect_response(&DnsResponseInspector)
        .expect("valid response")
        .expect("raw response is inspectable");
    assert_eq!(inspection.ttl.minimal_ttl, 60);
    assert_eq!(inspection.ttl.record_count, 2);
    assert_eq!(
        state.response,
        ResponseState::Raw(OwnedResponseWire(wire.clone()))
    );

    let second = state
        .inspect_response(&DnsResponseInspector)
        .expect("inspection remains repeatable")
        .expect("raw response remains inspectable");
    assert_eq!(second, inspection);
    assert_eq!(state.response, ResponseState::Raw(OwnedResponseWire(wire)));
}

#[test]
fn non_raw_inspection_does_not_change_none_or_synthesized_state() {
    let mut state = state();
    assert_eq!(state.inspect_response(&DnsResponseInspector), Ok(None));
    assert_eq!(state.response, ResponseState::None);

    state
        .set_synthesized_response(5)
        .expect("valid synthesized response");
    assert_eq!(state.inspect_response(&DnsResponseInspector), Ok(None));
    assert_eq!(state.response.synthesized_rcode(), Some(5));
}

#[test]
fn malformed_raw_inspection_returns_typed_error_and_retains_wire_until_clear() {
    let wire = vec![0x80];
    let mut state = state();
    state.set_raw_response(wire.clone());

    let error = state
        .inspect_response(&DnsResponseInspector)
        .expect_err("malformed wire must be observable");
    assert!(matches!(error, ResponseError::MalformedRawResponse { .. }));
    assert_eq!(state.response, ResponseState::Raw(OwnedResponseWire(wire)));

    state.set_response(ResponseState::None);
    assert_eq!(state.response, ResponseState::None);
}
