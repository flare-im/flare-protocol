//! Tenant configuration that clients may see. A plugin marks top-level
//! properties of its `configurationSchema` with `x-flare-client-visible: true`
//! (for example the maximum number of poll options, or whether anonymous polls
//! are allowed). The platform sends only that subset with the client catalog
//! ([`crate::ui::ClientPlugin::configuration`]); everything else stays with the
//! plugin service. Visibility is not authorization: the plugin re-checks every
//! limit it publishes this way.
use crate::{
    ProtocolError, Result,
    admin::{AdminHintWidget, AdminUiHint},
};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const CLIENT_VISIBLE_KEYWORD: &str = "x-flare-client-visible";
/// Serialized size of the client-visible subset.
pub const MAX_CLIENT_CONFIGURATION_BYTES: usize = 4096;

/// Top-level property names marked client-visible.
pub fn client_visible_keys(schema: &Value) -> BTreeSet<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter(|(_, property)| {
                    property.get(CLIENT_VISIBLE_KEYWORD) == Some(&Value::Bool(true))
                })
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The client-visible subset of `configuration` (an object). Keys absent from
/// the configuration are absent from the subset.
pub fn client_visible_configuration(
    schema: &Value,
    configuration: &Value,
) -> Result<Map<String, Value>> {
    let configuration = configuration
        .as_object()
        .ok_or(ProtocolError::Denied("configuration must be an object"))?;
    let visible: Map<String, Value> = client_visible_keys(schema)
        .into_iter()
        .filter_map(|key| configuration.get(&key).map(|value| (key, value.clone())))
        .collect();
    if serde_json::to_vec(&visible).map_or(true, |v| v.len() > MAX_CLIENT_CONFIGURATION_BYTES) {
        return Err(ProtocolError::Denied("client configuration exceeds limit"));
    }
    Ok(visible)
}

/// Object keys from the configuration's root to a secret value. An array on the way applies the
/// rest of the path to each of its items.
pub type SecretPath = Vec<String>;

/// Where `configurationSchema` and `admin.settings.uiHints` mark a value secret: `writeOnly: true`
/// in the schema, or the `secret` widget. These are the values encrypted at rest.
pub fn secret_paths(
    schema: Option<&Value>,
    hints: Option<&BTreeMap<String, AdminUiHint>>,
) -> Vec<SecretPath> {
    fn in_schema(schema: &Value, prefix: &[String], found: &mut BTreeSet<SecretPath>) {
        let schema = schema.get("items").unwrap_or(schema);
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return;
        };
        for (name, property) in properties {
            let mut path = prefix.to_vec();
            path.push(name.clone());
            if property.get("writeOnly") == Some(&Value::Bool(true)) {
                found.insert(path.clone());
            }
            in_schema(property, &path, found);
        }
    }
    fn in_hints(
        hints: &BTreeMap<String, AdminUiHint>,
        prefix: &[String],
        found: &mut BTreeSet<SecretPath>,
    ) {
        for (name, hint) in hints {
            let mut path = prefix.to_vec();
            path.push(name.clone());
            if hint.widget == Some(AdminHintWidget::Secret) {
                found.insert(path.clone());
            }
            in_hints(&hint.fields, &path, found);
        }
    }
    let mut found = BTreeSet::new();
    if let Some(schema) = schema {
        in_schema(schema, &[], &mut found);
    }
    if let Some(hints) = hints {
        in_hints(hints, &[], &mut found);
    }
    found.into_iter().collect()
}

/// Stands for a secret value in every configuration Core answers: a value is set here, and nothing
/// else is said about it. Written back unchanged, it keeps the stored value.
pub const SECRET_PRESENT: &str = "__flare_secret_present__";

