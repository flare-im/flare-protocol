use flare_plugin_protocol::{PluginManifest, ProtocolError};
use serde_json::{Value, json};

fn manifest() -> Value {
    json!({
        "manifestVersion": 1, "pluginId": "example.poll", "version": "1.0.0", "apiVersion": "1.0",
        "displayName": "Poll", "publisher": {"publisherId": "example", "displayName": "Example"},
        "runtime": {"kind": "client"},
        "capabilities": [{"id": "example.poll", "operations": [{
            "id": "run", "inputSchema": {"type": "object"}, "outputSchema": {"type": "object"},
            "permissionIds": ["example.run"], "idempotent": true, "timeoutMs": 1000
        }]}],
        "permissions": [{"id": "example.run", "description": "Run", "dataScopes": [], "riskLevel": "low"}],
        "features": [{"id": "run", "label": {"en-US": "Run"}, "gates": {"operations": ["example.poll.run"]}}]
    })
}

fn parse(value: &Value) -> Result<PluginManifest, ProtocolError> {
    PluginManifest::from_json(&serde_json::to_vec(value).unwrap())
}

#[test]
fn accepts_publisher_owned_localized_display_names() {
    let mut value = manifest();
    value["displayNameI18n"] = json!({"zh-CN": "投票", "en-US": "Poll"});
    let parsed = parse(&value).expect("localized names are part of the signed manifest");
    assert_eq!(
        serde_json::to_value(parsed).unwrap()["displayNameI18n"],
        value["displayNameI18n"]
    );
}

#[test]
fn absent_translations_preserve_the_serialized_manifest() {
    let parsed = parse(&manifest()).unwrap();
    assert!(
        serde_json::to_value(parsed)
            .unwrap()
            .get("displayNameI18n")
            .is_none()
    );
}

#[test]
fn resolves_exact_language_default_and_defensive_id_fallbacks() {
    let mut value = manifest();
    value["displayNameI18n"] = json!({"zh-CN": "投票", "en-US": "Poll US", "en-GB": "Poll GB"});
    let mut parsed = parse(&value).unwrap();
    assert_eq!(parsed.localized_display_name("zh-CN"), "投票");
    assert_eq!(parsed.localized_display_name("EN-us"), "Poll US");
    assert_eq!(parsed.localized_display_name("zh-TW"), "投票");
    assert_eq!(parsed.localized_display_name("en"), "Poll GB");
    assert_eq!(parsed.localized_display_name("fr-FR"), "Poll");
    assert_eq!(parsed.localized_display_name(""), "Poll");
    parsed.display_name.clear();
    assert_eq!(parsed.localized_display_name("fr-FR"), "example.poll");
}

#[test]
fn rejects_invalid_or_unbounded_localized_names() {
    for translations in [
        json!({"zh_CN": "投票"}),
        json!({"": "Poll"}),
        json!({"en-US": "   "}),
        json!({"en-US": "Line\nbreak"}),
        json!({"en-US": "x".repeat(161)}),
        Value::Object(
            (0..17)
                .map(|i| (format!("en-{i}"), json!("Poll")))
                .collect(),
        ),
    ] {
        let mut value = manifest();
        value["displayNameI18n"] = translations;
        assert!(
            matches!(parse(&value), Err(ProtocolError::InvalidManifest { field, .. }) if field == "displayNameI18n")
        );
    }
}

#[test]
fn schema_keeps_translations_optional_and_bounds_the_locale_count() {
    let schema = serde_json::to_value(schemars::schema_for!(PluginManifest)).unwrap();
    assert_eq!(schema["properties"]["displayNameI18n"]["maxProperties"], 16);
    assert!(
        !schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("displayNameI18n"))
    );
}
