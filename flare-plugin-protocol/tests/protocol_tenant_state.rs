//! The tenant state ticket and the binding list of a projection.
use flare_plugin_protocol::invocation::{
    PluginCallSigner, PluginCallVerifier, PluginTenantStateClaims,
};

const NOW: i64 = 1_800_000_000_000;

fn claims() -> PluginTenantStateClaims {
    PluginTenantStateClaims {
        tenant_id: "201".into(),
        organization_id: "101".into(),
        deployment_id: "poll-1".into(),
        manifest_digest: format!("sha256:{}", "a".repeat(64)),
        grant_id: "b1".into(),
        revision: 3,
        configuration_digest: format!("sha256:{}", "b".repeat(64)),
        enabled: true,
        issued_at_ms: NOW,
        expires_at_ms: NOW + 30_000,
    }
}

#[test]
fn a_tenant_state_ticket_verifies_only_as_itself() {
    let signer = PluginCallSigner::new("core-1".into(), [7; 32]).unwrap();
    let mut verifier = PluginCallVerifier::default();
    verifier
        .insert("core-1".into(), signer.public_key())
        .unwrap();
    let token = signer.sign_tenant_state(&claims(), NOW).unwrap();
    assert_eq!(verifier.verify_tenant_state(&token, NOW).unwrap(), claims());
    assert!(verifier.verify_system(&token, NOW).is_err());
    assert!(verifier.verify(&token, NOW).is_err());
    assert!(verifier.verify_tenant_state(&token, NOW + 60_000).is_err());
}

#[test]
fn a_ticket_for_any_tenant_or_an_endless_one_is_not_signed() {
    let signer = PluginCallSigner::new("core-1".into(), [7; 32]).unwrap();
    let mut any = claims();
    any.tenant_id = "*".into();
    assert!(signer.sign_tenant_state(&any, NOW).is_err());
    let mut long = claims();
    long.expires_at_ms = NOW + 3_600_000;
    assert!(signer.sign_tenant_state(&long, NOW).is_err());
}

#[test]
fn secrets_are_what_the_schema_or_the_hints_mark() {
    use flare_plugin_protocol::admin::AdminUiHint;
    use flare_plugin_protocol::configuration::secret_paths;
    use std::collections::BTreeMap;
    let schema = serde_json::json!({"type": "object", "properties": {
        "token": {"type": "string", "writeOnly": true},
        "limit": {"type": "integer"},
        "accounts": {"type": "array", "items": {"type": "object", "properties": {
            "key": {"type": "string", "writeOnly": true},
            "name": {"type": "string"},
        }}},
    }});
    let hints: BTreeMap<String, AdminUiHint> = serde_json::from_value(serde_json::json!({
        "webhook": {"widget": "secret"},
        "limit": {"widget": "number"},
    }))
    .unwrap();
    let paths = secret_paths(Some(&schema), Some(&hints));
    let path = |parts: &[&str]| parts.iter().map(|p| p.to_string()).collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            path(&["accounts", "key"]),
            path(&["token"]),
            path(&["webhook"])
        ]
    );
}
