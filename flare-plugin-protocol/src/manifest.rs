use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ProtocolError, Result};

pub const MANIFEST_VERSION: u32 = 1;
pub const PLUGIN_API_VERSION: &str = "1.0";
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
/// Entries of [`PluginManifest::features`].
pub const MAX_PLUGIN_FEATURES: usize = 64;
pub const MAX_EVENT_SUBSCRIPTIONS: usize = 32;
/// Bytes of one locale's [`PluginFeature::label`].
pub const MAX_FEATURE_LABEL_BYTES: usize = 64;
/// Bytes of one locale's [`PluginFeature::description`].
pub const MAX_FEATURE_DESCRIPTION_BYTES: usize = 1000;
/// Entries of [`PluginManifest::assets`].
pub const MAX_PLUGIN_ASSETS: usize = 64;
/// Bytes of a [`PluginAsset::path`].
pub const MAX_ASSET_PATH_BYTES: usize = 256;
/// Serialized size of one [`UiContribution::preview`].
pub const MAX_UI_PREVIEW_BYTES: usize = 8 * 1024;

/// Canonical platform contract. Endpoint addresses, secrets and tenant state are
/// supplied by the host; they must never be embedded in a published manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub manifest_version: u32,
    pub plugin_id: String,
    pub version: String,
    pub api_version: String,
    pub display_name: String,
    /// Publisher-owned display names per locale (`zh-CN`, `en-US`): at most
    /// 16 entries of 160 UTF-8 bytes each. Hosts use `displayName` when the
    /// requested language is unavailable.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("maxProperties" = 16))]
    pub display_name_i18n: BTreeMap<String, String>,
    #[serde(default)]
    pub description: String,
    pub publisher: Publisher,
    pub runtime: PluginRuntime,
    #[serde(default)]
    pub dependencies: Vec<PluginDependency>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_operations: Vec<crate::service::ServiceOperation>,
    pub capabilities: Vec<Capability>,
    pub permissions: Vec<PluginPermission>,
    #[serde(default)]
    pub events: Vec<PluginEvent>,
    /// Core observations delivered asynchronously, after approval of their permissions/scopes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = MAX_EVENT_SUBSCRIPTIONS))]
    pub event_subscriptions: Vec<crate::event::PluginEventSubscription>,
    /// What an application can publish separately. A plugin with user
    /// operations declares at least one (see [`PluginFeature`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = MAX_PLUGIN_FEATURES))]
    pub features: Vec<PluginFeature>,
    /// Digest table of the package's `assets/` files, covered by the
    /// publisher signature (see [`crate::package`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(length(max = MAX_PLUGIN_ASSETS))]
    pub assets: Vec<PluginAsset>,
    #[serde(default)]
    pub client_extensions: ClientExtensions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_schema: Option<Value>,
    #[serde(default)]
    pub data_policy: DataPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<PluginArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_version_range: Option<String>,
    /// The management surface (Admin Extension Contract), see
    /// [`crate::admin::PluginAdmin`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin: Option<crate::admin::PluginAdmin>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Publisher {
    pub publisher_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum PluginRuntime {
    Grpc {
        service: String,
        protocol: String,
    },
    Wasm {
        module: String,
        #[serde(rename = "abiVersion")]
        abi_version: String,
        #[serde(rename = "maxMemoryPages")]
        max_memory_pages: u32,
        #[serde(rename = "maxFuel")]
        max_fuel: u64,
    },
    Client,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginDependency {
    pub plugin_id: String,
    pub version_range: String,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capability {
    pub id: String,
    pub operations: Vec<PluginOperation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginOperation {
    /// Relative to the capability id; the wire operation is `<capability>.<id>`.
    pub id: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permission_ids: Vec<String>,
    pub idempotent: bool,
    pub timeout_ms: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_capabilities: Vec<crate::resource::HostCapability>,
    /// `query` declares that the operation only reads: hosts may call it in
    /// the background without a user gesture (a card's `viewerState`, an
    /// account entry's `summary`). Default `command`. The plugin still
    /// authorizes every call; the mode only says what a host may schedule.
    #[serde(default, skip_serializing_if = "OperationMode::is_command")]
    pub mode: OperationMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OperationMode {
    /// May change state; invoked on a user gesture or by the plugin's own flow.
    #[default]
    Command,
    /// Reads only; safe for hosts to call in the background.
    Query,
}
impl OperationMode {
    pub fn is_command(&self) -> bool {
        *self == Self::Command
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginPermission {
    pub id: String,
    pub description: String,
    pub data_scopes: Vec<String>,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginEvent {
    pub id: String,
    pub schema_version: u32,
    pub schema: Value,
}

/// One switch an application publishes on its own, with the global key
/// `<pluginId>.<id>`. Operation, service operation and contribution gates are
/// enforced by Core and never reach the plugin process. A configuration gate
/// is for behaviour that shares one operation (a voice and a video call are
/// the same `start`): the plugin reads the key from its effective configuration.
///
/// An operation may belong to several features; closing any of them closes it.
/// A feature the application has not published is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginFeature {
    /// Unique in the manifest.
    pub id: String,
    /// Display name per locale (`zh-CN`, `en-US`).
    pub label: BTreeMap<String, String>,
    /// What publishing or closing the feature changes, per locale.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub description: BTreeMap<String, String>,
    #[serde(default)]
    pub risk: RiskLevel,
    /// Whether the publishing flow preselects the feature. Not a runtime
    /// default: an unpublished feature stays closed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default_enabled: bool,
    pub gates: FeatureGates,
    /// Global keys (`<pluginId>.<id>`) of features this one needs, of this
    /// plugin or a declared dependency. They order and explain publishing;
    /// Core never cascades a close.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
}

/// What a closed [`PluginFeature`] takes away.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeatureGates {
    /// Wire ids of user operations and asynchronous Core event callbacks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<String>,
    /// Wire ids of `user` mode service operations. `recovery` mode is never
    /// gated: closing a payment must not stop its reconciliation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_operations: Vec<String>,
    /// Ids of client contributions the catalog leaves out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contributions: Vec<String>,
    /// A boolean top-level property of `configurationSchema` marked
    /// client-visible. Its value follows the switch; the plugin enforces it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<String>,
}

impl FeatureGates {
    fn is_empty(&self) -> bool {
        self.operations.is_empty()
            && self.service_operations.is_empty()
            && self.contributions.is_empty()
            && self.configuration.is_none()
    }
}

/// One file of a package's `assets/`. The table is part of the signed
/// manifest, so changing a file fails verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginAsset {
    /// Package path below `assets/`.
    pub path: String,
    /// `sha256:<hex>` of the file.
    pub digest: String,
    /// File size in bytes.
    pub size: u64,
}

/// How a plugin is presented and governed, derived from what it declares
/// ([`PluginManifest::kind`]), never declared by the publisher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// Reads IM or Social resources, or renders inside messages.
    Im,
    /// Everything else: Core only forwards its opaque data.
    Business,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientExtensions {
    #[serde(default)]
    pub platforms: Vec<ClientPlatform>,
    #[serde(default)]
    pub contributions: Vec<UiContribution>,
    #[serde(default)]
    pub native_capabilities: Vec<crate::ui::NativeCapability>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ClientPlatform {
    Web,
    Tauri,
    Android,
    Ios,
    Flutter,
    Harmonyos,
}

/// Kit icon registry names: kebab-case, starting with a letter (`red-packet`).
pub const UI_ICON_PATTERN: &str = r"^[a-z][a-z0-9]*(-[a-z0-9]+)*$";
pub const UI_ICON_MAX_LEN: usize = 32;
/// Order of a contribution that declares none.
pub const DEFAULT_UI_ORDER: i32 = 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiContribution {
    pub kind: UiContributionKind,
    pub id: String,
    pub schema_version: u32,
    pub schema: crate::ui::UiDocument,
    pub required_permissions: Vec<String>,
    /// Name in the design kit's icon registry. Presentation only: a kit that
    /// does not know the name shows its generic plugin icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = UI_ICON_MAX_LEN), regex(pattern = UI_ICON_PATTERN))]
    pub icon: Option<String>,
    /// Ascending position among entries of the same kind (default 1000). Hosts
    /// list plugin entries after their built-in ones; ties sort by title.
    #[serde(
        default = "default_ui_order",
        skip_serializing_if = "is_default_ui_order"
    )]
    #[schemars(extend("default" = DEFAULT_UI_ORDER))]
    pub order: i32,
    /// message-content only: JSON pointer into the card payload whose value
    /// (string or integer) is the plugin entity id, e.g. `/pollId`. Core binds
    /// `(tenant, pluginId, entity)` to the message when it is stored, which is
    /// what lets the plugin publish [`crate::state::SharedState`] for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_path: Option<String>,
    /// message-content only: whether hosts offer "forward" on this card
    /// (default false: plugin cards are bound to their conversation).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forwardable: bool,
    /// composer-action only: who sees the entry. Presentation only; the
    /// plugin still authorizes every invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<crate::ui::UiAudience>,
    /// message-content only: the "view all" document and the operation that
    /// returns its data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<crate::ui::UiDetail>,
    /// message-content only: the viewer's own layer of the card, read in the
    /// background by the core SDK while the card is presented (see
    /// [`crate::ui::UiViewerState`]). Its operation must be a declared
    /// `query`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewer_state: Option<crate::ui::UiViewerState>,
    /// message-content only, with `entityPath`: the operation Core calls
    /// with a platform system ticket when the card bound to an entity goes
    /// away (see [`UiEntityLifecycle`]). A card with `entityPath` and no
    /// `unbound` operation is valid; [`PluginManifest::warnings`] reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_lifecycle: Option<UiEntityLifecycle>,
    /// account-entry only: a scalar read from a `query` operation result and
    /// shown as the entry's subtitle (a wallet balance). Hosts call it on
    /// demand; listing entries never calls it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<crate::ui::UiEntrySummary>,
    /// composer-action only: a `query` whose result says whether the entry
    /// can be used now (see [`crate::ui::UiEntryAvailability`]). Without it
    /// the entry is always listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<crate::ui::UiEntryAvailability>,
    /// page only: the `query` operation whose result the page document is
    /// resolved against, its input bound from the page arguments, and its
    /// paging (see [`crate::ui::UiPageSource`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::ui::UiPageSource>,
    /// Sample data the marketplace renders this contribution with: an object
    /// of at most [`MAX_UI_PREVIEW_BYTES`]. Never read at run time; hosts
    /// leave it out of the client catalog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Value>,
}

/// What Core tells a plugin about the card bound to one of its entities.
///
/// Core invokes `unbound` with a platform system ticket (its own call domain,
/// verified by the plugin host; never a user ticket) and the input
/// `{entity, reason, serverMsgId?}`, `reason` one of
/// [`UNBOUND_REASON_RECALLED`] (the bound card was recalled) and
/// [`UNBOUND_REASON_POST_FAILED`] (the platform could not post the card).
/// Delivery is at least once: plugins handle it idempotently per entity (a
/// poll is cancelled, a red packet refunded at once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiEntityLifecycle {
    /// Wire operation declared by the plugin.
    pub unbound: String,
}
/// [`UiEntityLifecycle::unbound`] reason: the bound card was recalled.
pub const UNBOUND_REASON_RECALLED: &str = "recalled";
/// [`UiEntityLifecycle::unbound`] reason: the platform's post of the card
/// failed for good.
pub const UNBOUND_REASON_POST_FAILED: &str = "post_failed";

/// A non-fatal finding of [`PluginManifest::warnings`]: publishing proceeds,
/// tooling shows it to the author.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestWarning {
    pub field: String,
    pub reason: String,
}

fn default_ui_order() -> i32 {
    DEFAULT_UI_ORDER
}

fn is_default_ui_order(order: &i32) -> bool {
    *order == DEFAULT_UI_ORDER
}

impl UiContribution {
    /// The entity id of a card payload: the string or integer at
    /// [`Self::entity_path`], when it is a valid entity id.
    pub fn entity_of(&self, payload: &Value) -> Option<String> {
        let id = match payload.pointer(self.entity_path.as_deref()?)? {
            Value::String(id) => id.clone(),
            Value::Number(n) if n.is_u64() || n.is_i64() => n.to_string(),
            _ => return None,
        };
        crate::state::valid_entity_id(&id).then_some(id)
    }
}

/// Whether `value` is a well-formed kit icon name (see [`UI_ICON_PATTERN`]).
pub fn is_ui_icon_name(value: &str) -> bool {
    value.len() <= UI_ICON_MAX_LEN
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UiContributionKind {
    MessageContent,
    MessageAction,
    ConversationMenu,
    ComposerAction,
    Page,
    /// An entry of the "Me" (account) screen, such as a wallet: its title,
    /// translations and target come from its document, which holds no fields,
    /// blocks or inputs and exactly one action targeting a `page`
    /// contribution of the same plugin. `icon` and `order` place it; an
    /// optional `summary` names a `query` whose result gives a subtitle.
    AccountEntry,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataPolicy {
    #[serde(default)]
    pub tenant_isolation: TenantIsolation,
    #[serde(default)]
    pub uninstall_behavior: UninstallBehavior,
    #[serde(default)]
    pub migration_policy: MigrationPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TenantIsolation {
    #[default]
    Required,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UninstallBehavior {
    #[default]
    Retain,
    ExportThenRetain,
    DeleteByRequest,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum MigrationPolicy {
    #[default]
    None,
    BackwardCompatible,
    Versioned,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginArtifact {
    pub digest: String,
    pub signature_ref: String,
}

impl PluginManifest {
    /// Display text only; plugin identity and authorization always use `plugin_id`.
    /// Resolve an exact locale (case insensitive), then its language, then the
    /// publisher's default. The final ID fallback also handles unvalidated values.
    pub fn localized_display_name(&self, locale: &str) -> &str {
        let language = |tag: &str| tag.split('-').next().unwrap_or(tag).to_ascii_lowercase();
        self.display_name_i18n
            .iter()
            .find(|(tag, text)| tag.eq_ignore_ascii_case(locale) && !text.trim().is_empty())
            .or_else(|| {
                self.display_name_i18n.iter().find(|(tag, text)| {
                    !locale.is_empty()
                        && language(tag) == language(locale)
                        && !text.trim().is_empty()
                })
            })
            .map(|(_, text)| text.as_str())
            .unwrap_or_else(|| {
                if self.display_name.trim().is_empty() {
                    &self.plugin_id
                } else {
                    &self.display_name
                }
            })
    }

    pub fn service_operation_ids(&self) -> BTreeSet<String> {
        self.service_operations
            .iter()
            .map(|s| s.operation.clone())
            .collect()
    }
    /// Whether an installed service target matches a declared dependency.
    /// `optional` controls installation requirements; it does not forbid using
    /// a present target. This match never replaces either plugin's grants or
    /// the target's declared service-operation authorization.
    pub fn requires(&self, plugin_id: &str, version: &str) -> bool {
        self.dependencies.iter().any(|d| {
            d.plugin_id == plugin_id
                && semver::VersionReq::parse(&d.version_range)
                    .ok()
                    .zip(semver::Version::parse(version).ok())
                    .is_some_and(|(r, v)| r.matches(&v))
        })
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ProtocolError::ManifestTooLarge(MAX_MANIFEST_BYTES));
        }
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidJson)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        ensure(
            self.manifest_version == MANIFEST_VERSION,
            "manifestVersion",
            "unsupported version",
        )?;
        ensure(
            self.api_version == PLUGIN_API_VERSION,
            "apiVersion",
            "unsupported API version",
        )?;
        identifier(&self.plugin_id, "pluginId")?;
        ensure(self.plugin_id.len() <= 128, "pluginId", "too long")?;
        ensure(
            self.version.len() <= 64 && semver::Version::parse(&self.version).is_ok(),
            "version",
            "expected semantic version",
        )?;
        bounded(&self.display_name, 160, "displayName")?;
        localized(&self.display_name_i18n, 160, "displayNameI18n")?;
        ensure(self.description.len() <= 4000, "description", "too long")?;
        identifier(&self.publisher.publisher_id, "publisher.publisherId")?;
        bounded(&self.publisher.display_name, 160, "publisher.displayName")?;
        if let Some(range) = &self.host_version_range {
            version_range(range, "hostVersionRange")?;
        }
        match &self.runtime {
            PluginRuntime::Grpc { service, protocol } => {
                bounded(service, 256, "runtime.service")?;
                ensure(
                    protocol == "flare.capability.v1",
                    "runtime.protocol",
                    "unsupported protocol",
                )?;
            }
            PluginRuntime::Wasm {
                module,
                abi_version,
                max_memory_pages,
                max_fuel,
            } => {
                bounded(module, 512, "runtime.module")?;
                ensure(
                    !module.starts_with('/')
                        && !module.contains('\\')
                        && module
                            .split('/')
                            .all(|part| !part.is_empty() && part != "." && part != ".."),
                    "runtime.module",
                    "expected relative artifact path",
                )?;
                bounded(abi_version, 32, "runtime.abiVersion")?;
                ensure(
                    (1..=65536).contains(max_memory_pages),
                    "runtime.maxMemoryPages",
                    "out of range",
                )?;
                ensure(*max_fuel > 0, "runtime.maxFuel", "must be positive")?;
            }
            PluginRuntime::Client => {}
        }
        if let Some(artifact) = &self.artifact {
            ensure(
                is_sha256_digest(&artifact.digest),
                "artifact.digest",
                "expected sha256 digest",
            )?;
            bounded(&artifact.signature_ref, 512, "artifact.signatureRef")?;
        } else {
            ensure(
                matches!(self.runtime, PluginRuntime::Client),
                "artifact",
                "server runtime requires signed artifact reference",
            )?;
        }
        ensure(
            self.dependencies.len() <= 64,
            "dependencies",
            "too many dependencies",
        )?;
        ensure(
            self.service_operations.len() <= 256,
            "serviceOperations",
            "too many service operations",
        )?;
        let client_ops = self.client_operation_refs();
        let mut service_ops = BTreeSet::new();
        for service in &self.service_operations {
            if !service_ops.insert(&service.operation)
                || self.operation(&service.operation).is_none()
                || self.admin_operations().contains(&service.operation)
                || client_ops.contains(&service.operation)
            {
                return Err(ProtocolError::Denied(
                    "invalid service operation declaration",
                ));
            }
        }
        let mut dependencies = BTreeSet::new();
        for dep in &self.dependencies {
            identifier(&dep.plugin_id, "dependencies.pluginId")?;
            ensure(
                dep.plugin_id != self.plugin_id && dependencies.insert(&dep.plugin_id),
                "dependencies",
                "duplicate or self dependency",
            )?;
            version_range(&dep.version_range, "dependencies.versionRange")?;
        }
        let mut permissions = BTreeSet::new();
        for permission in &self.permissions {
            identifier(&permission.id, "permissions.id")?;
            ensure(
                permissions.insert(permission.id.as_str()),
                "permissions",
                "duplicate permission",
            )?;
            bounded(&permission.description, 1000, "permissions.description")?;
            unique_identifiers(&permission.data_scopes, "permissions.dataScopes")?;
        }
        ensure(
            !self.capabilities.is_empty() && self.capabilities.len() <= 128,
            "capabilities",
            "expected 1..128 capabilities",
        )?;
        let mut capabilities = BTreeSet::new();
        let mut operation_ids = BTreeSet::new();
        for cap in &self.capabilities {
            identifier(&cap.id, "capabilities.id")?;
            ensure(
                capabilities.insert(&cap.id),
                "capabilities",
                "duplicate capability",
            )?;
            ensure(
                !cap.operations.is_empty() && cap.operations.len() <= 256,
                "capabilities.operations",
                "expected 1..256 operations",
            )?;
            for op in &cap.operations {
                identifier(&op.id, "operations.id")?;
                ensure(
                    operation_ids.insert(format!("{}.{}", cap.id, op.id)),
                    "operations",
                    "duplicate wire operation",
                )?;
                ensure(
                    cap.id.len() + 1 + op.id.len() <= 128,
                    "operations.id",
                    "wire operation exceeds 128 bytes",
                )?;
                schema(&op.input_schema, "operations.inputSchema")?;
                schema(&op.output_schema, "operations.outputSchema")?;
                permission_refs(&op.permission_ids, &permissions, "operations.permissionIds")?;
                let mut host_capabilities = BTreeSet::new();
                for capability in &op.host_capabilities {
                    ensure(
                        host_capabilities.insert(capability),
                        "operations.hostCapabilities",
                        "duplicate host capability",
                    )?;
                    ensure(
                        self.permissions.iter().any(|permission| {
                            op.permission_ids.contains(&permission.id)
                                && permission
                                    .data_scopes
                                    .iter()
                                    .any(|scope| scope == capability.id())
                        }),
                        "operations.hostCapabilities",
                        "host capability requires an explicit operation permission data scope",
                    )?;
                }
                ensure(
                    (1..=120000).contains(&op.timeout_ms),
                    "operations.timeoutMs",
                    "out of range",
                )?;
            }
        }
        ensure(
            operation_ids.len() <= 256,
            "capabilities.operations",
            "expected at most 256 total wire operations",
        )?;
        let mut event_ids = BTreeSet::new();
        for event in &self.events {
            identifier(&event.id, "events.id")?;
            ensure(event_ids.insert(&event.id), "events", "duplicate event")?;
            ensure(
                event.schema_version > 0,
                "events.schemaVersion",
                "must be positive",
            )?;
            schema(&event.schema, "events.schema")?;
        }
        ensure(
            self.event_subscriptions.len() <= MAX_EVENT_SUBSCRIPTIONS,
            "eventSubscriptions",
            "at most 32 subscriptions",
        )?;
        let mut subscriptions = BTreeSet::new();
        for subscription in &self.event_subscriptions {
            use crate::event::{MESSAGE_BODY_SCOPE, MESSAGE_CREATED, MESSAGE_EVENTS_SCOPE};
            ensure(
                subscription.event_type == MESSAGE_CREATED && subscription.schema_version == 1,
                "eventSubscriptions",
                "unsupported event type or schema",
            )?;
            ensure(
                subscriptions.insert((&subscription.event_type, &subscription.operation)),
                "eventSubscriptions",
                "duplicate subscription",
            )?;
            let operation = self
                .operation(&subscription.operation)
                .ok_or(ProtocolError::Denied("undeclared event operation"))?;
            ensure(
                operation.idempotent
                    && !self.admin_operations().contains(&subscription.operation)
                    && !self
                        .service_operation_ids()
                        .contains(&subscription.operation)
                    && !self.client_extensions.contributions.iter().any(|c| {
                        c.entity_lifecycle
                            .as_ref()
                            .is_some_and(|l| l.unbound == subscription.operation)
                    }),
                "eventSubscriptions.operation",
                "event callbacks must be idempotent and use a dedicated operation",
            )?;
            permission_refs(
                &subscription.permission_ids,
                &permissions,
                "eventSubscriptions.permissionIds",
            )?;
            ensure(
                !subscription.permission_ids.is_empty()
                    && subscription
                        .permission_ids
                        .iter()
                        .all(|id| operation.permission_ids.contains(id)),
                "eventSubscriptions.permissionIds",
                "event permissions must belong to the operation",
            )?;
            for scope in [
                Some(MESSAGE_EVENTS_SCOPE),
                subscription.include_body.then_some(MESSAGE_BODY_SCOPE),
            ]
            .into_iter()
            .flatten()
            {
                ensure(
                    self.permissions.iter().any(|permission| {
                        subscription.permission_ids.contains(&permission.id)
                            && permission.data_scopes.iter().any(|value| value == scope)
                    }),
                    "eventSubscriptions",
                    "event and body scopes require explicit permissions",
                )?;
            }
        }
        if let Some(configuration) = &self.configuration_schema {
            schema(configuration, "configurationSchema")?;
            crate::configuration::validate_client_visible_markers(configuration)?;
        }
        let mut contribution_ids = BTreeSet::new();
        for contribution in &self.client_extensions.contributions {
            identifier(&contribution.id, "clientExtensions.contributions.id")?;
            ensure(
                contribution_ids.insert(&contribution.id),
                "clientExtensions.contributions",
                "duplicate contribution",
            )?;
            ensure(
                contribution.schema_version == 1,
                "clientExtensions.schemaVersion",
                "must be positive",
            )?;
            contribution.schema.validate()?;
            ensure(
                contribution.icon.as_deref().is_none_or(is_ui_icon_name),
                "clientExtensions.contributions.icon",
                "expected kebab-case kit icon name of at most 32 bytes",
            )?;
            if let Some(preview) = &contribution.preview {
                ensure(
                    preview.is_object()
                        && serde_json::to_vec(preview)
                            .is_ok_and(|bytes| bytes.len() <= MAX_UI_PREVIEW_BYTES),
                    "clientExtensions.contributions.preview",
                    "expected an object of at most 8 KiB",
                )?;
            }
            ensure(
                contribution.id.starts_with(&format!("{}.", self.plugin_id)),
                "clientExtensions.id",
                "must belong to plugin",
            )?;
            self.validate_contribution_surface(contribution)?;
            let documents = std::iter::once(&contribution.schema)
                .chain(contribution.detail.as_ref().map(|detail| &detail.document));
            for document in documents {
                for action in &document.actions {
                    use crate::ui::UiActionTarget;
                    let declared = match &action.target {
                        UiActionTarget::Operation { operation } => {
                            self.operation(operation).is_some()
                        }
                        UiActionTarget::Page { page, plugin_id }
                            if plugin_id.as_ref().is_some_and(|p| p != &self.plugin_id) =>
                        {
                            plugin_id
                                .as_ref()
                                .is_some_and(|id| page.starts_with(&format!("{id}.")))
                        }
                        UiActionTarget::Page { page, .. } => self
                            .client_extensions
                            .contributions
                            .iter()
                            .any(|c| &c.id == page && c.kind == UiContributionKind::Page),
                        UiActionTarget::Native { capability } => self
                            .client_extensions
                            .native_capabilities
                            .contains(capability),
                    };
                    ensure(
                        declared,
                        "clientExtensions.actions",
                        "target must be declared",
                    )?;
                }
                for source in document.rows_sources() {
                    ensure(
                        self.operation(source).is_some(),
                        "clientExtensions.blocks.rows.source",
                        "must be a declared operation",
                    )?;
                }
            }
            permission_refs(
                &contribution.required_permissions,
                &permissions,
                "clientExtensions.requiredPermissions",
            )?;
        }
        ensure(
            self.client_extensions
                .platforms
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == self.client_extensions.platforms.len(),
            "clientExtensions.platforms",
            "duplicate platform",
        )?;
        ensure(
            self.client_extensions
                .native_capabilities
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                == self.client_extensions.native_capabilities.len(),
            "clientExtensions.nativeCapabilities",
            "duplicate capability",
        )?;
        ensure(
            self.client_extensions.contributions.len() <= 64,
            "clientExtensions.contributions",
            "too many contributions",
        )?;
        if let Some(admin) = &self.admin {
            admin.validate(self)?;
            for page in &admin.pages {
                for document in std::iter::once(&page.document)
                    .chain(page.detail.as_ref().map(|detail| &detail.document))
                {
                    ensure(
                        document.inputs.iter().all(|input| input.file.is_none()),
                        "admin.pages.inputs.file",
                        "file uploads require a client page main-document form",
                    )?;
                }
            }
        }
        self.validate_features()?;
        self.validate_assets()
    }

    /// What a publisher may declare as a feature: unique ids, bounded texts,
    /// at least one gate, gates that name what they may close, and `requires`
    /// that resolve. A plugin with user operations has at least one feature.
    fn validate_features(&self) -> Result<()> {
        ensure(
            self.features.len() <= MAX_PLUGIN_FEATURES,
            "features",
            "too many features",
        )?;
        let user_operations = self.user_operation_ids();
        ensure(
            user_operations.is_empty() || !self.features.is_empty(),
            "features",
            "a plugin with user operations declares at least one feature",
        )?;
        let mut ids = BTreeSet::new();
        for feature in &self.features {
            identifier(&feature.id, "features.id")?;
            ensure(feature.id.len() <= 64, "features.id", "too long")?;
            ensure(
                ids.insert(feature.id.as_str()),
                "features",
                "duplicate feature",
            )?;
        }
        for feature in &self.features {
            ensure(!feature.label.is_empty(), "features.label", "required")?;
            localized(&feature.label, MAX_FEATURE_LABEL_BYTES, "features.label")?;
            localized(
                &feature.description,
                MAX_FEATURE_DESCRIPTION_BYTES,
                "features.description",
            )?;
            let gates = &feature.gates;
            ensure(
                !gates.is_empty(),
                "features.gates",
                "a feature gates at least one thing",
            )?;
            unique_identifiers(&gates.operations, "features.gates.operations")?;
            ensure(
                gates.operations.iter().all(|operation| {
                    user_operations.contains(operation)
                        || self
                            .event_subscriptions
                            .iter()
                            .any(|s| &s.operation == operation)
                }),
                "features.gates.operations",
                "must be declared user or event operations",
            )?;
            unique_identifiers(
                &gates.service_operations,
                "features.gates.serviceOperations",
            )?;
            ensure(
                gates.service_operations.iter().all(|operation| {
                    self.service_operations.iter().any(|service| {
                        &service.operation == operation
                            && service.mode == crate::service::ServiceMode::User
                    })
                }),
                "features.gates.serviceOperations",
                "must be service operations declared with mode user",
            )?;
            unique_identifiers(&gates.contributions, "features.gates.contributions")?;
            ensure(
                gates
                    .contributions
                    .iter()
                    .all(|contribution| self.contribution(contribution).is_some()),
                "features.gates.contributions",
                "must be declared contributions",
            )?;
            ensure(
                gates
                    .configuration
                    .as_deref()
                    .is_none_or(|key| self.is_client_visible_boolean(key)),
                "features.gates.configuration",
                "must be a boolean client-visible configurationSchema property",
            )?;
            unique_identifiers(&feature.requires, "features.requires")?;
            ensure(
                feature
                    .requires
                    .iter()
                    .all(|reference| self.resolves_feature(reference, &ids)),
                "features.requires",
                "must be <pluginId>.<id> of a feature of this plugin or a declared dependency",
            )?;
        }
        Ok(())
    }

    /// A `requires` reference: an existing feature of this plugin, or a
    /// feature of a declared dependency. A dependency's features are checked
    /// when it is resolved, not here.
    fn resolves_feature(&self, reference: &str, own: &BTreeSet<&str>) -> bool {
        let of = |plugin_id: &str| {
            reference
                .strip_prefix(plugin_id)
                .and_then(|id| id.strip_prefix('.'))
                .filter(|id| !id.is_empty())
        };
        of(&self.plugin_id).is_some_and(|id| own.contains(id))
            || self
                .dependencies
                .iter()
                .any(|dependency| of(&dependency.plugin_id).is_some())
    }

    fn is_client_visible_boolean(&self, key: &str) -> bool {
        self.configuration_schema.as_ref().is_some_and(|schema| {
            crate::configuration::client_visible_keys(schema).contains(key)
                && schema
                    .get("properties")
                    .and_then(|properties| properties.get(key))
                    .and_then(|property| property.get("type"))
                    .and_then(Value::as_str)
                    == Some("boolean")
        })
    }

    fn validate_assets(&self) -> Result<()> {
        ensure(
            self.assets.len() <= MAX_PLUGIN_ASSETS,
            "assets",
            "too many assets",
        )?;
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        for asset in &self.assets {
            ensure(
                is_asset_path(&asset.path),
                "assets.path",
                "expected a relative path below assets/",
            )?;
            ensure(paths.insert(&asset.path), "assets", "duplicate asset")?;
            ensure(
                is_sha256_digest(&asset.digest),
                "assets.digest",
                "expected sha256 digest",
            )?;
            ensure(asset.size > 0, "assets.size", "must be positive")?;
            total = total.saturating_add(asset.size);
            ensure(
                total <= crate::package::MAX_PACKAGE_BYTES,
                "assets.size",
                "assets exceed the package limit",
            )?;
        }
        Ok(())
    }

    /// Derived from the manifest, never declared: [`PluginKind::Im`] when an
    /// operation declares a host capability of the `im.` or `social.`
    /// namespace or a contribution renders inside a message, otherwise
    /// [`PluginKind::Business`].
    pub fn kind(&self) -> PluginKind {
        let host_access = self
            .capabilities
            .iter()
            .flat_map(|capability| &capability.operations)
            .flat_map(|operation| &operation.host_capabilities)
            .any(|capability| {
                capability.id().starts_with("im.") || capability.id().starts_with("social.")
            });
        let message_content = self
            .client_extensions
            .contributions
            .iter()
            .any(|contribution| contribution.kind == UiContributionKind::MessageContent);
        if host_access || message_content {
            PluginKind::Im
        } else {
            PluginKind::Business
        }
    }

    /// Operations a user ticket may invoke: everything declared but the admin
    /// operations, the service operations and the entity lifecycle callbacks.
    pub fn user_operation_ids(&self) -> BTreeSet<String> {
        let restricted: BTreeSet<String> = self
            .admin_operations()
            .into_iter()
            .chain(self.service_operation_ids())
            .chain(self.event_subscriptions.iter().map(|s| s.operation.clone()))
            .chain(
                self.client_extensions
                    .contributions
                    .iter()
                    .filter_map(|c| c.entity_lifecycle.as_ref().map(|l| l.unbound.clone())),
            )
            .collect();
        self.operation_ids()
            .into_iter()
            .filter(|operation| !restricted.contains(operation))
            .collect()
    }

    /// User operations an application's feature states close. `features` maps
    /// [`PluginFeature::id`] to its state; a missing key is an unpublished
    /// feature and counts as closed. A feature with only a configuration gate
    /// closes nothing here: the plugin enforces it.
    pub fn closed_operations(&self, features: &BTreeMap<String, bool>) -> BTreeSet<String> {
        self.closed_by(features, |gates| gates.operations.as_slice())
    }

    /// `user` mode service operations closed by `features`, as
    /// [`Self::closed_operations`].
    pub fn closed_service_operations(&self, features: &BTreeMap<String, bool>) -> BTreeSet<String> {
        self.closed_by(features, |gates| gates.service_operations.as_slice())
    }

    /// Contributions closed by `features`, as [`Self::closed_operations`].
    pub fn closed_contributions(&self, features: &BTreeMap<String, bool>) -> BTreeSet<String> {
        self.closed_by(features, |gates| gates.contributions.as_slice())
    }

    fn closed_by(
        &self,
        features: &BTreeMap<String, bool>,
        gated: impl Fn(&FeatureGates) -> &[String],
    ) -> BTreeSet<String> {
        self.features
            .iter()
            .filter(|feature| !features.get(&feature.id).copied().unwrap_or(false))
            .flat_map(|feature| gated(&feature.gates).iter().cloned())
            .collect()
    }

    /// Operations the client contributions use: action targets, rows sources,
    /// details, viewer states, summaries, page sources and entity lifecycle
    /// callbacks of every contribution. No admin operation may be one of them.
    pub fn client_operation_refs(&self) -> BTreeSet<String> {
        use crate::ui::UiActionTarget;
        let mut operations = BTreeSet::new();
        for contribution in &self.client_extensions.contributions {
            operations.extend(
                [
                    contribution.detail.as_ref().map(|d| &d.operation),
                    contribution.viewer_state.as_ref().map(|v| &v.operation),
                    contribution.summary.as_ref().map(|s| &s.operation),
                    contribution.availability.as_ref().map(|a| &a.operation),
                    contribution.entity_lifecycle.as_ref().map(|l| &l.unbound),
                    contribution.source.as_ref().map(|s| &s.operation),
                ]
                .into_iter()
                .flatten()
                .cloned(),
            );
            for document in std::iter::once(&contribution.schema)
                .chain(contribution.detail.as_ref().map(|detail| &detail.document))
            {
                operations.extend(document.rows_sources().map(str::to_owned));
                for file in document
                    .inputs
                    .iter()
                    .filter_map(|input| input.file.as_ref())
                {
                    operations.insert(file.upload.prepare_operation.clone());
                    operations.insert(file.upload.complete_operation.clone());
                }
                operations.extend(document.actions.iter().filter_map(|a| match &a.target {
                    UiActionTarget::Operation { operation } => Some(operation.clone()),
                    _ => None,
                }));
            }
        }
        operations
    }

    /// Non-fatal findings about a manifest that [`Self::validate`] accepts:
    /// a message-content card bound to an entity (`entityPath`) without
    /// `entityLifecycle.unbound` never learns that its card was recalled or
    /// never posted, so its entity can be left open (a poll still running, a
    /// red packet held until it expires).
    pub fn warnings(&self) -> Vec<ManifestWarning> {
        self.client_extensions
            .contributions
            .iter()
            .filter(|c| {
                c.kind == UiContributionKind::MessageContent
                    && c.entity_path.is_some()
                    && c.entity_lifecycle.is_none()
            })
            .map(|c| ManifestWarning {
                field: format!("clientExtensions.contributions[{}].entityLifecycle", c.id),
                reason: "entityPath without entityLifecycle.unbound: the plugin is not told when its card is recalled or never posted".to_owned(),
            })
            .collect()
    }

    /// Contribution-kind rules for the v2 fields: entity binding, forwarding,
    /// viewer state and entity lifecycle on message-content, audience and
    /// availability on composer-action, detail on message-content, the single page action and
    /// summary of account-entry, the source of a page, rows on page documents
    /// (console pages live in the manifest's `admin` section).
    fn validate_contribution_surface(&self, contribution: &UiContribution) -> Result<()> {
        use crate::ui::UiActionTarget;
        let kind = contribution.kind;
        let message_content = kind == UiContributionKind::MessageContent;
        ensure(
            contribution
                .entity_path
                .as_deref()
                .is_none_or(|path| message_content && crate::ui::valid_path(path)),
            "clientExtensions.contributions.entityPath",
            "message-content only, expected payload path",
        )?;
        ensure(
            !contribution.forwardable || message_content,
            "clientExtensions.contributions.forwardable",
            "message-content only",
        )?;
        if let Some(audience) = &contribution.audience {
            ensure(
                kind == UiContributionKind::ComposerAction,
                "clientExtensions.contributions.audience",
                "composer-action only",
            )?;
            audience.validate()?;
        }
        if let Some(availability) = &contribution.availability {
            ensure(
                kind == UiContributionKind::ComposerAction,
                "clientExtensions.contributions.availability",
                "composer-action only",
            )?;
            availability.validate()?;
            ensure(
                self.operation(&availability.operation)
                    .is_some_and(|op| op.mode == OperationMode::Query),
                "clientExtensions.contributions.availability.operation",
                "must be a declared query operation",
            )?;
        }
        if let Some(detail) = &contribution.detail {
            ensure(
                message_content,
                "clientExtensions.contributions.detail",
                "message-content only",
            )?;
            detail.validate()?;
            ensure(
                self.operation(&detail.operation).is_some(),
                "clientExtensions.contributions.detail.operation",
                "must be a declared operation",
            )?;
        }
        if let Some(viewer) = &contribution.viewer_state {
            ensure(
                message_content,
                "clientExtensions.contributions.viewerState",
                "message-content only",
            )?;
            viewer.validate()?;
            ensure(
                self.operation(&viewer.operation)
                    .is_some_and(|op| op.mode == OperationMode::Query),
                "clientExtensions.contributions.viewerState.operation",
                "must be a declared query operation",
            )?;
        }
        if let Some(lifecycle) = &contribution.entity_lifecycle {
            ensure(
                message_content && contribution.entity_path.is_some(),
                "clientExtensions.contributions.entityLifecycle",
                "message-content with entityPath only",
            )?;
            ensure(
                self.operation(&lifecycle.unbound).is_some(),
                "clientExtensions.contributions.entityLifecycle.unbound",
                "must be a declared operation",
            )?;
        }
        let account_entry = kind == UiContributionKind::AccountEntry;
        if account_entry {
            let document = &contribution.schema;
            ensure(
                document.fields.is_empty()
                    && document.blocks.is_empty()
                    && document.inputs.is_empty()
                    && contribution.detail.is_none()
                    && matches!(
                        document.actions.as_slice(),
                        [action] if matches!(action.target, UiActionTarget::Page { .. })
                            && action.form.is_empty()
                    ),
                "clientExtensions.contributions.schema",
                "account-entry document holds exactly one page action and nothing else",
            )?;
        }
        if let Some(summary) = &contribution.summary {
            ensure(
                account_entry,
                "clientExtensions.contributions.summary",
                "account-entry only",
            )?;
            summary.validate()?;
            ensure(
                self.operation(&summary.operation)
                    .is_some_and(|op| op.mode == OperationMode::Query),
                "clientExtensions.contributions.summary.operation",
                "must be a declared query operation",
            )?;
        }
        let page = kind == UiContributionKind::Page;
        if let Some(source) = &contribution.source {
            ensure(page, "clientExtensions.contributions.source", "page only")?;
            source.validate()?;
            ensure(
                source
                    .command_slot
                    .as_ref()
                    .is_none_or(|slot| slot.starts_with(&format!("{}.", self.plugin_id))),
                "clientExtensions.source.commandSlot",
                "journal slot must belong to plugin",
            )?;
            ensure(
                self.operation(&source.operation)
                    .is_some_and(|op| op.mode == OperationMode::Query),
                "clientExtensions.contributions.source.operation",
                "must be a declared query operation",
            )?;
            ensure(
                contribution.schema.rows().is_none()
                    || contribution.schema.filter_inputs().iter().all(|input| {
                        input.id != crate::ui::DETAIL_CURSOR_INPUT
                            && !source.input.contains_key(&input.id)
                    }),
                "clientExtensions.inputs",
                "a page filter input may not repeat a source input name or `cursor`",
            )?;
        }
        let documents = std::iter::once(&contribution.schema)
            .chain(contribution.detail.as_ref().map(|detail| &detail.document));
        for (index, document) in documents.enumerate() {
            for input in &document.inputs {
                if let Some(file) = &input.file {
                    let owners: Vec<_> = document
                        .actions
                        .iter()
                        .filter(|action| action.form.contains(&input.id))
                        .collect();
                    ensure(
                        page && index == 0
                            && !owners.is_empty()
                            && owners.iter().all(|action| {
                                !document.blocks.iter().any(|block| {
                                    block.owned_actions().contains(&action.id.as_str())
                                })
                            })
                            && (document.rows().is_none()
                                || !document
                                    .filter_inputs()
                                    .iter()
                                    .any(|filter| filter.id == input.id)),
                        "clientExtensions.inputs.file",
                        "file uploads require root action forms of a page main document, not rows or filters",
                    )?;
                    for operation in [
                        &file.upload.prepare_operation,
                        &file.upload.complete_operation,
                    ] {
                        ensure(
                            operation.starts_with(&format!("{}.", self.plugin_id))
                                && self.user_operation_ids().contains(operation)
                                && self
                                    .operation(operation)
                                    .is_some_and(|op| op.mode == OperationMode::Command),
                            "clientExtensions.inputs.file.upload",
                            "file uploads require command operations declared by the same plugin",
                        )?;
                    }
                }
            }
            for action in &document.actions {
                ensure(
                    (action.command.is_none() && action.continuation.is_none())
                        || (page && index == 0),
                    "clientExtensions.actions.command",
                    "durable commands and continuations require a page document",
                )?;
                ensure(
                    action.command.is_none()
                        || !document.inputs.iter().any(|i| {
                            action.form.contains(&i.id)
                                && i.kind == crate::ui::UiInputKind::Password
                        }),
                    "clientExtensions.actions.form",
                    "password inputs cannot enter durable commands",
                )?;
                if let Some(command) = &action.command {
                    ensure(
                        command.slot.starts_with(&format!("{}.", self.plugin_id)),
                        "clientExtensions.actions.command.slot",
                        "journal slot must belong to plugin",
                    )?;
                    if command.mode == crate::ui::UiPageCommandMode::Observe {
                        ensure(
                            matches!(&action.target, UiActionTarget::Operation { operation }
                                if self.operation(operation).is_some_and(|op| op.mode == OperationMode::Query)),
                            "clientExtensions.actions.command.mode",
                            "observe requires a declared query operation",
                        )?;
                    }
                }
                if let Some(t) = &action.continuation {
                    let target = t.plugin_id.as_deref().unwrap_or(&self.plugin_id);
                    ensure(
                        t.page.starts_with(&format!("{target}."))
                            && (target != self.plugin_id
                                || self
                                    .client_extensions
                                    .contributions
                                    .iter()
                                    .any(|c| c.id == t.page && c.kind == UiContributionKind::Page)),
                        "clientExtensions.actions.continuation",
                        "target page must be declared",
                    )?;
                }
            }
            let rows = document.rows_sources().count();
            ensure(
                rows == 0 || (page && index == 0 && rows == 1),
                "clientExtensions.blocks.rows",
                "one rows block, on a page document only",
            )?;
            ensure(
                document.is_client_surface(),
                "clientExtensions.blocks",
                "protected media and input references are for admin pages only",
            )?;
        }
        if page && let Some(rows) = contribution.schema.rows() {
            // A page's rows are its source's data and page with it.
            ensure(
                contribution
                    .source
                    .as_ref()
                    .is_some_and(|source| source.operation == rows.source)
                    && rows.cursor_path.is_none(),
                "clientExtensions.blocks.rows",
                "page rows read the page source and page with its cursorPath",
            )?;
        }
        Ok(())
    }

    pub fn contribution(&self, id: &str) -> Option<&UiContribution> {
        self.client_extensions
            .contributions
            .iter()
            .find(|c| c.id == id)
    }

    /// Wire operations staff may invoke through `dispatch_admin`: the
    /// `admin.operations`.
    pub fn admin_operations(&self) -> BTreeSet<String> {
        self.admin
            .as_ref()
            .map(crate::admin::PluginAdmin::operation_ids)
            .unwrap_or_default()
    }

    /// The staff permission code a staff ticket for the admin `operation`
    /// carries: the [`crate::invocation::plugin_permission`] code of the
    /// permission its `admin.operations` entry declares. `None` for any other
    /// operation.
    pub fn admin_permission_for(&self, operation: &str) -> Option<String> {
        let declared = self.admin.as_ref()?.operation(operation)?;
        crate::invocation::plugin_permission(&self.plugin_id, &declared.permission)
    }

    /// The subset of a tenant configuration that clients receive with the
    /// catalog: the top-level properties marked `x-flare-client-visible: true`.
    pub fn client_visible_configuration(
        &self,
        configuration: &Value,
    ) -> Result<serde_json::Map<String, Value>> {
        match &self.configuration_schema {
            Some(schema) => {
                crate::configuration::client_visible_configuration(schema, configuration)
            }
            None => Ok(Default::default()),
        }
    }

    /// Where this version's configuration holds secret values
    /// ([`crate::configuration::secret_paths`]).
    pub fn secret_configuration_paths(&self) -> Vec<crate::configuration::SecretPath> {
        crate::configuration::secret_paths(
            self.configuration_schema.as_ref(),
            self.admin
                .as_ref()
                .and_then(|admin| admin.settings.as_ref())
                .map(|settings| &settings.ui_hints),
        )
    }

    /// What the plugin runs with: `configuration` with every key a feature gates set to that
    /// feature's state. `features` is by feature id; a missing key is unpublished and closed.
    pub fn effective_configuration(
        &self,
        configuration: &Value,
        features: &BTreeMap<String, bool>,
    ) -> Value {
        let Some(values) = configuration.as_object() else {
            return configuration.clone();
        };
        let mut next = values.clone();
        for feature in &self.features {
            if let Some(name) = &feature.gates.configuration {
                let enabled = features.get(&feature.id).copied().unwrap_or(false);
                next.insert(name.clone(), Value::Bool(enabled));
            }
        }
        Value::Object(next)
    }

    pub fn operation(&self, wire_id: &str) -> Option<&PluginOperation> {
        self.capabilities.iter().find_map(|cap| {
            wire_id
                .strip_prefix(&format!("{}.", cap.id))
                .and_then(|id| cap.operations.iter().find(|op| op.id == id))
        })
    }

    pub fn operation_ids(&self) -> BTreeSet<String> {
        self.capabilities
            .iter()
            .flat_map(|cap| {
                cap.operations
                    .iter()
                    .map(|op| format!("{}.{}", cap.id, op.id))
            })
            .collect()
    }
}

pub fn is_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A package path below `assets/`: relative, no empty, `.` or `..` segment,
/// portable file name characters only.
pub fn is_asset_path(value: &str) -> bool {
    value.len() <= MAX_ASSET_PATH_BYTES
        && value.strip_prefix("assets/").is_some_and(|rest| {
            rest.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
            })
        })
}

fn identifier(value: &str, field: &str) -> Result<()> {
    ensure(
        !value.is_empty()
            && value.len() <= 192
            && value.split('.').all(|part| {
                part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                    && part.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
                    })
            }),
        field,
        "expected bounded lowercase namespaced identifier",
    )
}

