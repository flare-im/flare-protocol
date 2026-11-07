//! Data-only UI protocol shared by every client. No HTML, URLs, script, SQL,
//! expression evaluator, or dynamic native code is accepted.
//!
//! Version 2 of the document is additive and keeps `schemaVersion: 1`: display
//! `blocks`, form `inputs`, action `form`/`confirm`/`style`, `exists`
//! conditions, `texts` translations, the contribution `detail`, and resolution
//! over two state layers ([`UiStateLayers`]). Blocks and the detail take
//! `hiddenWhen` conditions like actions. Later additions are optional in the
//! same way: `allOf` conditions, static choices `options`, entries
//! `userIdsPath`, rows `filters`, input `maxPath` and detail paging
//! (`cursorPath`). A composer-action document resolves against
//! [`composer_data`]. Older documents parse, validate and resolve exactly as
//! before. Plugin platform v2 adds, equally optional: the viewer layer
//! ([`UiViewerState`], [`PluginMessageState::based_on_version`] and the
//! versioned merge of [`UiStateLayers`]), grouped entries, menu actions
//! ([`UiActionPlacement`]), account-entry summaries ([`UiEntrySummary`]),
//! composer-entry availability ([`UiEntryAvailability`]),
//! the envelope's `displayName`, and why a view is inert
//! ([`UiUnavailableReason`]). Pages add a data source with paging
//! ([`UiPageSource`]) and rows on page documents; choices take a selection
//! limit (`maxSelectionsPath`), entries and rows a badge per item
//! ([`UiValueLabels`]).
mod blocks;
mod inputs;
pub use blocks::*;
pub use inputs::*;

use crate::{
    ProtocolError, Result,
    manifest::{ClientExtensions, ClientPlatform, UiContribution, UiContributionKind},
    state::SharedState,
    tagged::kind_tagged,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const UI_CATALOG_OPERATION: &str = "flare.plugin.ui.catalog";
/// Document bounds.
pub const MAX_UI_FIELDS: usize = 32;
pub const MAX_UI_ACTIONS: usize = 16;
pub const MAX_UI_BLOCKS: usize = 24;
pub const MAX_UI_INPUTS: usize = 32;
/// Labels, titles, placeholders and confirmations: bytes, no control characters.
pub const MAX_UI_TEXT_BYTES: usize = 256;
/// Literal text of text and note blocks: bytes, newlines allowed.
pub const MAX_UI_LONG_TEXT_BYTES: usize = 2000;
/// Entries of one translation's `texts`.
pub const MAX_UI_TRANSLATED_TEXTS: usize = 256;
/// Serialized size of an action input (declared input plus collected values).
pub const MAX_UI_ACTION_INPUT_BYTES: usize = 16384;
/// Bounded inert props for a compiled-in design-kit presentation.
pub const MAX_UI_PRESENTATION_BYTES: usize = 65536;
/// Key of the client-visible configuration in the data of a composer-action
/// document (see [`composer_data`]).
pub const COMPOSER_CONFIG_KEY: &str = "config";
/// Input name of the page cursor sent to a paged detail operation.
pub const DETAIL_CURSOR_INPUT: &str = "cursor";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiDocument {
    pub title: String,
    /// Optional compiled-in design-kit renderer. Contains inert display data
    /// and references to this document's actions, never executable code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<UiPresentation>,
    #[serde(default)]
    pub fields: Vec<UiField>,
    #[serde(default)]
    pub actions: Vec<UiAction>,
    /// Display blocks, in order (`0..=24`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<UiBlock>,
    /// Form inputs referenced by action forms, and the filters of the rows
    /// block of a page or an admin page (`admin.pages`): its `filters`, or
    /// every input when it lists none (`0..=16`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<UiInput>,
    #[serde(default)]
    pub translations: BTreeMap<String, UiTranslation>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPresentation {
    pub renderer: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub actions: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiPresentation {
    pub renderer: String,
    pub props: BTreeMap<String, Value>,
    pub actions: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiTranslation {
    pub title: String,
    /// One label per document field, in order (empty when the document has
    /// no fields).
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub actions: BTreeMap<String, String>,
    /// Display labels for enumerated field values, keyed by field path.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, BTreeMap<String, String>>,
    /// Every other text of the document, keyed by its source text: block
    /// labels and literals, value labels of blocks and columns, input labels,
    /// placeholders and choices, action confirmations. A text without an entry
    /// is shown as declared.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub texts: BTreeMap<String, String>,
}
pub struct UiRenderContext<'a> {
    pub tenant: &'a str,
    pub platform: ClientPlatform,
    pub kind: UiContributionKind,
    pub locale: &'a str,
    /// The signed-in user the view is presented to; empty when unknown.
    pub viewer: &'a str,
    pub now_ms: i64,
}
/// The two state layers a card is resolved with, over its payload:
/// `payload ← shared ← local`. Each is a shallow overlay of top-level keys in
/// which null removes the key.
///
/// - `shared`: the entity's [`SharedState`] from Core (events and sync). It
///   applies only when the payload names the same entity at the
///   contribution's `entityPath`.
/// - `local`: the viewer's own layer ([`PluginMessageState`]), from the
///   viewer's call results (`data.messageState`) and the contribution's
///   [`UiViewerState`] reads, kept on this device only (such as
///   `mySelection`, `canClose`).
///
/// The local layer is versioned against the shared one: it was computed from
/// the shared version [`PluginMessageState::based_on_version`]. While that is
/// at least the applied shared version the local layer is fresh and each of
/// its keys replaces the shared key of the same name. Once a newer shared
/// version arrives it is stale: keys the shared layer also carries keep the
/// shared value (a stale `counts` never hides the final counts), its other
/// keys still apply. Without a shared layer every local key applies.
///
/// An invalid layer is ignored.
#[derive(Debug, Clone, Copy, Default)]
pub struct UiStateLayers<'a> {
    pub shared: Option<&'a SharedState>,
    pub local: Option<&'a PluginMessageState>,
}
impl UiStateLayers<'_> {
    /// The shared layer that applies to a card of `contribution`: valid and of
    /// the entity the payload names.
    pub fn applied_shared(
        &self,
        contribution: &UiContribution,
        payload: &Value,
    ) -> Option<&SharedState> {
        self.shared.filter(|shared| {
            shared.validate().is_ok()
                && contribution.entity_of(payload).as_deref() == Some(shared.entity.as_str())
        })
    }
    /// The data a card of `contribution` is resolved against.
    pub fn merge(&self, contribution: &UiContribution, payload: &Value) -> Value {
        let shared = self.applied_shared(contribution, payload);
        let mut data = shared.map_or_else(|| payload.clone(), |shared| shared.merge_into(payload));
        if let Some(local) = self.local.filter(|local| local.validate().is_ok()) {
            data = match shared {
                Some(shared) if !local.is_fresh_against(shared.version) => {
                    let values: Map<String, Value> = local
                        .values
                        .iter()
                        .filter(|(key, _)| !shared.values.contains_key(*key))
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect();
                    crate::state::overlay(&data, &values)
                }
                _ => local.merge_into(&data),
            };
        }
        data
    }
    /// [`Self::merge`] without the local layer: the data a
    /// [`UiViewerState`] input is bound from, so the viewer layer never feeds
    /// its own request.
    pub fn merge_shared(&self, contribution: &UiContribution, payload: &Value) -> Value {
        self.applied_shared(contribution, payload)
            .map_or_else(|| payload.clone(), |shared| shared.merge_into(payload))
    }
}

/// When the core SDK reads a card's [`UiViewerState`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum UiViewerRefresh {
    /// Presenting the card reads it when the viewer layer is missing or stale
    /// (computed from an older shared version).
    Present,
    /// A newer shared version of a card presented recently reads it again,
    /// without waiting for the host to present the card.
    SharedVersion,
}
/// Smallest and largest [`UiViewerState::min_interval_ms`].
pub const MIN_VIEWER_STATE_INTERVAL_MS: u32 = 1000;
pub const MAX_VIEWER_STATE_INTERVAL_MS: u32 = 3_600_000;
/// The viewer's own layer of a message-content card: what only this viewer
/// may see or do (`hasVoted`, `canClose`, counts visible to them alone). The
/// core SDK schedules it; hosts and kits write no call logic.
///
/// The SDK calls `operation` (a declared `query`) with `input` bound from the
/// card payload under its shared layer (never the local layer, so the result
/// never feeds its own request), when [`refreshOn`](Self::refresh_on) says
/// so, at most once per `minIntervalMs` per card, one call in flight per
/// card, with exponential backoff after failures, only for presented cards,
/// and never across a session change. The result `data` (an object) is
/// overlaid on the card's local layer (top-level keys, null removes one) and
/// stored with [`PluginMessageState::based_on_version`] taken from the
/// result's `basedOnVersion` (a non-negative integer, at most the shared
/// version seen when the call was made) or else that seen version. A result
/// may also carry `refreshOn`
/// to narrow the triggers for that card ([`Self::result_refresh_on`]); the
/// keys `basedOnVersion` and `refreshOn` are not stored as values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiViewerState {
    /// Wire operation declared by the plugin, mode `query`.
    pub operation: String,
    /// Input names to JSON pointers into the card data (`0..=32`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input: BTreeMap<String, String>,
    /// `1..=2` distinct triggers.
    pub refresh_on: Vec<UiViewerRefresh>,
    /// Calls per card at most once per this many milliseconds
    /// (`1000..=3600000`).
    pub min_interval_ms: u32,
}
impl UiViewerState {
    pub fn validate(&self) -> Result<()> {
        let refresh: BTreeSet<_> = self.refresh_on.iter().collect();
        if self.operation.is_empty()
            || self.operation.len() > 128
            || !valid_bindings(&self.input)
            || self.refresh_on.is_empty()
            || refresh.len() != self.refresh_on.len()
            || !(MIN_VIEWER_STATE_INTERVAL_MS..=MAX_VIEWER_STATE_INTERVAL_MS)
                .contains(&self.min_interval_ms)
        {
            return Err(ProtocolError::Denied("invalid UI viewer state"));
        }
        Ok(())
    }
    pub fn refreshes_on(&self, trigger: UiViewerRefresh) -> bool {
        self.refresh_on.contains(&trigger)
    }
    /// The operation input, bound from `data` ([`UiStateLayers::merge_shared`]).
    pub fn bind_input(&self, data: &Value) -> Result<Value> {
        bind_inputs(&self.input, data)
    }
    /// Whether a card whose local layer is `local` and whose applied shared
    /// layer has version `shared_version` (0 when none) needs a read: no
    /// local layer, or one computed from an older shared version.
    pub fn needs_read(local: Option<&PluginMessageState>, shared_version: u64) -> bool {
        local.is_none_or(|local| !local.is_fresh_against(shared_version))
    }
    /// The local layer a successful result `data` makes over `previous` (the
    /// stored layer, if any): its keys overlaid on the previous values,
    /// `basedOnVersion` from the result when it is a non-negative integer,
    /// `seen_shared_version` otherwise, and the previous layer's version (0
    /// when none), so a newer action result is never replaced by it. `None`
    /// when `data` is not an object or the layer would break the bounds of
    /// [`PluginMessageState::validate`].
    ///
    /// The result's `basedOnVersion` is capped at `seen_shared_version`
    /// ([`PluginMessageState::capped_at`]): a plugin cannot claim a shared
    /// version the caller never saw, which would keep its layer fresh against
    /// every later shared layer and hide the shared value of its keys for good.
    pub fn local_layer(
        data: &Value,
        previous: Option<&PluginMessageState>,
        seen_shared_version: u64,
    ) -> Option<PluginMessageState> {
        let mut result = data.as_object()?.clone();
        result.remove(REFRESH_ON_KEY);
        let based_on = match result.remove(BASED_ON_VERSION_KEY) {
            Some(Value::Number(n)) => n.as_u64()?.min(seen_shared_version),
            None | Some(Value::Null) => seen_shared_version,
            Some(_) => return None,
        };
        let mut values = previous.map(|p| p.values.clone()).unwrap_or_default();
        for (key, value) in result {
            if value.is_null() {
                values.remove(&key);
            } else {
                values.insert(key, value);
            }
        }
        let state = PluginMessageState {
            version: previous.map_or(0, |p| p.version),
            based_on_version: Some(based_on),
            values,
        };
        state.validate().ok().map(|()| state)
    }
}
/// Key of a [`UiViewerState`] result that names the shared version it was
/// computed from.
pub const BASED_ON_VERSION_KEY: &str = "basedOnVersion";
/// Key of a [`UiViewerState`] result that narrows `refreshOn` for that card
/// (a poll in a very large group reads only on presentation).
pub const REFRESH_ON_KEY: &str = "refreshOn";
impl UiViewerState {
    /// The triggers a result `data` keeps for its card: its `refreshOn`
    /// (strings, unknown ones ignored), intersected with the declared ones by
    /// the caller ([`Self::refreshes_on_with`]). `None` when the result does
    /// not say, so the declaration applies as is. Neither key is stored as a
    /// value of the local layer.
    pub fn result_refresh_on(data: &Value) -> Option<Vec<UiViewerRefresh>> {
        let items = data.get(REFRESH_ON_KEY)?.as_array()?;
        Some(
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect(),
        )
    }
    /// Whether `trigger` reads the card, given the declaration and the
    /// narrowing (`refreshOn`) its last result returned, if any. A result
    /// only narrows: it never adds a trigger the manifest does not declare.
    pub fn refreshes_on_with(
        &self,
        trigger: UiViewerRefresh,
        narrowed: Option<&[UiViewerRefresh]>,
    ) -> bool {
        self.refreshes_on(trigger) && narrowed.is_none_or(|kept| kept.contains(&trigger))
    }
}

