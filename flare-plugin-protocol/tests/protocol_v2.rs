//! Shared entity state, the host write capability, staff tickets, the plugin
//! runtime credential and client-visible configuration.
use flare_plugin_protocol::{
    PluginManifest,
    configuration::{CLIENT_VISIBLE_KEYWORD, client_visible_configuration, client_visible_keys},
    invocation::*,
    resource::*,
    runtime::{configuration_digest, digest_bytes},
    state::*,
};
use serde_json::{Value, json};

fn state(values: Value) -> SharedState {
    SharedState {
        entity: "poll-1".into(),
        version: 1,
        values: serde_json::from_value(values).unwrap(),
    }
}

#[test]
fn shared_state_is_bounded_and_round_trips_through_event_parts() {
    let poll = state(json!({
        "counts": [3, 0, 2], "total": 5, "voterCount": 4, "closed": false,
        "deadlineMs": 1700000000000_i64, "lines": ["1. Alice", "2. Bob"], "note": null
    }));
    poll.validate().unwrap();
    let parts = SharedState::from_parts(&poll.entity, poll.version, &poll.values_json()).unwrap();
    assert_eq!(parts, poll);
    assert_eq!(
        serde_json::to_value(&poll).unwrap()["values"]["counts"],
        json!([3, 0, 2])
    );
    let invalid = |s: SharedState| s.validate().is_err();
    assert!(invalid(state(json!({"nested": {"a": 1}}))));
    assert!(invalid(state(json!({"mixed": [1, "a"]}))));
    assert!(invalid(state(json!({"bools": [true]}))));
    assert!(invalid(state(json!({"bad-key": 1}))));
    assert!(invalid(state(json!({"k".repeat(65): 1}))));
    assert!(invalid(state(json!({"text": "x".repeat(1025)}))));
    assert!(invalid(state(json!({"text": "line\nbreak"}))));
    assert!(invalid(state(json!({"list": vec![1; 101]}))));
    assert!(invalid(state(json!({"list": ["x".repeat(257)]}))));
    assert!(!invalid(state(
        json!({"list": vec![1; 100], "text": "x".repeat(1024)})
    )));
    let keys = |n: usize| -> Value {
        (0..n)
            .map(|i| (format!("k{i}"), json!(i)))
            .collect::<serde_json::Map<_, _>>()
            .into()
    };
    assert!(!invalid(state(keys(32))));
    assert!(invalid(state(keys(33))));
    // Eight maximal strings fit the per-value bound but exceed 8 KiB in total.
    let big: Value = (0..9)
        .map(|i| (format!("k{i}"), json!("x".repeat(1000))))
        .collect::<serde_json::Map<_, _>>()
        .into();
    assert!(invalid(state(big)));
    for (entity, version) in [
        ("", 1),
        ("has space", 1),
        (&*"e".repeat(129), 1),
        ("poll-1", 0),
        ("poll-1", i64::MAX as u64 + 1),
    ] {
        assert!(invalid(SharedState {
            entity: entity.into(),
            version,
            ..state(json!({}))
        }));
    }
    assert!(valid_entity_id("tenant:poll.1_a-b"));
    // Bounded before parsing.
    assert!(SharedState::from_parts("poll-1", 1, &" ".repeat(8193)).is_err());
    assert!(SharedState::from_parts("poll-1", 1, "[1]").is_err());
    assert!(
        serde_json::from_value::<SharedState>(
            json!({"entity": "e", "version": 1, "values": {}, "x": 1})
        )
        .is_err()
    );
}

