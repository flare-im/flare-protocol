//! Feature declarations, the derived plugin kind, closed sets, preview samples,
//! the asset table and the uninstall check.
use std::collections::{BTreeMap, BTreeSet};

use flare_plugin_protocol::authorization::TenantPluginGrant;
use flare_plugin_protocol::manifest::{
    MAX_MANIFEST_BYTES, MAX_UI_PREVIEW_BYTES, PluginKind, RiskLevel,
};
use flare_plugin_protocol::package::MAX_PACKAGE_BYTES;
use flare_plugin_protocol::runtime::{ProjectedLifecycle, ProjectedPlugin};
use flare_plugin_protocol::{PluginManifest, ProtocolError};
use serde_json::{Value, json};

fn operation(id: &str) -> Value {
    json!({
        "id": id, "inputSchema": {"type": "object"}, "outputSchema": {"type": "object"},
        "permissionIds": ["example.pay.use"], "idempotent": true, "timeoutMs": 1000
    })
}

/// Two user operations, a `user` and a `recovery` service operation, a query
/// and a command for staff, a page and three configuration keys.
fn manifest() -> Value {
    let mut audit = operation("audit");
    audit["mode"] = json!("query");
    json!({
        "manifestVersion": 1, "pluginId": "example.pay", "version": "1.0.0", "apiVersion": "1.0",
        "displayName": "Pay", "publisher": {"publisherId": "example", "displayName": "Example"},
        "runtime": {"kind": "client"},
        "dependencies": [{"pluginId": "example.bank", "versionRange": "^1"}],
        "capabilities": [{"id": "example.pay", "operations": [
            operation("send"), operation("stats"), operation("debit"), operation("refund"),
            audit, operation("close"), operation("unbound")
        ]}],
        "permissions": [{"id": "example.pay.use", "description": "use", "dataScopes": [], "riskLevel": "low"}],
        "serviceOperations": [
            {"operation": "example.pay.debit", "mode": "user"},
            {"operation": "example.pay.refund", "mode": "recovery"}
        ],
        "configurationSchema": {"type": "object", "properties": {
            "videoEnabled": {"type": "boolean", "x-flare-client-visible": true},
            "internal": {"type": "boolean"},
            "limit": {"type": "integer", "x-flare-client-visible": true}
        }},
        "clientExtensions": {"platforms": ["web"], "contributions": [{
            "kind": "page", "id": "example.pay.home", "schemaVersion": 1,
            "requiredPermissions": [], "schema": {"title": "Pay"}
        }]},
        "features": [
            {
                "id": "send",
                "label": {"zh-CN": "付款", "en-US": "Send"},
                "description": {"en-US": "Closing it keeps sent items"},
                "risk": "medium",
                "defaultEnabled": true,
                "gates": {
                    "operations": ["example.pay.send"],
                    "serviceOperations": ["example.pay.debit"],
                    "contributions": ["example.pay.home"]
                },
                "requires": ["example.pay.stats", "example.bank.accounts"]
            },
            {
                "id": "stats",
                "label": {"en-US": "Stats"},
                "gates": {"operations": ["example.pay.stats", "example.pay.send"]}
            },
            {
                "id": "video",
                "label": {"en-US": "Video"},
                "gates": {"configuration": "videoEnabled"}
            }
        ],
        "admin": {
            "contractVersion": 1,
            "permissions": [{"id": "read", "label": {"en-US": "Read"}, "risk": "low"}],
            "operations": [
                {"id": "example.pay.audit", "mode": "query", "permission": "read"},
                {"id": "example.pay.close", "mode": "command", "permission": "read"}
            ],
            "lifecycle": {"uninstall": {"behavior": "retain", "check": "example.pay.audit"}}
        }
    })
}

fn parse(value: &Value) -> Result<PluginManifest, ProtocolError> {
    PluginManifest::from_json(&serde_json::to_vec(value).unwrap())
}