fn each_secret(value: &mut Value, path: &[String], visit: &mut dyn FnMut(&mut Value, &[Key])) {
    fn walk(
        value: &mut Value,
        path: &[String],
        at: &mut Vec<Key>,
        visit: &mut dyn FnMut(&mut Value, &[Key]),
    ) {
        match (value, path) {
            (Value::Array(items), _) => {
                for (index, item) in items.iter_mut().enumerate() {
                    at.push(Key::Index(index));
                    walk(item, path, at, visit);
                    at.pop();
                }
            }
            (value, []) => visit(value, at),
            (Value::Object(object), [key, rest @ ..]) => {
                if let Some(next) = object.get_mut(key) {
                    at.push(Key::Name(key.clone()));
                    walk(next, rest, at, visit);
                    at.pop();
                }
            }
            _ => (),
        }
    }
    walk(value, path, &mut Vec::new(), visit);
}

#[derive(Clone)]
enum Key {
    Name(String),
    Index(usize),
}

fn at<'a>(mut value: &'a Value, keys: &[Key]) -> Option<&'a Value> {
    for key in keys {
        value = match key {
            Key::Name(name) => value.get(name.as_str())?,
            Key::Index(index) => value.get(*index)?,
        };
    }
    Some(value)
}

fn holds_mask(value: &Value) -> bool {
    match value {
        Value::String(text) => text == SECRET_PRESENT,
        Value::Array(items) => items.iter().any(holds_mask),
        Value::Object(object) => object.values().any(holds_mask),
        _ => false,
    }
}

/// Replaces every value set at `paths` with [`SECRET_PRESENT`]. An empty or absent value stays:
/// it is what tells the reader nothing is set.
pub fn mask_secrets(configuration: &mut Value, paths: &[SecretPath]) {
    for path in paths {
        each_secret(configuration, path, &mut |value, _| {
            if !value.is_null() && value.as_str() != Some("") {
                *value = Value::String(SECRET_PRESENT.to_owned());
            }
        });
    }
}

/// Puts the stored value of `previous` back wherever `configuration` holds [`SECRET_PRESENT`] at
/// `paths`. Refused when there is no stored value, and when the marker is anywhere else: a value
/// that is not secret here must not be filled from one that was.
pub fn restore_secrets(
    configuration: &mut Value,
    previous: Option<&Value>,
    paths: &[SecretPath],
) -> Result<()> {
    let mut missing = false;
    for path in paths {
        each_secret(configuration, path, &mut |value, keys| {
            if value.as_str() != Some(SECRET_PRESENT) {
                return;
            }
            match previous.and_then(|previous| at(previous, keys)) {
                Some(stored) if !stored.is_null() && !holds_mask(stored) => *value = stored.clone(),
                _ => missing = true,
            }
        });
    }
    if missing || holds_mask(configuration) {
        return Err(ProtocolError::Denied("masked secret has no stored value"));
    }
    Ok(())
}

/// The marker is a boolean on a top-level property and nowhere else, so a
/// nested marker never looks like it hides or exposes something it does not.
pub(crate) fn validate_client_visible_markers(schema: &Value) -> Result<()> {
    fn nested(value: &Value) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key(CLIENT_VISIBLE_KEYWORD) || object.values().any(nested)
            }
            Value::Array(items) => items.iter().any(nested),
            _ => false,
        }
    }
    let invalid = || ProtocolError::InvalidManifest {
        field: "configurationSchema".into(),
        reason: format!("{CLIENT_VISIBLE_KEYWORD} must be a boolean on a top-level property"),
    };
    let Value::Object(root) = schema else {
        return Ok(());
    };
    for (key, value) in root {
        if key == CLIENT_VISIBLE_KEYWORD {
            return Err(invalid());
        }
        if key != "properties" {
            if nested(value) {
                return Err(invalid());
            }
            continue;
        }
        let Some(properties) = value.as_object() else {
            continue;
        };
        for property in properties.values() {
            let Value::Object(property) = property else {
                continue;
            };
            for (name, child) in property {
                if name == CLIENT_VISIBLE_KEYWORD {
                    if !child.is_boolean() {
                        return Err(invalid());
                    }
                } else if nested(child) {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}