/// The subtitle of an account entry: the scalar at `path` in the result of
/// `operation` (a declared `query` taking no input), such as a balance text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiEntrySummary {
    pub operation: String,
    pub path: String,
}
impl UiEntrySummary {
    pub fn validate(&self) -> Result<()> {
        if self.operation.is_empty() || self.operation.len() > 128 || !valid_path(&self.path) {
            return Err(ProtocolError::Denied("invalid UI entry summary"));
        }
        Ok(())
    }
    /// The subtitle in a result `data`: a non-blank scalar of at most 256
    /// characters without control characters.
    pub fn text(&self, data: &Value) -> Option<String> {
        data.pointer(&self.path)
            .and_then(scalar_text)
            .map(|text| {
                text.chars()
                    .filter(|c| !c.is_control())
                    .take(256)
                    .collect::<String>()
            })
            .filter(|text| !text.trim().is_empty())
    }
}
/// Whether a composer entry can be used now, as the plugin service says: the
/// boolean at `path` in the result of `operation` (a declared `query` taking
/// no input). For an entry whose capability depends on the deployment, such
/// as payments that need an identity service. Hosts list the entry only when
/// that value is `true`; a failed, slow or malformed answer hides it.
/// Presentation only: the plugin still refuses the invocation itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiEntryAvailability {
    pub operation: String,
    pub path: String,
}
impl UiEntryAvailability {
    pub fn validate(&self) -> Result<()> {
        if self.operation.is_empty() || self.operation.len() > 128 || !valid_path(&self.path) {
            return Err(ProtocolError::Denied("invalid UI entry availability"));
        }
        Ok(())
    }
    /// Whether the result `data` admits the entry: only a boolean `true` at
    /// `path` does.
    pub fn admits(&self, data: &Value) -> bool {
        data.pointer(&self.path) == Some(&Value::Bool(true))
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiField {
    pub label: String,
    pub path: String,
    /// Display labels for enumerated values (a status shown as words, not as its wire value).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiActionStyle {
    Primary,
    Danger,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiAction {
    pub id: String,
    pub label: String,
    pub target: UiActionTarget,
    #[serde(default)]
    pub input: BTreeMap<String, String>,
    /// Presentation only: the action is omitted when any condition matches. The
    /// plugin still authorizes every invocation; hiding is never a permission.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
    /// Ids of document inputs the host collects first (`0..=16`, operation
    /// targets only); their validated values join the action input under their
    /// ids, which must not repeat a declared input name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form: Vec<String>,
    /// Asked before invoking (translated through `texts`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<UiActionStyle>,
    /// Where the host shows the action: the button row (default) or the
    /// card's overflow menu (⋯). A resolved view lists them apart
    /// ([`ResolvedPluginView::actions`], [`ResolvedPluginView::menu_actions`]);
    /// `hiddenWhen` applies to both. An action a block owns (a choices
    /// submit, a row action) is never placed in the menu.
    #[serde(default, skip_serializing_if = "UiActionPlacement::is_buttons")]
    pub placement: UiActionPlacement,
    /// Page commands persist before dispatch. Retry/ack use the original
    /// authenticated journal record; a host cannot replace an unknown result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<UiPageCommand>,
    /// Navigate only after a verified successful operation response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<UiPageTransition>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPageCommand {
    pub slot: String,
    #[serde(default)]
    pub mode: UiPageCommandMode,
    #[serde(default)]
    pub terminal_error_codes: Vec<String>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiPageCommandMode {
    #[default]
    Submit,
    Retry,
    Acknowledge,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPageTransition {
    pub page: String,
    /// Execute this declared, form-free durable action after the initiating
    /// operation succeeds. Never executed when a page is read or opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub input: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<UiCondition>,
}
impl UiPageTransition {
    pub fn resolve(&self, data: &Value) -> Result<Option<Value>> {
        self.validate()?;
        if self.when.as_ref().is_some_and(|c| !c.matches(data, "")) {
            return Ok(None);
        }
        let mut target = serde_json::json!({"kind":"page","page":self.page});
        if let Some(id) = &self.plugin_id {
            target["pluginId"] = Value::from(id.clone());
        }
        Ok(Some(
            serde_json::json!({"kind":"host","target":target,"payload":bind_inputs(&self.input,data)?}),
        ))
    }
    pub fn validate(&self) -> Result<()> {
        if self.action_id.as_deref().is_some_and(|id| !valid_name(id))
            || self.page.is_empty()
            || self.page.len() > 160
            || !valid_bindings(&self.input)
            || self
                .plugin_id
                .as_ref()
                .is_some_and(|p| p.is_empty() || p.len() > 128)
            || self.when.as_ref().is_some_and(|c| !c.is_valid())
        {
            return Err(ProtocolError::Denied("invalid page transition"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UiActionPlacement {
    #[default]
    Buttons,
    Menu,
}
impl UiActionPlacement {
    pub fn is_buttons(&self) -> bool {
        *self == Self::Buttons
    }
}
/// A data-only test on the data the document is resolved against (for a row
/// action, on the row). No expressions: a value list, the viewer, presence, or
/// all of a few such tests. Exactly one of `anyOf`, `viewer`, `exists` and
/// `allOf` is given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiCondition {
    /// The value `anyOf`, `viewer` and `exists` test; required with them and
    /// absent with `allOf`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Matches when the value (text, number or bool) is one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_of: Vec<String>,
    /// Matches when the viewer is the value, or an element of an array value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub viewer: bool,
    /// Matches when the value is present and not empty (null, "", [] and {}
    /// are empty; false and 0 are present).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exists: bool,
    /// Matches when every one of these conditions matches (`1..=4`, each with
    /// its own `path`). One level only: they may not hold `allOf` themselves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all_of: Vec<UiCondition>,
    /// Inverts the match. For `anyOf` and `viewer` a missing value never
    /// matches either way, so older payloads without the path keep the action;
    /// a negated `exists` matches a missing value; a negated `allOf` matches
    /// when not every condition matches (each still follows its own rules).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub negate: bool,
}
fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => Some(v.chars().take(1024).collect()),
        Value::Number(v) => Some(v.to_string()),
        Value::Bool(v) => Some(v.to_string()),
        _ => None,
    }
}
impl UiCondition {
    pub(crate) fn is_valid(&self) -> bool {
        self.is_valid_at(true)
    }
    fn is_valid_at(&self, top_level: bool) -> bool {
        let one_test = [
            !self.any_of.is_empty(),
            self.viewer,
            self.exists,
            !self.all_of.is_empty(),
        ]
        .into_iter()
        .filter(|test| *test)
        .count()
            == 1;
        if !one_test {
            return false;
        }
        if !self.all_of.is_empty() {
            return top_level
                && self.path.is_none()
                && self.all_of.len() <= MAX_UI_CONDITIONS
                && self.all_of.iter().all(|c| c.is_valid_at(false));
        }
        self.path.as_deref().is_some_and(valid_path)
            && self.any_of.len() <= 16
            && self.any_of.iter().all(|v| !v.is_empty() && v.len() <= 64)
    }
    pub(crate) fn matches(&self, data: &Value, viewer: &str) -> bool {
        if !self.all_of.is_empty() {
            return self.all_of.iter().all(|c| c.matches(data, viewer)) != self.negate;
        }
        let value = self
            .path
            .as_deref()
            .and_then(|path| data.pointer(path))
            .filter(|v| !v.is_null());
        if self.exists {
            let present = value.is_some_and(|value| match value {
                Value::String(s) => !s.is_empty(),
                Value::Array(items) => !items.is_empty(),
                Value::Object(object) => !object.is_empty(),
                _ => true,
            });
            return present != self.negate;
        }
        let Some(value) = value else {
            return false;
        };
        let hit = (!self.any_of.is_empty()
            && scalar_text(value).is_some_and(|text| self.any_of.contains(&text)))
            || (self.viewer
                && !viewer.is_empty()
                && match value {
                    Value::Array(items) => items.iter().any(|item| item.as_str() == Some(viewer)),
                    other => other.as_str() == Some(viewer),
                });
        hit != self.negate
    }
}
/// JSON pointer of at most 128 bytes with non-empty `[A-Za-z0-9_]` segments.
pub(crate) fn valid_path(path: &str) -> bool {
    path.len() <= 128
        && path.starts_with('/')
        && path
            .split('/')
            .skip(1)
            .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}
pub(crate) fn valid_text(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= MAX_UI_TEXT_BYTES && !s.chars().any(char::is_control)
}
pub(crate) fn valid_long_text(s: &str) -> bool {
    !s.trim().is_empty()
        && s.len() <= MAX_UI_LONG_TEXT_BYTES
        && !s.chars().any(|c| c.is_control() && c != '\n')
}
/// Action input names and input ids: `[A-Za-z0-9_]{1,64}`.
pub(crate) fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub(crate) fn valid_field(field: &UiField) -> bool {
    valid_text(&field.label)
        && valid_path(&field.path)
        && field.values.len() <= 32
        && field
            .values
            .iter()
            .all(|(value, label)| !value.is_empty() && value.len() <= 64 && valid_text(label))
}
fn valid_bindings(input: &BTreeMap<String, String>) -> bool {
    input.len() <= 32
        && input
            .iter()
            .all(|(name, path)| valid_name(name) && valid_path(path))
}
/// Exact locale first, then the same language (`en`, `en-GB` → `en-US`).
fn translation_for<'a>(
    translations: &'a BTreeMap<String, UiTranslation>,
    locale: &str,
) -> Option<&'a UiTranslation> {
    if let Some(exact) = translations.get(locale) {
        return Some(exact);
    }
    let language = |tag: &str| {
        tag.split(['-', '_'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    let wanted = language(locale);
    (!wanted.is_empty())
        .then(|| {
            translations
                .iter()
                .find(|(tag, _)| language(tag) == wanted)
                .map(|(_, t)| t)
        })
        .flatten()
}
/// `source` in the view language, through the translation's `texts`.
pub(crate) fn translate(translation: Option<&UiTranslation>, source: &str) -> String {
    translation
        .and_then(|t| t.texts.get(source))
        .map_or(source, String::as_str)
        .to_owned()
}
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum UiActionTarget {
    Operation {
        operation: String,
    },
    Page {
        page: String,
        #[serde(rename = "pluginId", default, skip_serializing_if = "Option::is_none")]
        plugin_id: Option<String>,
    },
    Native {
        capability: NativeCapability,
    },
}
kind_tagged! {
    UiActionTarget = "UiActionTarget", deny_unknown_fields {
        "operation" => Operation { operation: String },
        "page" => Page {
            page: String,
            #[serde(rename = "pluginId", default)]
            plugin_id: Option<String>,
        },
        "native" => Native { capability: NativeCapability },
    }
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum NativeCapability {
    SelectImage,
    SelectFile,
    Camera,
    Microphone,
    ClipboardWrite,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum UiConversationType {
    Single,
    Group,
}
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum UiGroupRole {
    Owner,
    Admin,
    Member,
}
/// Who sees a composer-action entry. An empty list admits everyone; roles apply
/// in group conversations only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiAudience {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conversation_types: Vec<UiConversationType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_roles: Vec<UiGroupRole>,
}
impl UiAudience {
    pub fn validate(&self) -> Result<()> {
        let types: BTreeSet<_> = self.conversation_types.iter().collect();
        let roles: BTreeSet<_> = self.group_roles.iter().collect();
        if types.len() != self.conversation_types.len()
            || roles.len() != self.group_roles.len()
            || (!self.group_roles.is_empty()
                && self.conversation_types == [UiConversationType::Single])
        {
            return Err(ProtocolError::Denied("invalid UI audience"));
        }
        Ok(())
    }
    /// Whether the entry is shown in a conversation of `conversation` type to a
    /// viewer with `role` there (`None` when the host does not know it; a group
    /// entry restricted by role is then hidden).
    pub fn admits(&self, conversation: UiConversationType, role: Option<UiGroupRole>) -> bool {
        (self.conversation_types.is_empty() || self.conversation_types.contains(&conversation))
            && (conversation != UiConversationType::Group
                || self.group_roles.is_empty()
                || role.is_some_and(|role| self.group_roles.contains(&role)))
    }
}

/// The "view all" document of a message-content contribution or of an admin
/// page (`admin.pages`).
/// The host invokes `operation` with `input` bound from the card data (or, on
/// an admin page, from the row), then resolves `document` against the result
/// `data` and shows it in a sheet. A detail document has no rows block and no
/// detail of its own.
///
/// Paging (`cursorPath`): the first page is requested with the bound input
/// alone; the resolved view carries the cursor of the next page as
/// `nextCursor`, and the host requests that page with the same input plus
/// `cursor` ([`Self::page_input`]), resolves it against the same document and
/// appends it ([`ResolvedPluginView::append_page`]). No `nextCursor` means
/// the last page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiDetail {
    /// Wire operation declared by the plugin.
    pub operation: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input: BTreeMap<String, String>,
    /// The detail is not offered while any condition matches (`0..=4`), read
    /// on the data the contribution's document is resolved against: the
    /// merged card data, or an admin page's source result (not a row). The
    /// resolved view then has `detail: false`, entries blocks report no
    /// `more` (as without a detail), and rows carry no `detailInput`.
    /// Presentation only: the detail operation still authorizes every call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_when: Vec<UiCondition>,
    /// The cursor of the next page in the detail result: a non-blank scalar of
    /// at most 256 bytes without control characters; anything else, or no
    /// value, ends paging. When set, `input` may not bind `cursor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_path: Option<String>,
    pub document: UiDocument,
}
/// A page cursor a host may send back: non-blank, at most 256 bytes, no
/// control characters.
fn valid_page_cursor(cursor: &str) -> bool {
    !cursor.trim().is_empty()
        && cursor.len() <= MAX_ROWS_CURSOR_BYTES
        && !cursor.chars().any(char::is_control)
}
impl UiDetail {
    pub fn validate(&self) -> Result<()> {
        if self.operation.is_empty()
            || self.operation.len() > 128
            || !valid_bindings(&self.input)
            || !blocks::valid_conditions(&self.hidden_when)
            || self.document.rows().is_some()
            || self.cursor_path.as_deref().is_some_and(|path| {
                !valid_path(path) || self.input.contains_key(DETAIL_CURSOR_INPUT)
            })
        {
            return Err(ProtocolError::Denied("invalid UI detail"));
        }
        self.document.validate()
    }
    /// The detail operation input, bound from `data`.
    pub fn bind_input(&self, data: &Value) -> Result<Value> {
        bind_inputs(&self.input, data)
    }
    /// The input of the page after `cursor` (a resolved view's `nextCursor`):
    /// `bound`, the first page's input ([`Self::bind_input`], or an admin
    /// row's `detailInput`), plus `cursor`. Only for a detail with
    /// `cursorPath`; the first page is requested without `cursor`.
    pub fn page_input(&self, bound: Value, cursor: &str) -> Result<Value> {
        let Value::Object(mut input) = bound else {
            return Err(ProtocolError::Denied("invalid UI detail page"));
        };
        if self.cursor_path.is_none()
            || !valid_page_cursor(cursor)
            || input.contains_key(DETAIL_CURSOR_INPUT)
        {
            return Err(ProtocolError::Denied("invalid UI detail page"));
        }
        input.insert(DETAIL_CURSOR_INPUT.into(), Value::from(cursor));
        bounded_input(input)
    }
    /// The next-page cursor in a detail result `data`, when it is one.
    pub(crate) fn next_cursor(&self, data: &Value) -> Option<String> {
        data.pointer(self.cursor_path.as_deref()?)
            .and_then(scalar_text)
            .filter(|cursor| valid_page_cursor(cursor))
    }
    /// Whether `hiddenWhen` hides the detail on a document resolved against
    /// `data` for `viewer`.
    pub fn is_hidden(&self, data: &Value, viewer: &str) -> bool {
        self.hidden_when.iter().any(|c| c.matches(data, viewer))
    }
}

/// The data a `page` contribution is resolved against: the result `data` of
/// a declared `query` operation. A page is opened with arguments, the input
/// the opening action bound (a page target's payload; `{}` from an account
/// entry). `input` names operation inputs and the JSON pointers into those
/// arguments they are read from; one whose pointer finds nothing is left
/// out, so an argument is optional.
///
/// Paging follows the detail convention ([`UiDetail`]): with `cursorPath`
/// the resolved page carries `nextCursor`, the next page is requested with
/// the same input plus `cursor` ([`Self::page_input`]) and appended with
/// [`ResolvedPluginView::append_page`]. A page without a source resolves
/// against its arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiPageSource {
    /// Wire operation declared by the plugin with `mode: "query"`.
    pub operation: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub input: BTreeMap<String, String>,
    /// The cursor of the next page in the result, as for a detail. When set,
    /// `input` may not bind `cursor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_path: Option<String>,
    /// SDK-local journal projection, merged under `journal` after the source
    /// query. Scoped to the signed-in tenant/user; never supplied by a server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_slot: Option<String>,
}
impl UiPageSource {
    pub fn validate(&self) -> Result<()> {
        if self.operation.is_empty()
            || self.operation.len() > 128
            || !valid_bindings(&self.input)
            || self.input.contains_key(DETAIL_CURSOR_INPUT)
            || self
                .command_slot
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 128)
            || self
                .cursor_path
                .as_deref()
                .is_some_and(|path| !valid_path(path))
        {
            return Err(ProtocolError::Denied("invalid UI page source"));
        }
        Ok(())
    }
    /// The first page's input: each declared input whose pointer finds a
    /// value in `arguments`, plus `filters`, the validated filter values of
    /// the page's rows block ([`UiDocument::validate_inputs`]). A filter may
    /// not repeat an input name or `cursor`.
    pub fn bind_input(&self, arguments: &Value, filters: Map<String, Value>) -> Result<Value> {
        let mut input: Map<String, Value> = self
            .input
            .iter()
            .filter_map(|(name, path)| Some((name.clone(), arguments.pointer(path)?.clone())))
            .collect();
        for (name, value) in filters {
            if name == DETAIL_CURSOR_INPUT || input.contains_key(&name) {
                return Err(ProtocolError::Denied("UI input collision"));
            }
            input.insert(name, value);
        }
        bounded_input(input)
    }
    /// The input of the page after `cursor` (a resolved page's `nextCursor`):
    /// `bound`, the first page's input ([`Self::bind_input`]), plus `cursor`.
    /// Only for a source with `cursorPath`.
    pub fn page_input(&self, bound: Value, cursor: &str) -> Result<Value> {
        let Value::Object(mut input) = bound else {
            return Err(ProtocolError::Denied("invalid UI page request"));
        };
        if self.cursor_path.is_none()
            || !valid_page_cursor(cursor)
            || input.contains_key(DETAIL_CURSOR_INPUT)
        {
            return Err(ProtocolError::Denied("invalid UI page request"));
        }
        input.insert(DETAIL_CURSOR_INPUT.into(), Value::from(cursor));
        bounded_input(input)
    }
    /// The next-page cursor in a source result `data`, when it is one.
    fn next_cursor(&self, data: &Value) -> Option<String> {
        data.pointer(self.cursor_path.as_deref()?)
            .and_then(scalar_text)
            .filter(|cursor| valid_page_cursor(cursor))
    }
}

impl UiDocument {
    /// The title in `locale`, with the same language fallback as resolved views
    /// (exact tag, then same language); the declared title otherwise.
    pub fn localized_title(&self, locale: &str) -> &str {
        translation_for(&self.translations, locale).map_or(&self.title, |t| &t.title)
    }
    pub fn validate(&self) -> Result<()> {
        if !valid_text(&self.title)
            || self.fields.len() > MAX_UI_FIELDS
            || self.actions.len() > MAX_UI_ACTIONS
            || self.blocks.len() > MAX_UI_BLOCKS
            || self.inputs.len() > MAX_UI_INPUTS
        {
            return Err(ProtocolError::Denied("invalid UI document bounds"));
        }
        if let Some(p) = &self.presentation
            && (p.renderer.is_empty()
                || p.renderer.len() > 96
                || !p
                    .renderer
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || !valid_bindings(&p.bindings)
                || p.actions.len() > 16
                || p.labels.len() > 32
                || p.labels.iter().any(|(name, text)| {
                    !valid_name(name) || !valid_text(text) || p.bindings.contains_key(name)
                })
                || p.actions.iter().any(|(name, id)| {
                    !valid_name(name) || !self.actions.iter().any(|a| a.id == *id)
                }))
        {
            return Err(ProtocolError::Denied("invalid UI presentation"));
        }
        if self.translations.len() > 32
            || self.translations.iter().any(|(locale, t)| {
                locale.is_empty()
                    || locale.len() > 32
                    || !locale
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || !valid_text(&t.title)
                    || t.fields.len() != self.fields.len()
                    || t.fields.iter().any(|label| !valid_text(label))
                    || t.actions.len() > self.actions.len()
                    || t.actions.iter().any(|(id, label)| {
                        !valid_text(label) || !self.actions.iter().any(|a| &a.id == id)
                    })
                    || t.values.len() > self.fields.len()
                    || t.values.iter().any(|(path, labels)| {
                        !self
                            .fields
                            .iter()
                            .any(|f| &f.path == path && !f.values.is_empty())
                            || labels.len() > 32
                            || labels.iter().any(|(value, label)| {
                                value.is_empty() || value.len() > 64 || !valid_text(label)
                            })
                    })
                    || t.texts.len() > MAX_UI_TRANSLATED_TEXTS
                    || t.texts
                        .iter()
                        .any(|(source, text)| !valid_long_text(source) || !valid_long_text(text))
            })
        {
            return Err(ProtocolError::Denied("invalid UI translation"));
        }
        if !self.fields.iter().all(valid_field) {
            return Err(ProtocolError::Denied("invalid UI field"));
        }
        let mut input_ids = BTreeSet::new();
        for input in &self.inputs {
            input.validate()?;
            if !input_ids.insert(input.id.as_str()) {
                return Err(ProtocolError::Denied("invalid UI input"));
            }
        }
        let mut ids = BTreeSet::new();
        for action in &self.actions {
            if !valid_bindings(&action.input) {
                return Err(ProtocolError::Denied("invalid UI action bindings"));
            }
            if action.hidden_when.len() > 4 || !action.hidden_when.iter().all(UiCondition::is_valid)
            {
                return Err(ProtocolError::Denied("invalid UI action condition"));
            }
            let mut form = BTreeSet::new();
            if action.form.len() > MAX_UI_INPUTS
                || action.form.iter().any(|id| {
                    !input_ids.contains(id.as_str())
                        || !form.insert(id)
                        || action.input.contains_key(id)
                })
                || (!action.form.is_empty()
                    && !matches!(action.target, UiActionTarget::Operation { .. }))
            {
                return Err(ProtocolError::Denied("invalid UI action form"));
            }
            if !valid_text(&action.label)
                || action.command.as_ref().is_some_and(|c| {
                    c.slot.is_empty()
                        || c.slot.len() > 128
                        || !c
                            .slot
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
                        || c.terminal_error_codes.len() > 32
                        || c.terminal_error_codes.iter().any(|code| {
                            code.is_empty()
                                || code.len() > 128
                                || !code.bytes().all(|b| {
                                    b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'
                                })
                        })
                        || !matches!(action.target, UiActionTarget::Operation { .. })
                })
                || action.continuation.as_ref().is_some_and(|t| {
                    t.validate().is_err()
                        || !matches!(action.target, UiActionTarget::Operation { .. })
                })
                || action.confirm.as_deref().is_some_and(|c| !valid_text(c))
                || action.id.is_empty()
                || action.id.len() > 64
                || !action
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                || !ids.insert(&action.id)
            {
                return Err(ProtocolError::Denied("invalid UI action"));
            }
        }
        let mut owned = BTreeSet::new();
        for block in &self.blocks {
            block.validate(&self.actions, &self.inputs)?;
            if !block.owned_actions().into_iter().all(|id| owned.insert(id)) {
                return Err(ProtocolError::Denied("invalid UI block"));
            }
        }
        if self
            .actions
            .iter()
            .any(|a| a.placement == UiActionPlacement::Menu && owned.contains(a.id.as_str()))
        {
            return Err(ProtocolError::Denied("invalid UI action placement"));
        }
        Ok(())
    }
    /// The rows block, if any (page and admin page documents hold at most one).
    pub fn rows(&self) -> Option<&UiRowsBlock> {
        self.blocks.iter().find_map(|block| match block {
            UiBlock::Rows(rows) => Some(rows),
            _ => None,
        })
    }
    pub(crate) fn rows_sources(&self) -> impl Iterator<Item = &str> {
        self.blocks.iter().filter_map(|block| match block {
            UiBlock::Rows(rows) => Some(rows.source.as_str()),
            _ => None,
        })
    }
    /// The choices block that owns `action_id`, for its selection input.
    pub fn choices_for(&self, action_id: &str) -> Option<&UiChoicesBlock> {
        self.blocks.iter().find_map(|block| match block {
            UiBlock::Choices(choices) if choices.action.as_deref() == Some(action_id) => {
                Some(choices)
            }
            _ => None,
        })
    }
    fn owned_actions(&self) -> BTreeSet<&str> {
        self.blocks
            .iter()
            .flat_map(UiBlock::owned_actions)
            .collect()
    }
    /// The inputs of `action`'s form, in form order.
    pub fn form_inputs(&self, action: &UiAction) -> Vec<&UiInput> {
        action
            .form
            .iter()
            .filter_map(|id| self.inputs.iter().find(|input| &input.id == id))
            .collect()
    }
    /// Validate the form values submitted for `action` (see [`validate_form_values`]).
    pub fn validate_form(
        &self,
        action: &UiAction,
        values: &Value,
    ) -> std::result::Result<Map<String, Value>, Vec<UiFormError>> {
        validate_form_values(&self.form_inputs(action), values)
    }
    /// [`Self::validate_form`] against the inputs as resolved over `data`, the
    /// data the document is shown with: `maxPath` and `defaultPath` read
    /// there, as in the form the host rendered. A composer-action form passes
    /// [`composer_data`].
    pub fn validate_form_over(
        &self,
        action: &UiAction,
        data: &Value,
        values: &Value,
    ) -> std::result::Result<Map<String, Value>, Vec<UiFormError>> {
        let inputs: Vec<UiInput> = self
            .form_inputs(action)
            .into_iter()
            .map(|input| input.resolve(data, None))
            .collect();
        validate_form_values(&inputs.iter().collect::<Vec<_>>(), values)
    }
    /// The filter inputs of the rows block (admin page or page), in document
    /// order: those it lists in `filters`, or every input when it declares no
    /// `filters`.
    pub fn filter_inputs(&self) -> Vec<&UiInput> {
        match self.rows().and_then(|rows| rows.filters.as_ref()) {
            Some(filters) => self
                .inputs
                .iter()
                .filter(|input| filters.contains(&input.id))
                .collect(),
            None => self.inputs.iter().collect(),
        }
    }
    /// Validate rows filter values: the [`Self::filter_inputs`] only. A
    /// value for any other input (a row action's form) is `unknown` here.
    pub fn validate_inputs(
        &self,
        values: &Value,
    ) -> std::result::Result<Map<String, Value>, Vec<UiFormError>> {
        validate_form_values(&self.filter_inputs(), values)
    }
    /// Drop the actions whose target `keep` rejects together with every
    /// reference to them, and a rows block whose source it rejects. The
    /// document stays valid, so a host can trim it to what a viewer may invoke.
    /// Keep named actions and prune all references to removed actions.
    pub fn retain_actions(&mut self, keep: &mut dyn FnMut(&str) -> bool) {
        self.actions.retain(|action| keep(&action.id));
        self.retain_targets(&mut |_| true);
    }

    pub fn retain_targets(&mut self, keep: &mut dyn FnMut(&UiActionTarget) -> bool) {
        self.actions.retain(|action| keep(&action.target));
        let ids: BTreeSet<String> = self.actions.iter().map(|a| a.id.clone()).collect();
        if let Some(p) = &mut self.presentation {
            p.actions.retain(|_, id| ids.contains(id));
        }
        self.blocks.retain_mut(|block| match block {
            UiBlock::Choices(choices) => {
                if choices.action.as_ref().is_some_and(|id| !ids.contains(id)) {
                    choices.action = None;
                }
                true
            }
            UiBlock::Rows(rows) => {
                rows.actions.retain(|id| ids.contains(id));
                keep(&UiActionTarget::Operation {
                    operation: rows.source.clone(),
                })
            }
            _ => true,
        });
        for translation in self.translations.values_mut() {
            translation.actions.retain(|id, _| ids.contains(id));
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientPlugin {
    pub plugin_id: String,
    pub version: String,
    pub manifest_digest: String,
    pub grant_revision: u64,
    pub extensions: ClientExtensions,
    pub operations: BTreeSet<String>,
    /// The client-visible subset of the tenant configuration
    /// ([`crate::PluginManifest::client_visible_configuration`]).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub configuration: Map<String, Value>,
}
/// The data a composer-action document is resolved against, and its form
/// validated and its action input bound over: `{"config": configuration}`,
/// the plugin's client-visible configuration under [`COMPOSER_CONFIG_KEY`]
/// (`{"config": {}}` when there is none). So an input's `defaultPath` or
/// `maxPath` reads `/config/<key>`, and falls back to the declared value while
/// the tenant has not configured that key. No message payload or state layer
/// takes part.
pub fn composer_data(configuration: &Map<String, Value>) -> Value {
    let mut data = Map::new();
    data.insert(
        COMPOSER_CONFIG_KEY.into(),
        Value::Object(configuration.clone()),
    );
    Value::Object(data)
}
impl ClientPlugin {
    /// [`composer_data`] of this plugin's configuration.
    pub fn composer_data(&self) -> Value {
        composer_data(&self.configuration)
    }
    /// The data `contribution` (one of this plugin's) is resolved against:
    /// [`Self::composer_data`] for a composer-action, otherwise the payload
    /// under its state layers ([`UiStateLayers::merge`]).
    pub fn document_data(
        &self,
        contribution: &UiContribution,
        payload: &Value,
        layers: UiStateLayers<'_>,
    ) -> Value {
        if contribution.kind == UiContributionKind::ComposerAction {
            self.composer_data()
        } else {
            layers.merge(contribution, payload)
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiCatalog {
    pub tenant_id: String,
    pub version: u64,
    pub valid_until_ms: i64,
    pub plugins: Vec<ClientPlugin>,
    /// Plugins of this tenant the viewer cannot use, with the reason, so a
    /// card of one says why it is inert ("plugin disabled") instead of
    /// nothing. A plugin in neither list resolves as `not_installed`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable: Vec<UiUnavailablePlugin>,
}
/// A plugin listed in [`UiCatalog::unavailable`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiUnavailablePlugin {
    pub plugin_id: String,
    pub reason: UiUnavailableReason,
}
/// Why a plugin view is inert ([`ResolvedPluginView::unavailable_reason`]).
/// Kits show a reason text and the envelope's
/// [`PluginMessage::display_name`] with the summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UiUnavailableReason {
    /// Installed but disabled for the tenant.
    Disabled,
    /// Not installed for the tenant (or uninstalled).
    NotInstalled,
    /// Installed and enabled, but not for this viewer.
    NoPermission,
    /// The plugin does not support this client platform.
    UnsupportedPlatform,
    /// The card's contribution, schema version or document is not one the
    /// installed version (or this client) understands.
    IncompatibleVersion,
    /// The catalog could not be read; the card shows its summary until it can.
    CatalogUnreachable,
    /// A copy of a card that is not the one bound to its entity (forged or
    /// forwarded): its content is shown, read-only. The only reason with
    /// `available: true`.
    UnboundCopy,
}
/// A plugin card in a conversation. Besides cards the host builds itself, an
/// operation result may carry one as `data.message` (see [`Self::from_result`]):
/// the host then sends it to the conversation the action ran in as a plugin
/// message, with a client message id that is stable for that action, and every
/// receiver renders it through the plugin's message-content contribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginMessage {
    pub plugin_id: String,
    pub contribution_id: String,
    pub schema_version: u32,
    pub summary: String,
    pub payload: Value,
    /// The plugin's name, written by the platform when it posts the card (in
    /// the tenant's default language), for inert cards to say whose card it
    /// is. Display text only, at most [`MAX_PLUGIN_DISPLAY_NAME_BYTES`] bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}
/// Bytes of [`PluginMessage::display_name`].
pub const MAX_PLUGIN_DISPLAY_NAME_BYTES: usize = 64;
impl PluginMessage {
    /// The message a successful operation result of `plugin_id` asks the host to
    /// post: `data.message`, only when it is a valid message of that same plugin.
    /// Anything else (absent, malformed, another plugin's card) posts nothing.
    pub fn from_result(plugin_id: &str, data: &Value) -> Option<Self> {
        let message: Self = serde_json::from_value(data.get("message")?.clone()).ok()?;
        (message.plugin_id == plugin_id && message.validate().is_ok()).then_some(message)
    }
    pub fn validate(&self) -> Result<()> {
        let identifier = |id: &str| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        };
        if !identifier(&self.plugin_id)
            || self.plugin_id.len() > 128
            || !identifier(&self.contribution_id)
            || self.contribution_id.len() > 160
            || !self
                .contribution_id
                .starts_with(&format!("{}.", self.plugin_id))
            || self.schema_version != 1
            || self.summary.len() > 1024
            || self.summary.chars().any(char::is_control)
            || self.display_name.as_deref().is_some_and(|name| {
                name.trim().is_empty()
                    || name.len() > MAX_PLUGIN_DISPLAY_NAME_BYTES
                    || name.chars().any(char::is_control)
            })
            || !self.payload.is_object()
            || serde_json::to_vec(self).map_or(true, |v| v.len() > 65536)
        {
            return Err(ProtocolError::Denied("invalid plugin message"));
        }
        Ok(())
    }
}
/// The viewer's own state of a card: what a plugin returns for the message an
/// action came from (`data.messageState`), and what a contribution's
/// [`UiViewerState`] read returns. Clients keep the highest version per
/// message and overlay `values` on the payload before resolving: it is the
/// local layer of [`UiStateLayers`]. It grants nothing: every action is still
/// authorized by the plugin.
///
/// At most 16 keys `[A-Za-z0-9_]{1,64}` and 4 KiB; values are scalars (strings
/// up to 1024 bytes, such as the viewer's own entry text), or arrays of up to
/// 100 strings (each up to 256 bytes) or of up to 100 numbers (such as the
/// indices of `mySelection`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginMessageState {
    pub version: u64,
    /// The shared-state version (of the card's entity) these values were
    /// computed from. While it is at least the applied shared version, local
    /// keys replace shared keys of the same name; once the shared layer is
    /// newer they do not (see [`UiStateLayers`]). Absent counts as 0: stale
    /// against any shared layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on_version: Option<u64>,
    pub values: Map<String, Value>,
}
impl PluginMessageState {
    pub const MAX_KEYS: usize = 16;
    pub const MAX_BYTES: usize = 4096;
    /// Bytes of a string value.
    pub const MAX_STRING_BYTES: usize = 1024;
    /// Bytes of a string inside an array value.
    pub const MAX_ITEM_BYTES: usize = 256;
    pub fn validate(&self) -> Result<()> {
        let text = |s: &str, max: usize| s.len() <= max && !s.chars().any(char::is_control);
        let valid_value = |value: &Value| match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => true,
            Value::String(s) => text(s, Self::MAX_STRING_BYTES),
            Value::Array(items) => {
                items.len() <= 100
                    && (items
                        .iter()
                        .all(|i| i.as_str().is_some_and(|s| text(s, Self::MAX_ITEM_BYTES)))
                        || items.iter().all(Value::is_number))
            }
            Value::Object(_) => false,
        };
        if self.values.len() > Self::MAX_KEYS
            || self.based_on_version.is_some_and(|v| v > i64::MAX as u64)
            || self
                .values
                .iter()
                .any(|(key, value)| !crate::state::valid_state_key(key) || !valid_value(value))
            || serde_json::to_vec(self).map_or(true, |v| v.len() > Self::MAX_BYTES)
        {
            return Err(ProtocolError::Denied("invalid plugin message state"));
        }
        Ok(())
    }
    /// Whether these values were computed from `shared_version` or a newer
    /// one (always, when there is no shared layer: version 0).
    pub fn is_fresh_against(&self, shared_version: u64) -> bool {
        self.based_on_version.unwrap_or(0) >= shared_version
    }
    /// This layer with `basedOnVersion` at most `latest_shared_version`, the
    /// newest shared version the caller knows of for the card. The version
    /// comes from the plugin; capped, a plugin cannot pin its local layer
    /// fresh against shared layers it has not published yet.
    pub fn capped_at(mut self, latest_shared_version: u64) -> Self {
        if let Some(based_on) = &mut self.based_on_version {
            *based_on = (*based_on).min(latest_shared_version);
        }
        self
    }
    /// Shallow overlay: a value replaces the top-level payload key, null removes it.
    pub fn merge_into(&self, payload: &Value) -> Value {
        crate::state::overlay(payload, &self.values)
    }
}
/// A resolved document, flat and ready to render: every value read, mapped and
/// translated, conditions applied. Kits render it without reading paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedPluginView {
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<ResolvedUiPresentation>,
    /// Why the view is inert (`available: false`), or `unbound_copy` for a
    /// read-only copy (`available: true`, no actions, menu, detail or block
    /// submits). Absent on an interactive view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<UiUnavailableReason>,
    pub title: String,
    pub fields: Vec<ResolvedUiField>,
    /// Visible button-row actions not owned by a block, labels and
    /// confirmations translated.
    pub actions: Vec<UiAction>,
    /// Visible actions placed in the overflow menu (⋯), in document order,
    /// resolved like `actions`. The host shows the menu only when this is
    /// not empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub menu_actions: Vec<UiAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<ResolvedUiBlock>,
    /// All document inputs, labels translated and initial values resolved; an
    /// action's `form` lists the ones it collects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<UiInput>,
    /// The contribution declares a detail ("view all") and its `hiddenWhen`
    /// does not match.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub detail: bool,
    /// A detail view of a paged detail (`cursorPath`): the cursor of the next
    /// page, absent on the last page (see [`UiDetail::page_input`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
impl ResolvedPluginView {
    /// Append the next page of a paged detail, resolved against the same
    /// detail document. Entries blocks pair up in order of appearance: the
    /// page's entries follow this view's in the same block, numbered on from
    /// this block's largest `index`, and `total` becomes the largest of the
    /// two totals and the merged count. In a grouped block a page that starts
    /// in the group this view ends with continues it: that first header is
    /// dropped (its count and total update the shown header) and its entries
    /// are numbered on within the group; a page starting a new group numbers
    /// from 1 as resolved. Rows blocks (a page's) pair up the same way: the
    /// page's rows follow, a row whose key is already shown is dropped, and
    /// the block's `nextCursor` becomes the page's. Everything else (title,
    /// fields, actions, menu, inputs, other blocks) stays as on the first
    /// page; `nextCursor` becomes the page's. An unavailable view or page
    /// ends paging without changing the entries or rows.
    pub fn append_page(&mut self, page: ResolvedPluginView) {
        if !self.available || !page.available {
            self.next_cursor = None;
            return;
        }
        self.next_cursor = page.next_cursor;
        let (mut entry_pages, mut row_pages) = (Vec::new(), Vec::new());
        for block in page.blocks {
            match block {
                ResolvedUiBlock::Entries(entries) => entry_pages.push(entries),
                ResolvedUiBlock::Rows(rows) => row_pages.push(rows),
                _ => {}
            }
        }
        let mut row_pages = row_pages.into_iter();
        for block in &mut self.blocks {
            if let ResolvedUiBlock::Rows(rows) = block
                && let Some(next) = row_pages.next()
            {
                let shown: BTreeSet<String> = rows.rows.iter().map(|r| r.key.clone()).collect();
                rows.rows.extend(
                    next.rows
                        .into_iter()
                        .filter(|row| !shown.contains(&row.key)),
                );
                rows.next_cursor = next.next_cursor;
            }
        }
        let mut pages = entry_pages.into_iter();
        for block in &mut self.blocks {
            let ResolvedUiBlock::Entries(entries) = block else {
                continue;
            };
            let Some(next) = pages.next() else {
                break;
            };
            if entries.grouped {
                entries.append_grouped(next.items);
            } else {
                let offset = entries
                    .items
                    .iter()
                    .map(|item| item.index)
                    .max()
                    .unwrap_or(0);
                entries.items.extend(next.items.into_iter().map(|mut item| {
                    item.index = item.index.saturating_add(offset);
                    item
                }));
            }
            entries.total = entries
                .total
                .max(next.total)
                .max(entries.items.len() as u64);
        }
    }

    /// The read-only form of a card that is not the one bound to its entity
    /// (a forged or forwarded copy): the content stays, everything that would
    /// call the plugin goes (actions, menu, inputs, detail, block submits,
    /// row actions), and `unavailableReason` is `unbound_copy`. An unavailable
    /// view is returned as it is.
    pub fn into_read_only(mut self) -> Self {
        if !self.available {
            return self;
        }
        self.unavailable_reason = Some(UiUnavailableReason::UnboundCopy);
        if let Some(p) = &mut self.presentation {
            p.actions.clear();
        }
        self.actions.clear();
        self.menu_actions.clear();
        self.inputs.clear();
        self.detail = false;
        for block in &mut self.blocks {
            match block {
                ResolvedUiBlock::Choices(choices) => choices.action = None,
                ResolvedUiBlock::Entries(entries) => entries.more = false,
                ResolvedUiBlock::Rows(rows) => {
                    rows.actions.clear();
                    for row in &mut rows.rows {
                        row.actions.clear();
                        row.detail_input = None;
                    }
                }
                _ => {}
            }
        }
        self
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedUiField {
    pub label: String,
    pub value: String,
}
/// Fields with a value, labels and values mapped by the given rules. A field
/// without a value is not shown: a card says what it knows, not an empty row.
pub(crate) fn resolve_fields(
    fields: &[UiField],
    data: &Value,
    label: impl Fn(&UiField, usize) -> String,
    value: impl Fn(&UiField, &str) -> Option<String>,
) -> Vec<ResolvedUiField> {
    fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| {
            let raw = data.pointer(&field.path).and_then(scalar_text)?;
            if raw.trim().is_empty() {
                return None;
            }
            Some(ResolvedUiField {
                label: label(field, index),
                value: value(field, &raw).unwrap_or(raw),
            })
        })
        .collect()
}
pub(crate) fn resolve_action(action: &UiAction, translation: Option<&UiTranslation>) -> UiAction {
    let mut action = action.clone();
    action.hidden_when.clear();
    if let Some(label) = translation.and_then(|t| t.actions.get(&action.id)) {
        action.label = label.clone();
    }
    action.confirm = action.confirm.map(|text| translate(translation, &text));
    action
}
pub(crate) fn resolve_document(
    document: &UiDocument,
    data: &Value,
    locale: &str,
    viewer: &str,
    detail: Option<&UiDetail>,
) -> ResolvedPluginView {
    let translation = translation_for(&document.translations, locale);
    let fields = resolve_fields(
        &document.fields,
        data,
        |field, index| {
            translation
                .and_then(|t| t.fields.get(index))
                .unwrap_or(&field.label)
                .clone()
        },
        |field, raw| {
            translation
                .and_then(|t| t.values.get(&field.path))
                .and_then(|labels| labels.get(raw))
                .or_else(|| field.values.get(raw))
                .cloned()
        },
    );
    let owned = document.owned_actions();
    let (menu_actions, actions): (Vec<UiAction>, Vec<UiAction>) = document
        .actions
        .iter()
        .filter(|action| {
            !owned.contains(action.id.as_str())
                && !action
                    .hidden_when
                    .iter()
                    .any(|condition| condition.matches(data, viewer))
        })
        .map(|action| resolve_action(action, translation))
        .partition(|action| action.placement == UiActionPlacement::Menu);
    let detail = detail.filter(|d| !d.is_hidden(data, viewer));
    let scope = blocks::Scope {
        data,
        viewer,
        translation,
        actions: &document.actions,
        detail,
    };
    ResolvedPluginView {
        available: true,
        presentation: document.presentation.as_ref().and_then(|p| {
            let mut props: BTreeMap<String, Value> = p
                .bindings
                .iter()
                .filter_map(|(name, path)| {
                    data.pointer(path)
                        .filter(|v| !v.is_null())
                        .map(|v| (name.clone(), v.clone()))
                })
                .collect();
            for (name, label) in &p.labels {
                let translated = translation
                    .and_then(|t| t.texts.get(label))
                    .unwrap_or(label);
                props.insert(name.clone(), Value::String(translated.clone()));
            }
            // Keep display projections bounded, including arrays and strings.
            if serde_json::to_vec(&props).ok()?.len() > MAX_UI_PRESENTATION_BYTES {
                return None;
            }
            Some(ResolvedUiPresentation {
                renderer: p.renderer.clone(),
                props,
                actions: p
                    .actions
                    .iter()
                    .filter(|(_, id)| {
                        actions
                            .iter()
                            .chain(menu_actions.iter())
                            .any(|a| a.id == **id)
                    })
                    .map(|(name, id)| (name.clone(), id.clone()))
                    .collect(),
            })
        }),
        unavailable_reason: None,
        title: translation.map_or(&document.title, |t| &t.title).clone(),
        fields,
        actions,
        menu_actions,
        blocks: document
            .blocks
            .iter()
            .filter_map(|block| block.resolve(&scope))
            .collect(),
        inputs: document
            .inputs
            .iter()
            .map(|input| input.resolve(data, translation))
            .collect(),
        detail: detail.is_some(),
        next_cursor: None,
    }
}
impl UiContribution {
    /// Resolve the contribution's document against `data`: the merged card data
    /// ([`UiStateLayers::merge`]), a composer form's [`composer_data`], or a
    /// page's source result.
    pub fn resolve(&self, data: &Value, locale: &str, viewer: &str) -> ResolvedPluginView {
        resolve_document(&self.schema, data, locale, viewer, self.detail.as_ref())
    }
    /// Resolve the detail document against the detail operation's result data
    /// (one page of it, for a paged detail: the view then has `nextCursor`).
    pub fn resolve_detail(
        &self,
        data: &Value,
        locale: &str,
        viewer: &str,
    ) -> Option<ResolvedPluginView> {
        let detail = self.detail.as_ref()?;
        let mut view = resolve_document(&detail.document, data, locale, viewer, None);
        view.next_cursor = detail.next_cursor(data);
        Some(view)
    }
    /// Resolve a `page` contribution's document against `data`: its source
    /// result (one page of it, for a paged source: the view then has
    /// `nextCursor`), or its arguments when it declares no source. A page
    /// has no detail, so its entries report no `more` and its rows carry no
    /// `detailInput`. `None` for any other kind.
    pub fn resolve_page(
        &self,
        data: &Value,
        locale: &str,
        viewer: &str,
    ) -> Option<ResolvedPluginView> {
        if self.kind != UiContributionKind::Page {
            return None;
        }
        let mut view = resolve_document(&self.schema, data, locale, viewer, None);
        view.next_cursor = self.source.as_ref().and_then(|s| s.next_cursor(data));
        Some(view)
    }
    /// [`UiDocument::retain_targets`] over the document and the detail; the
    /// detail is dropped when `keep` rejects its operation.
    pub fn retain_targets(&mut self, mut keep: impl FnMut(&UiActionTarget) -> bool) {
        self.schema.retain_targets(&mut keep);
        if self.detail.as_ref().is_some_and(|detail| {
            !keep(&UiActionTarget::Operation {
                operation: detail.operation.clone(),
            })
        }) {
            self.detail = None;
        }
        if let Some(detail) = &mut self.detail {
            detail.document.retain_targets(&mut keep);
        }
    }
}
impl ResolvedPluginView {
    /// The inert view of `message`: its summary as the title (inert text, never
    /// a source of actions) and `reason`.
    pub fn unavailable(message: &PluginMessage, reason: UiUnavailableReason) -> Self {
        Self::inert(
            message
                .summary
                .chars()
                .filter(|c| !c.is_control())
                .take(256)
                .collect(),
            reason,
        )
    }
    /// The inert view of a page that cannot be shown: no content, an empty
    /// title (the host keeps its own page title) and `reason`.
    pub fn unavailable_page(reason: UiUnavailableReason) -> Self {
        Self::inert(String::new(), reason)
    }
    fn inert(title: String, reason: UiUnavailableReason) -> Self {
        Self {
            available: false,
            presentation: None,
            unavailable_reason: Some(reason),
            title,
            fields: vec![],
            actions: vec![],
            menu_actions: vec![],
            blocks: vec![],
            inputs: vec![],
            detail: false,
            next_cursor: None,
        }
    }
}
impl UiCatalog {
    /// Summary is inert text from message history. Never derive actions from it
    /// or from the historical payload, even when a newer plugin is installed.
    pub fn resolve_message(
        &self,
        tenant: &str,
        platform: ClientPlatform,
        message: &PluginMessage,
        now_ms: i64,
    ) -> ResolvedPluginView {
        self.resolve_with_context(
            message,
            UiRenderContext {
                tenant,
                platform,
                kind: UiContributionKind::MessageContent,
                locale: "",
                viewer: "",
                now_ms,
            },
        )
    }
    /// [`Self::resolve_with_state`] without state layers: the payload as sent.
    pub fn resolve_with_context(
        &self,
        message: &PluginMessage,
        context: UiRenderContext<'_>,
    ) -> ResolvedPluginView {
        self.resolve_with_state(message, UiStateLayers::default(), context)
    }
    /// The view of a plugin message over its state layers
    /// (`payload ← shared ← local`); a composer-action form resolves against
    /// the plugin's [`composer_data`] instead ([`ClientPlugin::document_data`]).
    /// Anything that cannot be rendered resolves to the inert summary with
    /// `available: false` and its [`UiUnavailableReason`] (see
    /// [`Self::unavailable_reason`]).
    pub fn resolve_with_state(
        &self,
        message: &PluginMessage,
        state: UiStateLayers<'_>,
        context: UiRenderContext<'_>,
    ) -> ResolvedPluginView {
        match self.contribution_for(message, &context) {
            Ok((plugin, contribution)) => contribution.resolve(
                &plugin.document_data(contribution, &message.payload, state),
                context.locale,
                context.viewer,
            ),
            Err(reason) => ResolvedPluginView::unavailable(message, reason),
        }
    }
    /// The detail view of a message, resolved against the result `data` of the
    /// detail operation ([`Self::detail`]).
    pub fn resolve_detail(
        &self,
        message: &PluginMessage,
        data: &Value,
        context: UiRenderContext<'_>,
    ) -> ResolvedPluginView {
        match self.contribution_for(message, &context) {
            Ok((_, c)) => c
                .resolve_detail(data, context.locale, context.viewer)
                .unwrap_or_else(|| {
                    ResolvedPluginView::unavailable(
                        message,
                        UiUnavailableReason::IncompatibleVersion,
                    )
                }),
            Err(reason) => ResolvedPluginView::unavailable(message, reason),
        }
    }
    /// Why `message` cannot be rendered in `context`, or `None` when it can:
    /// - another tenant's message, or a plugin in neither `plugins` nor
    ///   `unavailable`: `not_installed`;
    /// - a plugin in `unavailable`: its reason;
    /// - an expired catalog: `catalog_unreachable` (the host reads a fresh one);
    /// - a plugin that does not list the platform: `unsupported_platform`;
    /// - an invalid envelope, an unknown contribution, schema version or kind,
    ///   or an invalid document: `incompatible_version`.
    pub fn unavailable_reason(
        &self,
        message: &PluginMessage,
        context: &UiRenderContext<'_>,
    ) -> Option<UiUnavailableReason> {
        self.contribution_for(message, context).err()
    }
    fn contribution_for(
        &self,
        message: &PluginMessage,
        context: &UiRenderContext<'_>,
    ) -> std::result::Result<(&ClientPlugin, &UiContribution), UiUnavailableReason> {
        if self.tenant_id != context.tenant {
            return Err(UiUnavailableReason::NotInstalled);
        }
        if self.valid_until_ms <= context.now_ms {
            return Err(UiUnavailableReason::CatalogUnreachable);
        }
        if message.validate().is_err() {
            return Err(UiUnavailableReason::IncompatibleVersion);
        }
        let Some(plugin) = self
            .plugins
            .iter()
            .find(|p| p.plugin_id == message.plugin_id)
        else {
            return Err(self
                .unavailable
                .iter()
                .find(|p| p.plugin_id == message.plugin_id)
                .map_or(UiUnavailableReason::NotInstalled, |p| p.reason));
        };
        if !plugin.extensions.platforms.contains(&context.platform) {
            return Err(UiUnavailableReason::UnsupportedPlatform);
        }
        plugin
            .extensions
            .contributions
            .iter()
            .find(|c| {
                c.id == message.contribution_id
                    && c.schema_version == message.schema_version
                    && c.kind == context.kind
            })
            .filter(|contribution| {
                contribution.schema.validate().is_ok()
                    && contribution
                        .detail
                        .as_ref()
                        .is_none_or(|detail| detail.validate().is_ok())
            })
            .map(|contribution| (plugin, contribution))
            .ok_or(UiUnavailableReason::IncompatibleVersion)
    }
    /// This validates declaration and host consent, not server-side authorization.
    /// Invoke again through Core; it checks current grants, flags and actor policy.
    #[allow(clippy::too_many_arguments)]
    pub fn action(
        &self,
        tenant: &str,
        plugin_id: &str,
        contribution_id: &str,
        action_id: &str,
        platform: ClientPlatform,
        native_allowlist: &BTreeSet<NativeCapability>,
        user_gesture: bool,
        now_ms: i64,
    ) -> Result<UiActionTarget> {
        if tenant != self.tenant_id || now_ms >= self.valid_until_ms || !user_gesture {
            return Err(ProtocolError::Denied("UI action unavailable"));
        }
        let p = self
            .plugins
            .iter()
            .find(|p| p.plugin_id == plugin_id && p.extensions.platforms.contains(&platform))
            .ok_or(ProtocolError::Denied("UI plugin unavailable"))?;
        let c = p
            .extensions
            .contributions
            .iter()
            .find(|c| c.id == contribution_id && c.schema_version == 1)
            .ok_or(ProtocolError::Denied("UI contribution unavailable"))?;
        c.schema.validate()?;
        let a = c
            .schema
            .actions
            .iter()
            .find(|a| a.id == action_id)
            .ok_or(ProtocolError::Denied("UI action unavailable"))?;
        match &a.target {
            UiActionTarget::Operation { operation } if !p.operations.contains(operation) => {
                return Err(ProtocolError::Denied("UI operation unavailable"));
            }
            UiActionTarget::Page { page, plugin_id }
                if !self
                    .plugins
                    .iter()
                    .filter(|candidate| {
                        candidate.plugin_id == plugin_id.as_deref().unwrap_or(&p.plugin_id)
                            && candidate.extensions.platforms.contains(&platform)
                    })
                    .any(|target| {
                        target
                            .extensions
                            .contributions
                            .iter()
                            .any(|c| &c.id == page && c.kind == UiContributionKind::Page)
                    }) =>
            {
                return Err(ProtocolError::Denied("UI page unavailable"));
            }
            UiActionTarget::Native { capability }
                if !native_allowlist.contains(capability)
                    || !p.extensions.native_capabilities.contains(capability) =>
            {
                return Err(ProtocolError::Denied("native capability unavailable"));
            }
            _ => {}
        }
        Ok(a.target.clone())
    }
    /// The detail of a contribution, when its operation is in the catalog. Like
    /// [`Self::action`] this checks declaration and gesture only; Core
    /// authorizes the invocation. Hosts offer it only while the resolved view
    /// has `detail: true` (see [`UiDetail::hidden_when`]).
    pub fn detail(
        &self,
        tenant: &str,
        plugin_id: &str,
        contribution_id: &str,
        platform: ClientPlatform,
        user_gesture: bool,
        now_ms: i64,
    ) -> Result<UiDetail> {
        if tenant != self.tenant_id || now_ms >= self.valid_until_ms || !user_gesture {
            return Err(ProtocolError::Denied("UI detail unavailable"));
        }
        let p = self
            .plugins
            .iter()
            .find(|p| p.plugin_id == plugin_id && p.extensions.platforms.contains(&platform))
            .ok_or(ProtocolError::Denied("UI plugin unavailable"))?;
        let detail = p
            .extensions
            .contributions
            .iter()
            .find(|c| c.id == contribution_id && c.schema_version == 1)
            .and_then(|c| c.detail.as_ref())
            .ok_or(ProtocolError::Denied("UI detail unavailable"))?;
        detail.validate()?;
        if !p.operations.contains(&detail.operation) {
            return Err(ProtocolError::Denied("UI operation unavailable"));
        }
        Ok(detail.clone())
    }
    /// The `page` contribution `page` of `plugin_id` with its plugin, or why
    /// it cannot be shown in `context` (its `kind` is ignored):
    /// - another tenant, or a plugin in neither `plugins` nor `unavailable`:
    ///   `not_installed`; a plugin in `unavailable`: its reason;
    /// - an expired catalog: `catalog_unreachable`;
    /// - a plugin that does not list the platform: `unsupported_platform`;
    /// - no such page, another schema version or an invalid document or
    ///   source: `incompatible_version`;
    /// - a source operation the catalog does not offer the viewer:
    ///   `no_permission`.
    ///
    /// Like [`Self::detail`] this checks declaration only; Core authorizes
    /// the source call.
    pub fn page(
        &self,
        plugin_id: &str,
        page: &str,
        context: &UiRenderContext<'_>,
    ) -> std::result::Result<(&ClientPlugin, &UiContribution), UiUnavailableReason> {
        if self.tenant_id != context.tenant {
            return Err(UiUnavailableReason::NotInstalled);
        }
        if self.valid_until_ms <= context.now_ms {
            return Err(UiUnavailableReason::CatalogUnreachable);
        }
        let Some(plugin) = self.plugins.iter().find(|p| p.plugin_id == plugin_id) else {
            return Err(self
                .unavailable
                .iter()
                .find(|p| p.plugin_id == plugin_id)
                .map_or(UiUnavailableReason::NotInstalled, |p| p.reason));
        };
        if !plugin.extensions.platforms.contains(&context.platform) {
            return Err(UiUnavailableReason::UnsupportedPlatform);
        }
        let contribution = plugin
            .extensions
            .contributions
            .iter()
            .find(|c| c.id == page && c.kind == UiContributionKind::Page && c.schema_version == 1)
            .filter(|c| {
                c.schema.validate().is_ok()
                    && c.source.as_ref().is_none_or(|s| s.validate().is_ok())
            })
            .ok_or(UiUnavailableReason::IncompatibleVersion)?;
        if contribution
            .source
            .as_ref()
            .is_some_and(|source| !plugin.operations.contains(&source.operation))
        {
            return Err(UiUnavailableReason::NoPermission);
        }
        Ok((plugin, contribution))
    }
}

fn bind_map(input: &BTreeMap<String, String>, payload: &Value) -> Result<Map<String, Value>> {
    input
        .iter()
        .map(|(name, path)| {
            let value = payload
                .pointer(path)
                .ok_or(ProtocolError::Denied("UI input unavailable"))?;
            Ok((name.clone(), value.clone()))
        })
        .collect()
}
fn bounded_input(output: Map<String, Value>) -> Result<Value> {
    let output = Value::Object(output);
    if serde_json::to_vec(&output).map_or(true, |v| v.len() > MAX_UI_ACTION_INPUT_BYTES) {
        return Err(ProtocolError::Denied("UI input limit"));
    }
    Ok(output)
}
pub(crate) fn bind_inputs(input: &BTreeMap<String, String>, payload: &Value) -> Result<Value> {
    bounded_input(bind_map(input, payload)?)
}

/// Copy only manifest-declared input fields. Historical data cannot inject
/// tenant/actor/permissions or extra parameters into a capability command.
pub fn bind_action_input(action: &UiAction, payload: &Value) -> Result<Value> {
    bind_inputs(&action.input, payload)
}

/// [`bind_action_input`] plus host-collected values: validated form values
/// ([`UiDocument::validate_form`]) or a choices selection
/// ([`UiChoicesBlock::selection_input`]). A name may not repeat a declared
/// input, so collected values never replace bound ones.
pub fn bind_action_input_with(
    action: &UiAction,
    payload: &Value,
    extra: Map<String, Value>,
) -> Result<Value> {
    let mut output = bind_map(&action.input, payload)?;
    for (name, value) in extra {
        if !valid_name(&name) || output.contains_key(&name) {
            return Err(ProtocolError::Denied("UI input collision"));
        }
        output.insert(name, value);
    }
    bounded_input(output)
}