/// The field a manifest is refused over.
fn refused(value: &Value) -> String {
    match parse(value) {
        Err(ProtocolError::InvalidManifest { field, .. }) => field,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn with_feature(edit: impl FnOnce(&mut Value)) -> Value {
    let mut value = manifest();
    edit(&mut value["features"][0]);
    value
}

fn with_card(value: &mut Value) {
    value["clientExtensions"]["contributions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind": "message-content", "id": "example.pay.card", "schemaVersion": 1,
            "requiredPermissions": [], "entityPath": "/id",
            "entityLifecycle": {"unbound": "example.pay.unbound"},
            "schema": {"title": "Card"}
        }));
}

#[test]
fn a_feature_declares_its_gates_and_defaults_the_rest() {
    let manifest = parse(&manifest()).unwrap();
    let send = &manifest.features[0];
    assert_eq!(send.risk, RiskLevel::Medium);
    assert!(send.default_enabled);
    assert_eq!(send.gates.service_operations, ["example.pay.debit"]);
    assert_eq!(send.label["zh-CN"], "付款");
    let video = &manifest.features[2];
    assert_eq!(video.risk, RiskLevel::Low);
    assert!(!video.default_enabled && video.description.is_empty());
    assert_eq!(video.gates.configuration.as_deref(), Some("videoEnabled"));
    let raw = serde_json::to_value(&manifest).unwrap();
    assert_eq!(
        raw["features"][2]["gates"],
        json!({"configuration": "videoEnabled"})
    );
    assert_eq!(parse(&raw).unwrap(), manifest);
}

#[test]
fn the_manifest_limit_is_one_mebibyte_and_hooks_are_gone() {
    assert_eq!(MAX_MANIFEST_BYTES, 1024 * 1024);
    let mut value = manifest();
    value["hooks"] = json!([]);
    assert_eq!(parse(&value), Err(ProtocolError::InvalidJson));
}

#[test]
fn a_plugin_with_user_operations_declares_a_feature() {
    let mut value = manifest();
    value["features"] = json!([]);
    assert_eq!(refused(&value), "features");
    // Service and admin operations alone need no feature.
    value["capabilities"][0]["operations"]
        .as_array_mut()
        .unwrap()
        .retain(|op| ["debit", "refund", "audit", "close"].contains(&op["id"].as_str().unwrap()));
    assert!(parse(&value).is_ok());
}

#[test]
fn feature_ids_are_unique_identifiers_with_a_label_and_a_gate() {
    let mut duplicate = manifest();
    duplicate["features"][1]["id"] = json!("send");
    assert_eq!(refused(&duplicate), "features");
    assert_eq!(
        refused(&with_feature(|f| f["id"] = json!("Send"))),
        "features.id"
    );
    assert_eq!(
        refused(&with_feature(|f| f["id"] = json!("a".repeat(65)))),
        "features.id"
    );
    assert_eq!(
        refused(&with_feature(|f| f["label"] = json!({}))),
        "features.label"
    );
    assert_eq!(
        refused(&with_feature(
            |f| f["label"] = json!({"en-US": "x".repeat(65)})
        )),
        "features.label"
    );
    assert_eq!(
        refused(&with_feature(|f| f["description"] = json!({"en US": "x"}))),
        "features.description"
    );
    assert_eq!(
        refused(&with_feature(|f| f["gates"] = json!({}))),
        "features.gates"
    );
    assert_eq!(
        parse(&with_feature(|f| f["gates"]["unknown"] = json!([]))),
        Err(ProtocolError::InvalidJson)
    );
}

#[test]
fn gated_operations_are_declared_user_operations() {
    for operation in [
        "example.pay.nope",
        // Admin, service, and lifecycle operations never serve a user ticket.
        "example.pay.audit",
        "example.pay.debit",
        "example.pay.unbound",
    ] {
        let mut value = with_feature(|f| f["gates"]["operations"] = json!([operation]));
        with_card(&mut value);
        assert_eq!(refused(&value), "features.gates.operations", "{operation}");
    }
    assert_eq!(
        refused(&with_feature(|f| {
            f["gates"]["operations"] = json!(["example.pay.send", "example.pay.send"])
        })),
        "features.gates.operations"
    );
}

