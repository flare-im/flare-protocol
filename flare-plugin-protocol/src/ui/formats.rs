//! Console vocabulary of a UI document: how a field reads ([`UiFieldFormat`]) and
//! where a control takes its choices from ([`UiReference`]).
//!
//! A format is optional and display only. The resolved value always carries its
//! plain text, so a surface that does not know a format draws that text. The
//! resolved format ([`ResolvedUiFormat`]) only adds what a richer surface needs
//! (a user id, an instant, an asset whose scale the host knows).
//!
//! A reference is read by the host with the viewer's own permission, through the
//! platform: the declaring plugin never reads for the viewer. See
//! [`crate::admin::PluginAdmin::validate`] for what publication checks and the
//! platform's reference route for what is checked when the plugin is enabled
//! and on every read.
use super::{UiTranslation, translate, valid_name, valid_path, valid_text};
use crate::{ProtocolError, Result, tagged::kind_tagged};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Largest decimal scale of a money value.
pub const MAX_UI_MONEY_SCALE: u8 = 18;
/// Digits of a money value in minor units.
const MAX_UI_MINOR_DIGITS: usize = 40;
/// Mapped values of an enum format.
const MAX_UI_ENUM_LABELS: usize = 64;
/// Fixed and parent inputs of a reference.
pub const MAX_UI_REFERENCE_PARAMS: usize = 8;
pub const MAX_UI_REFERENCE_PARENTS: usize = 4;
/// Options one read of a reference may answer.
pub const MAX_UI_REFERENCE_OPTIONS: usize = 100;
/// Bytes of an option value, an option label and a read cursor.
pub const MAX_UI_REFERENCE_VALUE_BYTES: usize = 128;
pub const MAX_UI_REFERENCE_LABEL_BYTES: usize = 512;
pub const MAX_UI_REFERENCE_CURSOR_BYTES: usize = 512;
/// Characters of the text a viewer searches a reference for.
pub const MAX_UI_REFERENCE_SEARCH_CHARS: usize = 128;
/// What a plugin records in front of the id of a staff actor.
pub const STAFF_ACTOR_PREFIX: &str = "$staff:";
/// Input names the platform fills on every reference read.
pub const REFERENCE_CURSOR_INPUT: &str = "cursor";
pub const REFERENCE_SELECTED_INPUT: &str = "selected";

/// How a field or a column reads, tagged by `kind`. Paths read the object the
/// field reads: the resolved data for a field, the row for a column.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UiFieldFormat {
    Money(UiMoneyFormat),
    Enum(UiEnumFormat),
    Person(UiPersonFormat),
    Reference(UiReferenceFormat),
    Time(UiTimeFormat),
}
kind_tagged! {
    UiFieldFormat = "UiFieldFormat" {
        "money" => Money(UiMoneyFormat),
        "enum" => Enum(UiEnumFormat),
        "person" => Person(UiPersonFormat),
        "reference" => Reference(UiReferenceFormat),
        "time" => Time(UiTimeFormat),
    }
}

/// An integer amount in minor units, as a number or a digit string. The scale
/// comes from `scale` or `scalePath`. Without either, or when the path holds
/// no scale, the host looks the asset at `assetPath` up in what it already
/// knows; until then the value stays the minor amount. At least one of the
/// three is set, and at most one of `scale` and `scalePath`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiMoneyFormat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_path: Option<String>,
    /// `0..=18`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<u8>,
}

/// Labels of an enumerated value (`1..=64`, keys `1..=64` bytes), translated
/// through `texts`. An unmapped value is shown as sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiEnumFormat {
    pub labels: BTreeMap<String, String>,
}

/// A user id: the host shows the person. A staff actor is written
/// [`STAFF_ACTOR_PREFIX`]`<id>`; the host gets the bare id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPersonFormat {}

/// An id shown through its label, read beside it at `labelPath`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiReferenceFormat {
    pub label_path: String,
}

/// Epoch milliseconds: the host formats the instant.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiTimeFormat {}