#[test]
fn shared_state_from_result_and_version_order() {
    assert_eq!(SharedState::from_result(&json!({})).unwrap(), None);
    assert_eq!(
        SharedState::from_result(&json!({"sharedState": null})).unwrap(),
        None
    );
    let ok = SharedState::from_result(&json!({"message": {}, "sharedState": {
        "entity": "poll-1", "version": 2, "values": {"total": 1}
    }}))
    .unwrap()
    .unwrap();
    assert_eq!(ok.version, 2);
    assert!(
        SharedState::from_result(
            &json!({"sharedState": {"entity": "poll-1", "version": 0, "values": {}}})
        )
        .is_err()
    );
    assert!(SharedState::from_result(&json!({"sharedState": "x"})).is_err());
    assert_eq!(ok.order_against(None), SharedStateOrder::Newer);
    assert_eq!(ok.order_against(Some(1)), SharedStateOrder::Newer);
    assert_eq!(ok.order_against(Some(2)), SharedStateOrder::Same);
    assert_eq!(ok.order_against(Some(3)), SharedStateOrder::Older);
    assert_eq!(
        ok.merge_into(&json!({"total": 0, "question": "q"})),
        json!({"total": 1, "question": "q"})
    );
}

#[test]
fn entity_publication_and_service_dispatch_are_typed_writes() {
    let raw = json!({"capability": "im.plugin.entity_state.publish", "entity": "poll-1", "version": 3, "values": {"total": 2}});
    let request: ResourceWriteRequest = serde_json::from_value(raw.clone()).unwrap();
    request.validate().unwrap();
    assert_eq!(request.capability(), HostCapability::EntityStatePublish);
    assert_eq!(serde_json::to_value(&request).unwrap(), raw);
    let ResourceWriteRequest::EntityStatePublish(published) = &request else {
        panic!("expected an entity state publish");
    };
    assert_eq!(published.values["total"], json!(2));
    for bad in [
        json!({"capability": "im.plugin.entity_state.publish", "entity": "poll-1", "version": 3, "values": {}, "conversationId": "c"}),
        json!({"capability": "im.conversation.read"}),
        json!({"entity": "poll-1", "version": 3, "values": {}}),
    ] {
        assert!(serde_json::from_value::<ResourceWriteRequest>(bad).is_err());
    }
    let stale: ResourceWriteRequest = serde_json::from_value(
        json!({"capability": "im.plugin.entity_state.publish", "entity": "poll-1", "version": 0, "values": {}}),
    )
    .unwrap();
    assert!(stale.validate().is_err());
    // Reads stay reads.
    assert!(serde_json::from_value::<ResourceRequest>(raw).is_err());
    assert!(HostCapability::EntityStatePublish.is_write());
    assert!(HostCapability::ServiceDispatch.is_write());
    assert!(!HostCapability::ConversationRead.is_write());
    assert_eq!(
        HostCapability::EntityStatePublish.id(),
        "im.plugin.entity_state.publish"
    );
    assert_eq!(
        serde_json::to_value(HostCapability::EntityStatePublish).unwrap(),
        json!("im.plugin.entity_state.publish")
    );
    assert_eq!(
        serde_json::to_value(EntityStatePublishView {
            entity: "poll-1".into(),
            version: 3,
            outcome: EntityStatePublishOutcome::Duplicate
        })
        .unwrap(),
        json!({"entity": "poll-1", "version": 3, "outcome": "duplicate"})
    );
    // Declaring it still needs an operation permission with that data scope.
    let mut manifest: Value =
        serde_json::from_slice(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap();
    manifest["permissions"][1]["dataScopes"] = json!(["poll.publish"]);
    assert!(PluginManifest::from_json(&serde_json::to_vec(&manifest).unwrap()).is_err());
}

fn staff_claims() -> PluginStaffCallClaims {
    PluginStaffCallClaims {
        scope: Default::default(),
        tenant_id: "201".into(),
        plugin_id: "flare.poll".into(),
        plugin_version: "1.0.0".into(),
        manifest_digest: format!("sha256:{}", "a".repeat(64)),
        deployment_id: "poll-1".into(),
        grant_id: "grant-1".into(),
        grant_revision: 3,
        configuration_digest: configuration_digest(&json!({})).unwrap(),
        staff_id: "staff-9".into(),
        org_id: Some("org-1".into()),
        permission: "plugin:flare.poll:manage".into(),
        operation: "flare.poll.list".into(),
        request_id: "req-1".into(),
        reason: Some("清理违规投票".into()),
        issued_at_ms: 1000,
        expires_at_ms: 2000,
    }
}

#[test]
fn staff_tickets_never_verify_as_user_tickets_and_bind_the_admin_permission() {
    let signer = PluginCallSigner::new("call-1".into(), [7; 32]).unwrap();
    let mut verifier = PluginCallVerifier::default();
    verifier
        .insert("call-1".into(), signer.public_key())
        .unwrap();
    let claims = staff_claims();
    let token = signer.sign_staff(&claims, 1000).unwrap();
    assert_eq!(verifier.verify_staff(&token, 1000).unwrap(), claims);
    assert!(
        verifier.verify(&token, 1000).is_err(),
        "a staff ticket is not a user ticket"
    );
    assert!(verifier.verify_staff(&token, 2000).is_err());
    let user = PluginCallClaims {
        tenant_id: "201".into(),
        plugin_id: "flare.poll".into(),
        plugin_version: "1.0.0".into(),
        manifest_digest: claims.manifest_digest.clone(),
        deployment_id: "poll-1".into(),
        grant_id: "grant-1".into(),
        grant_revision: 3,
        configuration_digest: claims.configuration_digest.clone(),
        actor_id: "u1".into(),
        operation: "flare.poll.vote".into(),
        request_id: "req-2".into(),
        conversation_id: Some("c-1".into()),
        issued_at_ms: 1000,
        expires_at_ms: 2000,
    };
    let user_token = signer.sign(&user, 1000).unwrap();
    assert!(
        verifier.verify_staff(&user_token, 1000).is_err(),
        "a user ticket is not a staff ticket"
    );
    assert_eq!(verifier.verify(&user_token, 1000).unwrap(), user);
    for bad in [
        PluginStaffCallClaims {
            permission: "plugin:other.plugin:manage".into(),
            ..staff_claims()
        },
        PluginStaffCallClaims {
            reason: Some("  ".into()),
            ..staff_claims()
        },
        PluginStaffCallClaims {
            reason: Some("a\nb".into()),
            ..staff_claims()
        },
        PluginStaffCallClaims {
            reason: Some("r".repeat(MAX_STAFF_REASON_BYTES + 1)),
            ..staff_claims()
        },
        PluginStaffCallClaims {
            permission: "core:plugins:write".into(),
            ..staff_claims()
        },
        PluginStaffCallClaims {
            scope: Default::default(),
            tenant_id: "*".into(),
            ..staff_claims()
        },
        PluginStaffCallClaims {
            scope: Default::default(),
            tenant_id: String::new(),
            ..staff_claims()
        },
        PluginStaffCallClaims {
            org_id: Some(String::new()),
            ..staff_claims()
        },
        PluginStaffCallClaims {
            expires_at_ms: 1001 + MAX_CALL_TTL_MS,
            ..staff_claims()
        },
    ] {
        assert!(signer.sign_staff(&bad, 1000).is_err(), "{bad:?}");
    }
    assert!(
        signer
            .sign_staff(
                &PluginStaffCallClaims {
                    org_id: None,
                    ..staff_claims()
                },
                1000
            )
            .is_ok(),
        "platform staff have no organization"
    );
    // The reason is optional on the wire and round-trips when present.
    for reason in [None, Some("r".repeat(MAX_STAFF_REASON_BYTES))] {
        let claims = PluginStaffCallClaims {
            reason: reason.clone(),
            ..staff_claims()
        };
        let token = signer.sign_staff(&claims, 1000).unwrap();
        assert_eq!(verifier.verify_staff(&token, 1000).unwrap().reason, reason);
    }
    assert!(valid_staff_reason("清理违规投票"));
    for bad in ["", " ", "a\tb", "\u{7f}"] {
        assert!(!valid_staff_reason(bad), "{bad:?}");
    }
}

fn runtime_claims(request: &[u8]) -> PluginRuntimeClaims {
    PluginRuntimeClaims {
        tenant_id: "201".into(),
        plugin_id: "flare.poll".into(),
        plugin_version: "1.0.0".into(),
        manifest_digest: format!("sha256:{}", "a".repeat(64)),
        deployment_id: "poll-1".into(),
        grant_id: "grant-1".into(),
        grant_revision: 3,
        capability: HostCapability::EntityStatePublish,
        request_id: "deadline-poll-7".into(),
        request_digest: digest_bytes(request),
        issued_at_ms: 1000,
        expires_at_ms: 1000 + MAX_RUNTIME_CALL_TTL_MS,
    }
}

#[test]
fn runtime_credential_binds_deployment_capability_and_exact_request() {
    use base64::Engine;
    let request = serde_json::to_vec(&json!({"capability": "im.plugin.entity_state.publish",
        "entity": "poll-7", "version": 9, "values": {"closed": true}}))
    .unwrap();
    let signer = PluginRuntimeSigner::new("poll-runtime-1".into(), [11; 32]).unwrap();
    let keys = json!([{"key_id": "poll-runtime-1", "deployment_id": "poll-1",
        "public_key_base64": base64::engine::general_purpose::STANDARD.encode(signer.public_key())}]);
    let verifier = PluginRuntimeVerifier::from_public_keys_json(&keys.to_string()).unwrap();
    let claims = runtime_claims(&request);
    let token = signer.sign(&claims, 1000).unwrap();
    assert_eq!(verifier.verify(&token, &request, 1000).unwrap(), claims);
    let mut altered = request.clone();
    altered[10] ^= 1;
    assert!(
        verifier.verify(&token, &altered, 1000).is_err(),
        "bound to the exact body"
    );
    assert!(
        verifier
            .verify(&token, &request, 1000 + MAX_RUNTIME_CALL_TTL_MS)
            .is_err()
    );
    // The key verifies only its own deployment.
    let other = signer
        .sign(
            &PluginRuntimeClaims {
                deployment_id: "poll-2".into(),
                ..runtime_claims(&request)
            },
            1000,
        )
        .unwrap();
    assert!(verifier.verify(&other, &request, 1000).is_err());
    // Not a call ticket, and a call ticket is not a runtime credential.
    let mut calls = PluginCallVerifier::default();
    calls
        .insert("poll-runtime-1".into(), signer.public_key())
        .unwrap();
    assert!(calls.verify(&token, 1000).is_err());
    assert!(calls.verify_staff(&token, 1000).is_err());
    let call_signer = PluginCallSigner::new("poll-runtime-1".into(), [11; 32]).unwrap();
    let staff = call_signer.sign_staff(&staff_claims(), 1000).unwrap();
    assert!(verifier.verify(&staff, &request, 1000).is_err());
    for bad in [
        PluginRuntimeClaims {
            capability: HostCapability::ConversationRead,
            ..runtime_claims(&request)
        },
        PluginRuntimeClaims {
            expires_at_ms: 1001 + MAX_RUNTIME_CALL_TTL_MS,
            ..runtime_claims(&request)
        },
        PluginRuntimeClaims {
            request_digest: "sha256:short".into(),
            ..runtime_claims(&request)
        },
        PluginRuntimeClaims {
            tenant_id: "*".into(),
            ..runtime_claims(&request)
        },
    ] {
        assert!(signer.sign(&bad, 1000).is_err(), "{bad:?}");
    }
    // Key sets: runtime keys name their deployment, call keys do not.
    let call_keys = json!([{"key_id": "k", "public_key_base64":
        base64::engine::general_purpose::STANDARD.encode(signer.public_key())}]);
    assert!(PluginRuntimeVerifier::from_public_keys_json(&call_keys.to_string()).is_err());
    assert!(PluginCallVerifier::from_public_keys_json(&call_keys.to_string()).is_ok());
    assert!(PluginCallVerifier::from_public_keys_json(&keys.to_string()).is_err());
    let mut rebind = verifier.clone();
    assert!(
        rebind
            .insert(
                "poll-runtime-1".into(),
                "poll-2".into(),
                signer.public_key()
            )
            .is_err(),
        "a key id cannot move to another deployment"
    );
    assert_eq!(PLUGIN_RUNTIME_HEADER, "x-flare-plugin-runtime");
}

#[test]
fn client_visible_configuration_is_marked_top_level_and_bounded() {
    let manifest =
        PluginManifest::from_json(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap();
    let schema = manifest.configuration_schema.clone().unwrap();
    assert_eq!(
        client_visible_keys(&schema).into_iter().collect::<Vec<_>>(),
        ["allowAnonymous", "maxOptions"]
    );
    let configuration = json!({"maxOptions": 10, "allowAnonymous": false, "webhookSecret": "s"});
    assert_eq!(
        Value::Object(
            manifest
                .client_visible_configuration(&configuration)
                .unwrap()
        ),
        json!({"maxOptions": 10, "allowAnonymous": false})
    );
    assert_eq!(
        Value::Object(
            manifest
                .client_visible_configuration(&json!({"webhookSecret": "s"}))
                .unwrap()
        ),
        json!({})
    );
    assert!(manifest.client_visible_configuration(&json!([])).is_err());
    assert!(
        client_visible_configuration(
            &json!({"type": "object", "properties": {"big": {"type": "string", CLIENT_VISIBLE_KEYWORD: true}}}),
            &json!({"big": "x".repeat(4096)})
        )
        .is_err()
    );
    // The red packet schema marks nothing: clients get no configuration.
    let red_packet =
        PluginManifest::from_json(include_bytes!("fixtures/red-packet-0.4.1.json")).unwrap();
    assert!(
        red_packet
            .client_visible_configuration(&json!({"currency": "CNY", "maxRecipients": 10}))
            .unwrap()
            .is_empty()
    );
    let with_schema = |schema: Value| {
        let mut raw: Value =
            serde_json::from_slice(include_bytes!("fixtures/plugin-ui-v2-manifest.json")).unwrap();
        raw["configurationSchema"] = schema;
        PluginManifest::from_json(&serde_json::to_vec(&raw).unwrap())
    };
    for bad in [
        json!({"type": "object", "properties": {"a": {"type": "string", CLIENT_VISIBLE_KEYWORD: "yes"}}}),
        json!({"type": "object", CLIENT_VISIBLE_KEYWORD: true}),
        json!({"type": "object", "properties": {"a": {"type": "object", "properties": {"b": {"type": "string", CLIENT_VISIBLE_KEYWORD: true}}}}}),
        json!({"type": "object", "$defs": {"x": {"type": "string", CLIENT_VISIBLE_KEYWORD: true}}}),
    ] {
        assert!(with_schema(bad.clone()).is_err(), "{bad}");
    }
    assert!(
        with_schema(json!({"type": "object", "properties": {"a": {"type": "string", CLIENT_VISIBLE_KEYWORD: false}}}))
            .is_ok()
    );
}

#[test]
fn organization_admin_scope_has_one_canonical_wire_shape() {
    use flare_plugin_protocol::invocation::PluginAdminScope;
    let scope = PluginAdminScope::Organization {
        organization_id: "org-1".into(),
    };
    assert_eq!(
        serde_json::to_value(&scope).unwrap(),
        serde_json::json!({"kind":"organization","organizationId":"org-1"})
    );
    assert!(
        serde_json::from_value::<PluginAdminScope>(
            serde_json::json!({"kind":"organization","organization_id":"org-1"})
        )
        .is_err()
    );
    let mut claims = staff_claims();
    claims.scope = scope;
    claims.org_id = Some("other-org".into());
    assert!(claims.validate(claims.issued_at_ms).is_err());
    claims.org_id = None;
    assert!(claims.validate(claims.issued_at_ms).is_ok());
}