#[test]
fn gated_service_operations_are_declared_with_user_mode() {
    for operation in ["example.pay.refund", "example.pay.send", "example.pay.nope"] {
        assert_eq!(
            refused(&with_feature(|f| {
                f["gates"]["serviceOperations"] = json!([operation])
            })),
            "features.gates.serviceOperations",
            "{operation}"
        );
    }
}

#[test]
fn gated_contributions_exist() {
    assert_eq!(
        refused(&with_feature(|f| {
            f["gates"]["contributions"] = json!(["example.pay.nope"])
        })),
        "features.gates.contributions"
    );
}

#[test]
fn a_configuration_gate_is_a_client_visible_boolean() {
    for key in ["internal", "limit", "nope"] {
        assert_eq!(
            refused(&with_feature(|f| f["gates"]["configuration"] = json!(key))),
            "features.gates.configuration",
            "{key}"
        );
    }
    let mut no_schema = manifest();
    no_schema
        .as_object_mut()
        .unwrap()
        .remove("configurationSchema");
    assert_eq!(refused(&no_schema), "features.gates.configuration");
}

#[test]
fn requires_resolve_in_this_plugin_or_a_declared_dependency() {
    for reference in [
        "example.pay.nope",
        "example.other.send",
        "example.bank",
        "payments",
    ] {
        assert_eq!(
            refused(&with_feature(|f| f["requires"] = json!([reference]))),
            "features.requires",
            "{reference}"
        );
    }
    assert_eq!(
        refused(&with_feature(|f| f["requires"] = json!(["Not An Id"]))),
        "features.requires"
    );
}

#[test]
fn the_kind_follows_host_access_and_message_content() {
    assert_eq!(parse(&manifest()).unwrap().kind(), PluginKind::Business);
    let with_capability = |id: &str| {
        let mut value = manifest();
        value["capabilities"][0]["operations"][0]["hostCapabilities"] = json!([id]);
        value["permissions"][0]["dataScopes"] = json!([id]);
        parse(&value).unwrap().kind()
    };
    assert_eq!(with_capability("im.conversation.read"), PluginKind::Im);
    assert_eq!(with_capability("social.group.member.read"), PluginKind::Im);
    assert_eq!(
        with_capability("plugin.service.dispatch"),
        PluginKind::Business
    );
    let mut card = manifest();
    with_card(&mut card);
    assert_eq!(parse(&card).unwrap().kind(), PluginKind::Im);
}

#[test]
fn a_missing_key_is_closed_and_a_configuration_gate_closes_nothing() {
    let manifest = parse(&manifest()).unwrap();
    let states = |entries: &[(&str, bool)]| -> BTreeMap<String, bool> {
        entries.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    };
    let set =
        |ids: &[&str]| -> BTreeSet<String> { ids.iter().map(|id| (*id).to_owned()).collect() };
    let none = states(&[]);
    assert_eq!(
        manifest.closed_operations(&none),
        set(&["example.pay.send", "example.pay.stats"])
    );
    assert_eq!(
        manifest.closed_service_operations(&none),
        set(&["example.pay.debit"])
    );
    assert_eq!(
        manifest.closed_contributions(&none),
        set(&["example.pay.home"])
    );
    // An operation of two features closes with either.
    let send_only = states(&[("send", true), ("stats", false)]);
    assert_eq!(
        manifest.closed_operations(&send_only),
        set(&["example.pay.send", "example.pay.stats"])
    );
    assert!(manifest.closed_service_operations(&send_only).is_empty());
    assert!(manifest.closed_contributions(&send_only).is_empty());
    let open = states(&[("send", true), ("stats", true), ("video", false)]);
    assert!(manifest.closed_operations(&open).is_empty());
    assert!(manifest.closed_service_operations(&open).is_empty());
    assert!(manifest.closed_contributions(&open).is_empty());
}