/// What a resolved field adds to its plain text.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolvedUiFormat {
    Money(ResolvedUiMoney),
    Enum(ResolvedUiEnum),
    Person(ResolvedUiPerson),
    Reference(ResolvedUiReference),
    Time(ResolvedUiInstant),
}
kind_tagged! {
    ResolvedUiFormat = "ResolvedUiFormat" {
        "money" => Money(ResolvedUiMoney),
        "enum" => Enum(ResolvedUiEnum),
        "person" => Person(ResolvedUiPerson),
        "reference" => Reference(ResolvedUiReference),
        "time" => Time(ResolvedUiInstant),
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiMoney {
    /// The amount in minor units, as sent.
    pub minor: String,
    /// Absent: the value is still `minor`; the host may scale it by `assetId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiEnum {
    pub raw: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiPerson {
    pub user_id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiReference {
    pub id: String,
    /// False: the data held no label and the value is the bare id, which a
    /// console does not show.
    pub labelled: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiInstant {
    pub at_ms: i64,
}

fn minor_text(raw: &str, scale: u8) -> String {
    let scale = usize::from(scale);
    if scale == 0 {
        return raw.to_owned();
    }
    let (sign, digits) = raw.strip_prefix('-').map_or(("", raw), |d| ("-", d));
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (whole, fraction) = padded.split_at(padded.len() - scale);
    format!("{sign}{whole}.{fraction}")
}
fn is_minor(raw: &str) -> bool {
    let digits = raw.strip_prefix('-').unwrap_or(raw);
    !digits.is_empty()
        && digits.len() <= MAX_UI_MINOR_DIGITS
        && digits.bytes().all(|b| b.is_ascii_digit())
}
fn text_at(data: &Value, path: &str) -> Option<String> {
    data.pointer(path)
        .and_then(super::scalar_text)
        .filter(|text| !text.trim().is_empty())
}

impl UiFieldFormat {
    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::Money(f) => {
                (f.asset_path.is_some() || f.scale_path.is_some() || f.scale.is_some())
                    && !(f.scale.is_some() && f.scale_path.is_some())
                    && f.scale.is_none_or(|s| s <= MAX_UI_MONEY_SCALE)
                    && [&f.asset_path, &f.scale_path]
                        .into_iter()
                        .all(|p| p.as_deref().is_none_or(valid_path))
            }
            Self::Enum(f) => {
                !f.labels.is_empty()
                    && f.labels.len() <= MAX_UI_ENUM_LABELS
                    && f.labels.iter().all(|(value, label)| {
                        !value.is_empty() && value.len() <= 64 && valid_text(label)
                    })
            }
            Self::Reference(f) => valid_path(&f.label_path),
            Self::Person(_) | Self::Time(_) => true,
        }
    }

    /// The text and the format of the value `raw` read from `data`. A value
    /// the format does not fit stays plain: no text, no format.
    pub(crate) fn resolve(
        &self,
        data: &Value,
        raw: &str,
        translation: Option<&UiTranslation>,
    ) -> (Option<String>, Option<ResolvedUiFormat>) {
        match self {
            Self::Money(f) => {
                if !is_minor(raw) {
                    return (None, None);
                }
                let scale = f.scale.or_else(|| {
                    data.pointer(f.scale_path.as_deref()?)?
                        .as_u64()
                        .and_then(|s| u8::try_from(s).ok())
                        .filter(|s| *s <= MAX_UI_MONEY_SCALE)
                });
                (
                    scale.map(|scale| minor_text(raw, scale)),
                    Some(ResolvedUiFormat::Money(ResolvedUiMoney {
                        minor: raw.to_owned(),
                        scale,
                        asset_id: f.asset_path.as_deref().and_then(|p| text_at(data, p)),
                    })),
                )
            }
            Self::Enum(f) => (
                f.labels.get(raw).map(|label| translate(translation, label)),
                Some(ResolvedUiFormat::Enum(ResolvedUiEnum {
                    raw: raw.to_owned(),
                })),
            ),
            Self::Person(_) => {
                let id = raw.strip_prefix(STAFF_ACTOR_PREFIX).unwrap_or(raw);
                (
                    None,
                    crate::resource::valid_resource_id(id).then(|| {
                        ResolvedUiFormat::Person(ResolvedUiPerson {
                            user_id: id.to_owned(),
                        })
                    }),
                )
            }
            Self::Reference(f) => {
                let label = text_at(data, &f.label_path);
                let labelled = label.is_some();
                (
                    label,
                    Some(ResolvedUiFormat::Reference(ResolvedUiReference {
                        id: raw.to_owned(),
                        labelled,
                    })),
                )
            }
            Self::Time(_) => (
                None,
                raw.parse()
                    .ok()
                    .map(|at_ms| ResolvedUiFormat::Time(ResolvedUiInstant { at_ms })),
            ),
        }
    }
}

/// Where a select input or a configuration field takes its options from: an
/// admin `query` operation of this plugin, or of `plugin`, one of its declared
/// dependencies.
///
/// Each read calls `operation` with `params`, the values of `parents`, the
/// viewer's `search` text, `cursor` and `selected` (the value already chosen,
/// so it can be named when it is not on the first page). The operation answers
/// `{items: [..], nextCursor?, selected?}`; `valuePath` and `labelPath` read
/// inside an item and inside `selected`.
///
/// The read runs under the viewer's own permission on the plugin that owns the
/// operation. A viewer without it gets a disabled control: this plugin's
/// permissions never read for them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiReference {
    /// The plugin that owns `operation`; this plugin when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    pub operation: String,
    pub value_path: String,
    pub label_path: String,
    /// Operation input of the viewer's search text. Absent: no search box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    /// Fixed scalar inputs of the operation (`0..=8`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    /// Operation inputs taken from sibling controls (`0..=4`), by operation
    /// input name: the id of an input of the same document, or a property next
    /// to the configuration field. The control stays disabled until each holds
    /// a value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parents: BTreeMap<String, String>,
}

/// One option of a reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiReferenceOption {
    pub value: String,
    pub label: String,
}

/// One read of a reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiReferencePage {
    pub items: Vec<UiReferenceOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// The value the read asked about, when the viewer can see it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<UiReferenceOption>,
}

