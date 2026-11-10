use std::collections::BTreeSet;

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use flare_plugin_protocol::authorization::{PluginInvocation, TenantPluginGrant};
use flare_plugin_protocol::lifecycle::{PluginLifecycle, RuntimeState};
use flare_plugin_protocol::manifest::{MAX_MANIFEST_BYTES, PluginDependency};
use flare_plugin_protocol::registry::PluginRegistry;
use flare_plugin_protocol::signature::{PublisherTrustStore, VerifiedManifest};
use flare_plugin_protocol::{PluginManifest, ProtocolError};
use serde_json::json;

#[test]
fn runtime_empty_snapshot_is_valid_revocation_with_bounded_lease_and_identity() {
    use flare_plugin_protocol::runtime::*;
    let snapshot = PluginProjection {
        organization_id: Some("org-1".into()),
        tenant_id: "201".into(),
        version: 9,
        issued_at_ms: 1000,
        valid_until_ms: 11000,
        plugins: vec![],
        bindings: Vec::new(),
    };
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    assert_eq!(PluginProjection::from_json(&bytes, 1000).unwrap(), snapshot);
    assert!(PluginProjection::from_json(&bytes, 11000).is_err());
    for bad in [
        PluginProjection {
            organization_id: Some("org-1".into()),
            tenant_id: "*".into(),
            ..snapshot.clone()
        },
        PluginProjection {
            version: 0,
            ..snapshot.clone()
        },
        PluginProjection {
            valid_until_ms: 1000 + MAX_PROJECTION_LEASE_MS + 1,
            ..snapshot.clone()
        },
        PluginProjection {
            issued_at_ms: 32000,
            ..snapshot.clone()
        },
    ] {
        assert!(bad.validate(1000).is_err());
    }
    assert!(PluginProjection::from_json(&vec![b' '; MAX_SNAPSHOT_BYTES + 1], 1000).is_err());
    for url in [
        "http://user@localhost:1",
        "https://host/path",
        "http://host?token=x",
        "http://host\n",
        "file://host",
        "http://",
    ] {
        assert!(!valid_grpc_authority(url));
    }
    assert!(valid_grpc_authority("http://127.0.0.1:7901"));
    assert!(configuration_digest(&json!([])).is_err());
    assert_eq!(
        configuration_digest(
            &serde_json::from_str::<serde_json::Value>(r#"{"b":{"y":1,"x":2},"a":3}"#).unwrap()
        )
        .unwrap(),
        configuration_digest(
            &serde_json::from_str::<serde_json::Value>(r#"{"a":3,"b":{"x":2,"y":1}}"#).unwrap()
        )
        .unwrap()
    );
}

#[test]
fn invocation_ticket_has_independent_signatures_target_fields_and_bounded_lifetime() {
    use flare_plugin_protocol::{invocation::*, runtime::configuration_digest};
    let signer = PluginCallSigner::new("call-1".into(), [27; 32]).unwrap();
    let mut verifier = PluginCallVerifier::default();
    verifier
        .insert("call-1".into(), signer.public_key())
        .unwrap();
    let claims = PluginCallClaims {
        tenant_id: "201".into(),
        plugin_id: "example.notice".into(),
        plugin_version: "1.0.0".into(),
        manifest_digest: format!("sha256:{}", "a".repeat(64)),
        deployment_id: "notice-1".into(),
        grant_id: "grant-1".into(),
        grant_revision: 9,
        configuration_digest: configuration_digest(&json!({})).unwrap(),
        actor_id: "user-1".into(),
        operation: "example.notice.run".into(),
        request_id: "req-1".into(),
        conversation_id: Some("conversation-1".into()),
        issued_at_ms: 1000,
        expires_at_ms: 2000,
    };
    let token = signer.sign(&claims, 1000).unwrap();
    assert_eq!(verifier.verify(&token, 1000).unwrap(), claims);
    assert!(verifier.verify(&token, 2000).is_err());
    let mut attacker = PluginCallVerifier::default();
    attacker
        .insert(
            "call-1".into(),
            PluginCallSigner::new("call-1".into(), [28; 32])
                .unwrap()
                .public_key(),
        )
        .unwrap();
    assert!(attacker.verify(&token, 1000).is_err());
    let mut changed = token.into_bytes();
    changed[20] = if changed[20] == b'A' { b'B' } else { b'A' };
    assert!(
        verifier
            .verify(std::str::from_utf8(&changed).unwrap(), 1000)
            .is_err()
    );
    assert!(
        signer
            .sign(
                &PluginCallClaims {
                    expires_at_ms: 1001 + MAX_CALL_TTL_MS,
                    ..claims.clone()
                },
                1000
            )
            .is_err()
    );
    assert!(
        signer
            .sign(
                &PluginCallClaims {
                    tenant_id: "*".into(),
                    ..claims
                },
                1000
            )
            .is_err()
    );
}

fn manifest(id: &str, version: &str) -> PluginManifest {
    PluginManifest::from_json(&serde_json::to_vec(&json!({
        "manifestVersion": 1, "pluginId": id, "version": version, "apiVersion": "1.0",
        "displayName": "Test plugin", "publisher": { "publisherId": "test", "displayName": "Test" },
        "runtime": { "kind": "client" },
        "capabilities": [{ "id": id, "operations": [{
            "id": "run", "inputSchema": {"type": "object"}, "outputSchema": {"type": "object"},
            "permissionIds": ["test.run"], "idempotent": true, "timeoutMs": 1000
        }] }],
        "permissions": [{ "id": "test.run", "description": "Execute the test operation", "dataScopes": ["test.data"], "riskLevel": "low" }],
        "features": [{ "id": "run", "label": { "en-US": "Run" }, "gates": { "operations": [format!("{id}.run")] } }]
    })).unwrap()).unwrap()
}

#[test]
fn optional_service_dependencies_still_require_a_declared_compatible_target() {
    let mut source = manifest("source", "1.0.0");
    source.dependencies.push(PluginDependency {
        plugin_id: "target".into(),
        version_range: ">=2.0.0, <3.0.0".into(),
        optional: true,
    });
    assert!(source.requires("target", "2.0.0"));
    assert!(!source.requires("target", "1.9.9"));
    assert!(!source.requires("target", "3.0.0"));
    assert!(!source.requires("undeclared", "2.0.0"));
    assert!(!source.requires("target", "invalid"));
    source.dependencies[0].optional = false;
    assert!(source.requires("target", "2.0.0"));
}

fn verified(manifest: &PluginManifest) -> VerifiedManifest {
    let key = SigningKey::from_bytes(&[7; 32]);
    let mut trust = PublisherTrustStore::default();
    trust
        .insert(
            "test-key".into(),
            manifest.publisher.publisher_id.clone(),
            key.verifying_key().to_bytes(),
        )
        .unwrap();
    let bytes = serde_json::to_vec(manifest).unwrap();
    trust
        .verify(
            &bytes,
            "test-key",
            &STANDARD.encode(key.sign(&bytes).to_bytes()),
        )
        .unwrap()
}

fn dep(id: &str, range: &str) -> PluginDependency {
    PluginDependency {
        plugin_id: id.into(),
        version_range: range.into(),
        optional: false,
    }
}

#[test]
fn manifest_rejects_undeclared_permissions_duplicate_operations_and_unsupported_api() {
    let original = manifest("test.plugin", "1.0.0");
    let mut too_long = original.clone();
    too_long.version = format!("1.0.0+{}", "a".repeat(64));
    assert!(too_long.validate().is_err());
    let mut too_many = original.clone();
    let mut extra = original.capabilities[0].clone();
    extra.id = "test.another".into();
    extra.operations = (0..256)
        .map(|index| {
            let mut operation = original.capabilities[0].operations[0].clone();
            operation.id = format!("run{index}");
            operation
        })
        .collect();
    too_many.capabilities.push(extra);
    assert!(too_many.validate().is_err());
    let mut permission = original.clone();
    permission.capabilities[0].operations[0]
        .permission_ids
        .push("test.admin".into());
    assert!(permission.validate().is_err());
    let mut duplicate = original.clone();
    duplicate.capabilities[0]
        .operations
        .push(original.capabilities[0].operations[0].clone());
    assert!(duplicate.validate().is_err());
    let mut api = original;
    api.api_version = "2.0".into();
    assert!(api.validate().is_err());
    assert_eq!(
        PluginManifest::from_json(&vec![b' '; MAX_MANIFEST_BYTES + 1]),
        Err(ProtocolError::ManifestTooLarge(MAX_MANIFEST_BYTES))
    );
}

#[test]
fn manifest_rejects_unknown_commercial_fields_and_unbounded_wasm_access() {
    let mut value = serde_json::to_value(manifest("test.plugin", "1.0.0")).unwrap();
    value["subscription"] = json!({"required": true});
    assert_eq!(
        PluginManifest::from_json(&serde_json::to_vec(&value).unwrap()),
        Err(ProtocolError::InvalidJson)
    );
    value.as_object_mut().unwrap().remove("subscription");
    value["runtime"] = json!({"kind": "wasm", "module": "../outside.wasm", "abiVersion": "1.0", "maxMemoryPages": 128, "maxFuel": 1000});
    value["artifact"] =
        json!({"digest": format!("sha256:{}", "a".repeat(64)), "signatureRef": "test.sig"});
    assert!(PluginManifest::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    value["runtime"]["module"] = json!("modules/plugin.wasm");
    assert!(PluginManifest::from_json(&serde_json::to_vec(&value).unwrap()).is_ok());
    value["runtime"]["maxFuel"] = json!(0);
    assert!(PluginManifest::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn plugin_schema_cannot_trigger_external_reference_fetches() {
    let mut manifest = manifest("test.plugin", "1.0.0");
    manifest.capabilities[0].operations[0].input_schema =
        json!({"$ref": "https://untrusted.example/schema.json"});
    assert!(manifest.validate().is_err());
    manifest.capabilities[0].operations[0].input_schema =
        json!({"$ref": "#/$defs/input", "$defs": {"input": {"type": "object"}}});
    assert!(manifest.validate().is_ok());
}

#[test]
fn publisher_signature_binds_bytes_key_and_publisher_and_supports_revocation() {
    let key = SigningKey::from_bytes(&[8; 32]);
    let mut trust = PublisherTrustStore::default();
    trust
        .insert(
            "publisher-key".into(),
            "test".into(),
            key.verifying_key().to_bytes(),
        )
        .unwrap();
    let mut bytes = serde_json::to_vec(&manifest("test.plugin", "1.0.0")).unwrap();
    let signature = STANDARD.encode(key.sign(&bytes).to_bytes());
    assert!(trust.verify(&bytes, "publisher-key", &signature).is_ok());
    bytes.push(b' ');
    assert!(matches!(
        trust.verify(&bytes, "publisher-key", &signature),
        Err(ProtocolError::InvalidSignature)
    ));
    bytes.pop();
    let mut other = PublisherTrustStore::default();
    other
        .insert(
            "publisher-key".into(),
            "other".into(),
            key.verifying_key().to_bytes(),
        )
        .unwrap();
    assert!(matches!(
        other.verify(&bytes, "publisher-key", &signature),
        Err(ProtocolError::InvalidSignature)
    ));
    trust.remove("publisher-key");
    assert!(matches!(
        trust.verify(&bytes, "publisher-key", &signature),
        Err(ProtocolError::InvalidSignature)
    ));
}

#[test]
fn published_versions_are_immutable_and_namespaces_cannot_be_taken_over() {
    let original = verified(&manifest("test.one", "1.0.0"));
    let mut registry = PluginRegistry::default();
    registry.publish(original.clone()).unwrap();
    registry.publish(original).unwrap();
    let mut changed = manifest("test.one", "1.0.0");
    changed.display_name = "Changed after publication".into();
    assert!(matches!(
        registry.publish(verified(&changed)),
        Err(ProtocolError::VersionConflict(_))
    ));
    let mut intruder = manifest("test.two", "1.0.0");
    intruder.capabilities[0].id = "test.one.child".into();
    intruder.features[0].gates.operations = vec!["test.one.child.run".into()];
    assert!(matches!(
        registry.publish(verified(&intruder)),
        Err(ProtocolError::NamespaceConflict(_))
    ));
    let mut takeover = manifest("test.one", "2.0.0");
    takeover.publisher.publisher_id = "another-publisher".into();
    assert!(matches!(
        registry.publish(verified(&takeover)),
        Err(ProtocolError::PublisherConflict(_))
    ));
}

#[test]
fn dependency_resolution_backtracks_and_returns_dependencies_before_dependents() {
    let mut registry = PluginRegistry::default();
    for version in ["1.0.0", "2.0.0"] {
        registry
            .publish(verified(&manifest("test.shared", version)))
            .unwrap();
    }
    let mut one = manifest("test.a", "1.0.0");
    one.dependencies = vec![dep("test.shared", "^1")];
    registry.publish(verified(&one)).unwrap();
    one.version = "2.0.0".into();
    one.dependencies = vec![dep("test.shared", "^2")];
    registry.publish(verified(&one)).unwrap();
    let mut two = manifest("test.b", "1.0.0");
    two.dependencies = vec![dep("test.shared", "^1")];
    registry.publish(verified(&two)).unwrap();
    let mut root = manifest("test.root", "1.0.0");
    root.dependencies = vec![dep("test.a", ">=1, <3"), dep("test.b", "^1")];
    registry.publish(verified(&root)).unwrap();
    let resolved = registry.resolve("test.root", "^1", "1.0.0").unwrap();
    assert_eq!(
        resolved
            .iter()
            .map(|item| (
                item.manifest().plugin_id.as_str(),
                item.manifest().version.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("test.shared", "1.0.0"),
            ("test.a", "1.0.0"),
            ("test.b", "1.0.0"),
            ("test.root", "1.0.0")
        ]
    );
}

#[test]
fn dependency_cycles_and_incompatible_host_are_rejected() {
    let mut registry = PluginRegistry::default();
    let mut one = manifest("test.a", "1.0.0");
    one.dependencies = vec![dep("test.b", "^1")];
    let mut two = manifest("test.b", "1.0.0");
    two.dependencies = vec![dep("test.a", "^1")];
    registry.publish(verified(&one)).unwrap();
    registry.publish(verified(&two)).unwrap();
    assert!(matches!(
        registry.resolve("test.a", "^1", "1.0.0"),
        Err(ProtocolError::DependencyCycle(_))
    ));
    let mut host = manifest("test.host", "1.0.0");
    host.host_version_range = Some("^2".into());
    registry.publish(verified(&host)).unwrap();
    assert!(registry.resolve("test.host", "^1", "1.0.0").is_err());
}

#[test]
fn cyclic_newest_version_does_not_hide_an_acyclic_solution() {
    let mut registry = PluginRegistry::default();
    registry
        .publish(verified(&manifest("test.a", "1.0.0")))
        .unwrap();
    let mut newest = manifest("test.a", "2.0.0");
    newest.dependencies = vec![dep("test.b", "^1")];
    registry.publish(verified(&newest)).unwrap();
    let mut b = manifest("test.b", "1.0.0");
    b.dependencies = vec![dep("test.a", ">=1, <3")];
    registry.publish(verified(&b)).unwrap();
    let resolved = registry.resolve("test.a", ">=1, <3", "1.0.0").unwrap();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].manifest().version, "1.0.0");
}

#[test]
fn authorization_requires_matching_tenant_version_permission_scope_and_fresh_approval() {
    let manifest = verified(&manifest("test.plugin", "1.0.0"));
    let grant = TenantPluginGrant {
        grant_id: "grant-1".into(),
        revision: 1,
        tenant_id: "tenant-a".into(),
        plugin_id: "test.plugin".into(),
        plugin_version: "1.0.0".into(),
        manifest_digest: manifest.digest().into(),
        approved_operations: BTreeSet::from(["test.plugin.run".into()]),
        approved_permissions: BTreeSet::from(["test.run".into()]),
        approved_data_scopes: BTreeSet::from(["test.data".into()]),
        enabled: true,
        revoked: false,
        suspended: false,
        valid_from_ms: 1,
        valid_until_ms: 100,
    };
    let permissions = BTreeSet::from(["test.run".into()]);
    let invocation = PluginInvocation {
        tenant_id: "tenant-a",
        actor_id: "user-1",
        operation: "test.plugin.run",
        actor_permissions: &permissions,
        platform_enabled: true,
        now_ms: 50,
    };
    grant.authorize(&manifest, &invocation).unwrap();
    assert!(
        grant
            .authorize(
                &manifest,
                &PluginInvocation {
                    tenant_id: "tenant-b",
                    ..invocation
                }
            )
            .is_err()
    );
    assert!(
        grant
            .authorize(
                &manifest,
                &PluginInvocation {
                    platform_enabled: false,
                    ..invocation
                }
            )
            .is_err()
    );
    assert!(
        grant
            .authorize(
                &manifest,
                &PluginInvocation {
                    now_ms: 100,
                    ..invocation
                }
            )
            .is_err()
    );
    let mut changed = grant.clone();
    changed.enabled = false;
    assert!(changed.authorize(&manifest, &invocation).is_err());
    changed = grant.clone();
    changed.manifest_digest = "changed".into();
    assert!(changed.authorize(&manifest, &invocation).is_err());
    changed = grant.clone();
    changed.approved_data_scopes.clear();
    assert!(changed.authorize(&manifest, &invocation).is_err());
    changed = grant.clone();
    changed.suspended = true;
    assert!(changed.authorize(&manifest, &invocation).is_err());
    assert!(
        grant
            .authorize(
                &manifest,
                &PluginInvocation {
                    actor_permissions: &BTreeSet::new(),
                    ..invocation
                }
            )
            .is_err()
    );
}

/// The platform stamps `validFromMs` with its database clock; a host clock 2.85 s behind (seen on
/// the full stack) must not close a plugin whose projection just changed. Expiry, revocation and a
/// start beyond the bound stay refused.
#[test]
fn authorization_tolerates_bounded_issuer_clock_skew_on_start_only() {
    use flare_plugin_protocol::authorization::CLOCK_SKEW_MARGIN_MS;
    let manifest = verified(&manifest("test.plugin", "1.0.0"));
    let host_now = 1_000_000;
    let issuer_now = host_now + 2_850;
    let grant = TenantPluginGrant {
        grant_id: "grant-1".into(),
        revision: 2,
        tenant_id: "tenant-a".into(),
        plugin_id: "test.plugin".into(),
        plugin_version: "1.0.0".into(),
        manifest_digest: manifest.digest().into(),
        approved_operations: BTreeSet::from(["test.plugin.run".into()]),
        approved_permissions: BTreeSet::from(["test.run".into()]),
        approved_data_scopes: BTreeSet::from(["test.data".into()]),
        enabled: true,
        revoked: false,
        suspended: false,
        valid_from_ms: issuer_now,
        valid_until_ms: issuer_now + 60_000,
    };
    let permissions = BTreeSet::from(["test.run".into()]);
    let invocation = PluginInvocation {
        tenant_id: "tenant-a",
        actor_id: "user-1",
        operation: "test.plugin.run",
        actor_permissions: &permissions,
        platform_enabled: true,
        now_ms: host_now,
    };
    assert!(grant.in_effect(host_now));
    grant.authorize(&manifest, &invocation).unwrap();

    let mut far = grant.clone();
    far.valid_from_ms = host_now + CLOCK_SKEW_MARGIN_MS + 1;
    assert!(!far.in_effect(host_now));
    assert!(far.authorize(&manifest, &invocation).is_err());
    far.valid_from_ms = host_now + CLOCK_SKEW_MARGIN_MS;
    assert!(far.in_effect(host_now));

    let mut expired = grant.clone();
    expired.valid_until_ms = host_now;
    assert!(!expired.in_effect(host_now));
    assert!(expired.authorize(&manifest, &invocation).is_err());

    let mut revoked = grant.clone();
    revoked.revoked = true;
    assert!(revoked.authorize(&manifest, &invocation).is_err());
    let mut disabled = grant.clone();
    disabled.enabled = false;
    assert!(disabled.authorize(&manifest, &invocation).is_err());
}

#[test]
fn lifecycle_requires_validation_and_fences_stale_workers() {
    let mut lifecycle = PluginLifecycle::default();
    assert_eq!(
        lifecycle.transition(0, RuntimeState::Running),
        Err(ProtocolError::InvalidTransition)
    );
    lifecycle.transition(0, RuntimeState::Validated).unwrap();
    assert_eq!(
        lifecycle.transition(0, RuntimeState::Starting),
        Err(ProtocolError::StaleRevision)
    );
    lifecycle.transition(1, RuntimeState::Starting).unwrap();
    lifecycle.transition(2, RuntimeState::Running).unwrap();
    lifecycle.transition(3, RuntimeState::Draining).unwrap();
    lifecycle.transition(4, RuntimeState::Stopped).unwrap();
    assert_eq!(lifecycle.state(), RuntimeState::Stopped);
}