#[test]
fn a_preview_is_a_bounded_object() {
    let with_preview = |preview: Value| {
        let mut value = manifest();
        value["clientExtensions"]["contributions"][0]["preview"] = preview;
        value
    };
    let field = "clientExtensions.contributions.preview";
    let manifest = parse(&with_preview(json!({"title": "Sample"}))).unwrap();
    assert_eq!(
        manifest.client_extensions.contributions[0].preview,
        Some(json!({"title": "Sample"}))
    );
    assert_eq!(refused(&with_preview(json!(["sample"]))), field);
    // `{"a":""}` is eight bytes.
    let text = "x".repeat(MAX_UI_PREVIEW_BYTES - 8);
    assert!(parse(&with_preview(json!({"a": text}))).is_ok());
    assert_eq!(
        refused(&with_preview(json!({"a": format!("{text}x")}))),
        field
    );
}

#[test]
fn the_asset_table_is_bounded_and_stays_below_assets() {
    let digest = format!("sha256:{}", "a".repeat(64));
    let with_assets = |assets: Value| {
        let mut value = manifest();
        value["assets"] = assets;
        value
    };
    let asset = |path: &str| json!({"path": path, "digest": digest, "size": 10});
    let manifest = parse(&with_assets(json!([asset("assets/icons/app-1.png")]))).unwrap();
    assert_eq!(manifest.assets[0].size, 10);
    for path in [
        "icon.png",
        "assets/",
        "assets",
        "/assets/icon.png",
        "assets//icon.png",
        "assets/../icon.png",
        "assets/./icon.png",
        "assets\\icon.png",
        "assets/图标.png",
    ] {
        assert_eq!(
            refused(&with_assets(json!([asset(path)]))),
            "assets.path",
            "{path}"
        );
    }
    assert_eq!(
        refused(&with_assets(json!([
            asset("assets/a.png"),
            asset("assets/a.png")
        ]))),
        "assets"
    );
    assert_eq!(
        refused(&with_assets(
            json!([{"path": "assets/a.png", "digest": "sha256:ABC", "size": 1}])
        )),
        "assets.digest"
    );
    assert_eq!(
        refused(&with_assets(
            json!([{"path": "assets/a.png", "digest": digest, "size": 0}])
        )),
        "assets.size"
    );
    assert_eq!(
        refused(&with_assets(json!([
            {"path": "assets/a.png", "digest": digest, "size": MAX_PACKAGE_BYTES},
            {"path": "assets/b.png", "digest": digest, "size": 1}
        ]))),
        "assets.size"
    );
}

#[test]
fn the_uninstall_check_is_an_admin_query() {
    let with_check = |check: &str| {
        let mut value = manifest();
        value["admin"]["lifecycle"]["uninstall"]["check"] = json!(check);
        value
    };
    assert_eq!(
        parse(&manifest())
            .unwrap()
            .admin
            .unwrap()
            .lifecycle
            .unwrap()
            .uninstall
            .unwrap()
            .check
            .as_deref(),
        Some("example.pay.audit")
    );
    for check in ["example.pay.close", "example.pay.send", "example.pay.nope"] {
        assert_eq!(refused(&with_check(check)), "admin", "{check}");
    }
}

#[test]
fn unset_projection_fields_leave_the_wire_as_it_was() {
    let projected = ProjectedPlugin {
        grant: TenantPluginGrant {
            grant_id: "grant-1".into(),
            revision: 1,
            tenant_id: "201".into(),
            plugin_id: "example.pay".into(),
            plugin_version: "1.0.0".into(),
            manifest_digest: "sha256:x".into(),
            approved_operations: Default::default(),
            approved_permissions: Default::default(),
            approved_data_scopes: Default::default(),
            enabled: true,
            revoked: false,
            suspended: false,
            valid_from_ms: 1,
            valid_until_ms: 2,
        },
        lifecycle: ProjectedLifecycle::Installed,
        export_until_ms: None,
        features: Default::default(),
    };
    let raw = serde_json::to_value(&projected).unwrap();
    assert!(raw.get("features").is_none());
    assert!(raw["grant"].get("suspended").is_none());
    assert_eq!(
        serde_json::from_value::<ProjectedPlugin>(raw).unwrap(),
        projected
    );
    let mut set = projected;
    set.grant.suspended = true;
    set.features.insert("send".into(), true);
    let raw = serde_json::to_value(&set).unwrap();
    assert_eq!(raw["features"], json!({"send": true}));
    assert_eq!(raw["grant"]["suspended"], json!(true));
}