fn reserved(name: &str) -> bool {
    name == REFERENCE_CURSOR_INPUT || name == REFERENCE_SELECTED_INPUT
}
fn scalar(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

impl UiReference {
    /// The plugin that owns the operation, for a reference declared by `own`.
    pub fn target<'a>(&'a self, own: &'a str) -> &'a str {
        self.plugin.as_deref().unwrap_or(own)
    }

    /// Whether `admin` of the owning plugin serves this reference.
    pub fn served_by(&self, admin: &crate::admin::PluginAdmin) -> bool {
        admin
            .operation(&self.operation)
            .is_some_and(|op| op.mode == crate::manifest::OperationMode::Query)
    }

    pub(crate) fn is_valid(&self) -> bool {
        let mut names = std::collections::BTreeSet::new();
        self.plugin
            .as_deref()
            .is_none_or(|p| !p.is_empty() && p.len() <= 128)
            && !self.operation.is_empty()
            && self.operation.len() <= 128
            && valid_path(&self.value_path)
            && valid_path(&self.label_path)
            && self.params.len() <= MAX_UI_REFERENCE_PARAMS
            && self.parents.len() <= MAX_UI_REFERENCE_PARENTS
            && self.params.values().all(scalar)
            && self.parents.values().all(|id| valid_name(id))
            && self
                .params
                .keys()
                .chain(self.parents.keys())
                .chain(self.search.iter())
                .all(|name| valid_name(name) && !reserved(name) && names.insert(name))
    }

    /// The operation input of one read. `parents` must hold exactly the
    /// declared names, each a scalar; `search` needs a declared search input.
    pub fn input(
        &self,
        search: Option<&str>,
        cursor: Option<&str>,
        selected: Option<&str>,
        parents: &Map<String, Value>,
    ) -> Result<Value> {
        let refused = || ProtocolError::Denied("invalid UI reference read");
        let declared = |name: &String| self.parents.contains_key(name);
        if parents.len() != self.parents.len()
            || !parents.keys().all(declared)
            || !parents.values().all(|value| {
                scalar(value)
                    && value
                        .as_str()
                        .is_none_or(|s| !s.is_empty() && s.len() <= 256)
            })
        {
            return Err(refused());
        }
        let mut input: Map<String, Value> = self
            .params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        input.extend(parents.clone());
        if let Some(text) = search.filter(|text| !text.is_empty()) {
            let name = self.search.as_ref().ok_or_else(refused)?;
            if text.chars().count() > MAX_UI_REFERENCE_SEARCH_CHARS
                || text.chars().any(char::is_control)
            {
                return Err(refused());
            }
            input.insert(name.clone(), Value::from(text));
        }
        if let Some(cursor) = cursor {
            if cursor.is_empty()
                || cursor.len() > MAX_UI_REFERENCE_CURSOR_BYTES
                || cursor.chars().any(char::is_control)
            {
                return Err(refused());
            }
            input.insert(REFERENCE_CURSOR_INPUT.into(), Value::from(cursor));
        }
        if let Some(selected) = selected {
            if selected.is_empty()
                || selected.len() > MAX_UI_REFERENCE_VALUE_BYTES
                || selected.chars().any(char::is_control)
            {
                return Err(refused());
            }
            input.insert(REFERENCE_SELECTED_INPUT.into(), Value::from(selected));
        }
        Ok(Value::Object(input))
    }

    fn option(&self, item: &Value) -> Option<UiReferenceOption> {
        let value = text_at(item, &self.value_path)?;
        let label = text_at(item, &self.label_path)?;
        (value.len() <= MAX_UI_REFERENCE_VALUE_BYTES
            && label.len() <= MAX_UI_REFERENCE_LABEL_BYTES
            && !value.chars().any(char::is_control)
            && !label.chars().any(char::is_control))
        .then_some(UiReferenceOption { value, label })
    }

    /// The operation's answer as options, or `None` when it does not keep the
    /// contract: a host never shows half of a malformed directory.
    pub fn page(&self, data: &Value) -> Option<UiReferencePage> {
        let items = data.pointer("/items")?.as_array()?;
        if items.len() > MAX_UI_REFERENCE_OPTIONS {
            return None;
        }
        let next_cursor = match data.pointer("/nextCursor") {
            None | Some(Value::Null) => None,
            Some(Value::String(cursor)) if cursor.is_empty() => None,
            Some(Value::String(cursor))
                if cursor.len() <= MAX_UI_REFERENCE_CURSOR_BYTES
                    && !cursor.chars().any(char::is_control) =>
            {
                Some(cursor.clone())
            }
            Some(_) => return None,
        };
        let selected = match data.pointer("/selected") {
            None | Some(Value::Null) => None,
            Some(item) => Some(self.option(item)?),
        };
        Some(UiReferencePage {
            items: items
                .iter()
                .map(|item| self.option(item))
                .collect::<Option<_>>()?,
            next_cursor,
            selected,
        })
    }
}
