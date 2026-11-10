use flare_plugin_protocol::{
    event::*,
    invocation::{PluginCallSigner, PluginCallVerifier},
};

fn event() -> PluginMessageEvent {
    PluginMessageEvent {
        id: "message:app:msg-1".into(),
        event_type: MESSAGE_CREATED.into(),
        schema_version: 1,
        app: "app".into(),
        time: 1000,
        data: MessageCreatedData {
            server_message_id: "msg-1".into(),
            conversation_id: "group-1".into(),
            actor_id: "member-1".into(),
            conversation_type: "group".into(),
            text: Some("签到".into()),
        },
    }
}
#[test]
fn event_tickets_bind_payload_audience_grant_and_never_authenticate_a_user() {
    let signer = PluginCallSigner::new("core".into(), [9; 32]).unwrap();
    let mut verifier = PluginCallVerifier::default();
    verifier.insert("core".into(), signer.public_key()).unwrap();
    let event = event();
    let bytes = serde_json::to_vec(&event).unwrap();
    let claims = PluginEventClaims {
        tenant_id: "app".into(),
        plugin_id: "checkin".into(),
        plugin_version: "1.0.0".into(),
        manifest_digest: format!("sha256:{}", "a".repeat(64)),
        deployment_id: "deploy".into(),
        grant_id: "grant".into(),
        grant_revision: 1,
        configuration_digest: format!("sha256:{}", "b".repeat(64)),
        enabled_features: ["observe".into()].into(),
        active_since_ms: 1000,
        operation: "checkin.onMessage".into(),
        event_id: event.id.clone(),
        request_digest: flare_plugin_protocol::runtime::digest_bytes(&bytes),
        actor_id: "member-1".into(),
        conversation_id: "group-1".into(),
        issued_at_ms: 2000,
        expires_at_ms: 3000,
    };
    let token = signer.sign_event(&claims, 2000).unwrap();
    assert_eq!(verifier.verify_event(&token, &bytes, 2000).unwrap(), claims);
    assert!(verifier.verify(&token, 2000).is_err());
    assert!(verifier.verify_system(&token, 2000).is_err());
    assert!(verifier.verify_service(&token, 2000).is_err());
    assert!(verifier.verify_event(&token, &bytes, 3000).is_err());
    let mut before_activation = claims.clone();
    before_activation.active_since_ms = 1001;
    let early = signer.sign_event(&before_activation, 2000).unwrap();
    assert!(verifier.verify_event(&early, &bytes, 2000).is_err());
    let user = flare_plugin_protocol::invocation::PluginCallClaims {
        tenant_id: claims.tenant_id.clone(),
        plugin_id: claims.plugin_id.clone(),
        plugin_version: claims.plugin_version.clone(),
        manifest_digest: claims.manifest_digest.clone(),
        deployment_id: claims.deployment_id.clone(),
        grant_id: claims.grant_id.clone(),
        grant_revision: claims.grant_revision,
        configuration_digest: claims.configuration_digest.clone(),
        actor_id: claims.actor_id.clone(),
        operation: claims.operation.clone(),
        request_id: claims.event_id.clone(),
        conversation_id: Some(claims.conversation_id.clone()),
        issued_at_ms: 2000,
        expires_at_ms: 3000,
    };
    assert!(
        verifier
            .verify_event(&signer.sign(&user, 2000).unwrap(), &bytes, 2000)
            .is_err()
    );
    let mut changed = event.clone();
    changed.data.text = Some("other".into());
    assert!(
        verifier
            .verify_event(&token, &serde_json::to_vec(&changed).unwrap(), 2000)
            .is_err()
    );
    let mut changed = event;
    changed.app = "other-app".into();
    assert!(
        verifier
            .verify_event(&token, &serde_json::to_vec(&changed).unwrap(), 2000)
            .is_err()
    );
}

#[test]
fn subscriptions_require_explicit_body_scope_and_dedicated_idempotent_callbacks() {
    use serde_json::json;
    let mut value = json!({"manifestVersion":1,"pluginId":"observer","version":"1.0.0","apiVersion":"1.0","displayName":"Observer",
        "publisher":{"publisherId":"test","displayName":"Test"},"runtime":{"kind":"grpc","service":"test.Plugin","protocol":"flare.capability.v1"},
        "capabilities":[{"id":"observer","operations":[{"id":"receive","inputSchema":{"type":"object"},"outputSchema":{"type":"object"},"permissionIds":["observe"],"idempotent":true,"timeoutMs":1000}]}],
        "permissions":[{"id":"observe","description":"Observe metadata","dataScopes":[MESSAGE_EVENTS_SCOPE],"riskLevel":"high"}],
        "eventSubscriptions":[{"eventType":MESSAGE_CREATED,"schemaVersion":1,"operation":"observer.receive","permissionIds":["observe"]}],
        "artifact":{"digest":format!("sha256:{}","a".repeat(64)),"signatureRef":"test"}});
    let parse = |value: &serde_json::Value| {
        flare_plugin_protocol::PluginManifest::from_json(&serde_json::to_vec(value).unwrap())
    };
    let manifest = parse(&value).unwrap();
    assert!(!manifest.event_subscriptions[0].include_body);
    assert!(!manifest.user_operation_ids().contains("observer.receive"));
    value["eventSubscriptions"][0]["includeBody"] = json!(true);
    assert!(parse(&value).is_err());
    value["permissions"][0]["dataScopes"] = json!([MESSAGE_EVENTS_SCOPE, MESSAGE_BODY_SCOPE]);
    assert!(parse(&value).is_ok());
    value["capabilities"][0]["operations"][0]["idempotent"] = json!(false);
    assert!(parse(&value).is_err());
}