fn bounded(value: &str, max: usize, field: &str) -> Result<()> {
    ensure(
        !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control),
        field,
        "empty, control character or too long",
    )
}

/// Text per locale: `0..=16` entries of [`bounded`] text.
fn localized(text: &BTreeMap<String, String>, max: usize, field: &str) -> Result<()> {
    ensure(text.len() <= 16, field, "too many locales")?;
    for (locale, value) in text {
        ensure(
            !locale.is_empty()
                && locale.len() <= 32
                && locale
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            field,
            "invalid locale",
        )?;
        bounded(value, max, field)?;
    }
    Ok(())
}

fn version_range(range: &str, field: &str) -> Result<()> {
    ensure(
        range.len() <= 128 && semver::VersionReq::parse(range).is_ok(),
        field,
        "invalid semantic version range",
    )
}

fn unique_identifiers(values: &[String], field: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        identifier(value, field)?;
        ensure(seen.insert(value), field, "duplicate identifier")?;
    }
    Ok(())
}

fn permission_refs(values: &[String], declared: &BTreeSet<&str>, field: &str) -> Result<()> {
    unique_identifiers(values, field)?;
    ensure(
        values.iter().all(|id| declared.contains(id.as_str())),
        field,
        "undeclared permission",
    )
}

fn schema(value: &Value, field: &str) -> Result<()> {
    ensure(
        value.is_object()
            && (value.get("type").is_some()
                || value.get("$ref").is_some()
                || value.get("oneOf").is_some()),
        field,
        "expected typed JSON schema",
    )?;
    fn local_refs(value: &Value, field: &str, budget: &mut usize) -> Result<()> {
        ensure(*budget > 0, field, "schema complexity limit exceeded")?;
        *budget -= 1;
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    if key == "$ref" || key == "$dynamicRef" {
                        ensure(
                            child
                                .as_str()
                                .is_some_and(|reference| reference.starts_with('#')),
                            field,
                            "external schema references are forbidden",
                        )?;
                    }
                    local_refs(child, field, budget)?;
                }
            }
            Value::Array(array) => {
                for child in array {
                    local_refs(child, field, budget)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    local_refs(value, field, &mut 8192)
}

fn ensure(condition: bool, field: &str, reason: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(ProtocolError::InvalidManifest {
            field: field.to_owned(),
            reason: reason.to_owned(),
        })
    }
}
