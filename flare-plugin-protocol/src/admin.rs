//! Admin Extension Contract: the management surface a plugin declares in its
//! manifest (`admin`). The plugin only declares; the platform validates,
//! projects the permissions into roles, authorizes and audits; Core executes;
//! the console renders every page with the kit's generic plugin renderer. A
//! plugin ships no front-end code (no HTML, URL or script).
//!
//! ```jsonc
//! "admin": {
//!   "contractVersion": 1,
//!   "permissions": [{"id": "read", "label": {"zh-CN": "查看投票"}, "risk": "low",
//!                    "defaultRoles": ["org_owner", "org_admin"]}],
//!   "operations": [{"id": "flare.poll.admin.list", "mode": "query", "permission": "read"},
//!                  {"id": "flare.poll.admin.close", "mode": "command", "permission": "manage",
//!                   "requiresReason": true, "auditTarget": "/pollId"}],
//!   "navigation": [{"id": "polls", "section": "plugins", "label": {"zh-CN": "投票"},
//!                   "icon": "poll", "order": 110, "page": "polls"}],
//!   "pages": [{"id": "polls", "kind": "list", "permission": "read", "document": {..}}],
//!   "settings": {"uiHints": {"maxOptions": {"widget": "number", "group": "limits"}}},
//!   "events": {"emit": [{"id": "flare.poll.closed", "schemaVersion": 1}], "subscribe": []},
//!   "lifecycle": {"migrations": [{"from": ">=0.1.0, <0.2.0", "to": "0.2.0",
//!                                 "mode": "offline", "reversible": false}],
//!                 "uninstall": {"behavior": "export-then-retain", "export": true},
//!                 "configurationCarryOver": true}
//! }
//! ```
//!
//! [`crate::PluginManifest::validate`] checks everything the manifest alone
//! decides (see [`PluginAdmin::validate`]). What needs the registry or the
//! running plugin stays with the platform: that the migrations cover every
//! published version ([`PluginAdmin::migration_from`] answers per version),
//! that an icon exists in the kit, and that a plugin declaring
//! `uninstall.export` implements the data export protocol.
use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::{OperationMode, PluginManifest, RiskLevel, UninstallBehavior};
use crate::ui::{ResolvedPluginView, UiActionTarget, UiDetail, UiDocument, resolve_document};
use crate::{ProtocolError, Result};

/// The only [`PluginAdmin::contract_version`].
pub const ADMIN_CONTRACT_VERSION: u32 = 1;
const MAX_ADMIN_ITEMS: usize = 64;
const MAX_ADMIN_LABEL_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginAdmin {
    pub contract_version: u32,
    /// Staff permissions of this plugin, granted as
    /// [`crate::invocation::plugin_permission`] codes
    /// (`plugin:<pluginId>:<id>`).
    pub permissions: Vec<AdminPermission>,
    /// The admin operations, each a declared capability operation that no
    /// client contribution uses.
    pub operations: Vec<AdminOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub navigation: Vec<AdminNavigation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<AdminPage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<AdminSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<AdminEvents>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<AdminLifecycle>,
}

/// Display text per locale (`zh-CN`, `en-US`): `1..=16` entries, each at most
/// 64 bytes without control characters.
pub type AdminLabel = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminPermission {
    /// `[a-z][a-z0-9_-]{0,31}`, unique.
    pub id: String,
    pub label: AdminLabel,
    pub risk: RiskLevel,
    /// Built-in roles the platform grants it to on install (`org_owner`, …);
    /// tenants may assign it to other roles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminOperation {
    /// Wire operation declared in `capabilities`.
    pub id: String,
    /// Must equal the capability operation's `mode`.
    pub mode: OperationMode,
    /// One of [`PluginAdmin::permissions`]: the staff ticket must carry its code.
    pub permission: String,
    /// Authority required for this operation. An application grant never grants organization work.
    #[serde(default, skip_serializing_if = "AdminOperationScope::is_application")]
    pub scope: AdminOperationScope,
    /// The console asks the staff member for a reason; Core records it in the
    /// audit with the call.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub requires_reason: bool,
    /// JSON pointer into the operation input naming the audited target (the
    /// poll id of a close).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_target: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AdminOperationScope {
    #[default]
    Application,
    Organization,
}
impl AdminOperationScope {
    pub fn is_application(&self) -> bool {
        *self == Self::Application
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AdminSection {
    Plugins,
    Operations,
    Analytics,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AdminAudience {
    /// Tenant staff (and platform staff acting for the tenant).
    #[default]
    Tenant,
    /// Platform staff only.
    Platform,
}
impl AdminAudience {
    fn is_tenant(&self) -> bool {
        *self == Self::Tenant
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminNavigation {
    pub id: String,
    pub section: AdminSection,
    pub label: AdminLabel,
    /// Kit icon name ([`crate::manifest::UI_ICON_PATTERN`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Ascending position in its section (default 1000).
    #[serde(default = "default_order", skip_serializing_if = "is_default_order")]
    pub order: i32,
    /// One of [`PluginAdmin::pages`].
    pub page: String,
    /// A workspace's independently authorized pages. Empty keeps a single-page entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<AdminNavigationPage>,
    #[serde(default, skip_serializing_if = "AdminAudience::is_tenant")]
    pub audience: AdminAudience,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AdminNavigationPlacement {
    #[default]
    Tab,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminNavigationPage {
    pub page: String,
    pub label: AdminLabel,
    #[serde(default)]
    pub placement: AdminNavigationPlacement,
    /// Context page whose toolbar offers this auxiliary page; never grants access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_page: Option<String>,
}

impl AdminNavigation {
    pub fn contains_page(&self, page: &str) -> bool {
        self.page == page || self.pages.iter().any(|member| member.page == page)
    }
}
fn default_order() -> i32 {
    crate::manifest::DEFAULT_UI_ORDER
}
fn is_default_order(order: &i32) -> bool {
    *order == crate::manifest::DEFAULT_UI_ORDER
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AdminPageKind {
    /// A rows block (filters, row actions, paging) and an optional detail.
    List,
    /// A document over one result; no rows.
    Detail,
    /// Action forms; no rows.
    Form,
    /// Kit whitelisted widgets ([`AdminWidgetKind`]).
    Dashboard,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminPage {
    pub id: String,
    pub kind: AdminPageKind,
    /// One of [`PluginAdmin::permissions`]: needed to see the page.
    pub permission: String,
    /// Rendered with the generic plugin renderer; every action, rows source
    /// and the detail call admin operations only.
    pub document: UiDocument,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<UiDetail>,
    /// `dashboard` only (`1..=12`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widgets: Vec<AdminWidget>,
}

impl AdminPage {
    /// The page resolved against its rows source result (`{}` before the
    /// first read), the way the console's generic plugin renderer shows it.
    pub fn resolve(&self, data: &Value, locale: &str, viewer: &str) -> ResolvedPluginView {
        resolve_document(&self.document, data, locale, viewer, self.detail.as_ref())
    }
    /// The detail document resolved against one page of the detail operation's
    /// result (with `nextCursor` while a paged detail has more).
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
    /// [`UiDocument::retain_targets`] over the document and the detail; the
    /// detail is dropped when `keep` rejects its operation (an installation
    /// that did not approve it).
    pub fn retain_targets(&mut self, mut keep: impl FnMut(&UiActionTarget) -> bool) {
        self.document.retain_targets(&mut keep);
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

/// A dashboard widget from the kit's whitelist; never plugin code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AdminWidgetKind {
    #[serde(rename = "kit:stat")]
    Stat,
    #[serde(rename = "kit:chart.timeseries")]
    ChartTimeseries,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminWidget {
    pub widget: AdminWidgetKind,
    pub label: AdminLabel,
    /// An admin `query` operation, called with `{}`.
    pub source: String,
    /// JSON pointer into its result: a number (`kit:stat`) or an array of
    /// `{atMs, value}` points (`kit:chart.timeseries`).
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminSettings {
    /// Presentation of the configuration form generated from
    /// `configurationSchema`, by top-level property.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ui_hints: BTreeMap<String, AdminUiHint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AdminHintWidget {
    Text,
    Textarea,
    Number,
    Switch,
    Select,
    MultiSelect,
    Secret,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminUiHint {
    pub widget: AdminHintWidget,
    /// Group key the form puts the field under (`[a-z][a-z0-9_-]{0,31}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminEvents {
    /// Events the plugin emits (each one of the manifest's `events`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emit: Vec<AdminEventRef>,
    /// Events the plugin wants; only the shape is declared, Core does not deliver them yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscribe: Vec<AdminEventRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminEventRef {
    pub id: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AdminMigrationMode {
    /// The platform disables the plugin, runs the migration, waits for the
    /// runtime ACK, then enables it.
    Offline,
    /// Runs while the plugin serves.
    Online,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminMigration {
    /// Semantic version range of the installed version it upgrades.
    pub from: String,
    /// The version it upgrades to; at most this manifest's version.
    pub to: String,
    pub mode: AdminMigrationMode,
    /// `false`: once run, the platform refuses to enable an older version.
    pub reversible: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminUninstall {
    pub behavior: UninstallBehavior,
    /// The plugin implements the data export protocol (`flare.plugin.data.v1`),
    /// so tenants can export before and for a while after uninstalling.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub export: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminLifecycle {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub migrations: Vec<AdminMigration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uninstall: Option<AdminUninstall>,
    /// An upgrade keeps the tenant configuration (new keys take their default,
    /// removed keys are dropped and audited). Default true.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub configuration_carry_over: bool,
}
fn yes() -> bool {
    true
}
fn is_true(value: &bool) -> bool {
    *value
}

fn segment(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
fn label(label: &AdminLabel) -> bool {
    !label.is_empty()
        && label.len() <= 16
        && label.iter().all(|(locale, text)| {
            !locale.is_empty()
                && locale.len() <= 32
                && locale
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !text.trim().is_empty()
                && text.len() <= MAX_ADMIN_LABEL_BYTES
                && !text.chars().any(char::is_control)
        })
}
fn unique<'a>(ids: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    ids.into_iter().all(|id| seen.insert(id))
}
fn invalid(reason: &str) -> ProtocolError {
    ProtocolError::InvalidManifest {
        field: "admin".to_owned(),
        reason: reason.to_owned(),
    }
}
fn ensure(condition: bool, reason: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(reason))
    }
}

impl PluginAdmin {
    /// Wire ids of the admin operations.
    pub fn operation_ids(&self) -> BTreeSet<String> {
        self.operations.iter().map(|op| op.id.clone()).collect()
    }
    pub fn operation(&self, id: &str) -> Option<&AdminOperation> {
        self.operations.iter().find(|op| op.id == id)
    }
    pub fn page(&self, id: &str) -> Option<&AdminPage> {
        self.pages.iter().find(|page| page.id == id)
    }
    /// The migration that upgrades an installed `version`, if one declares it.
    pub fn migration_from(&self, version: &semver::Version) -> Option<&AdminMigration> {
        self.lifecycle
            .as_ref()?
            .migrations
            .iter()
            .find(|m| semver::VersionReq::parse(&m.from).is_ok_and(|range| range.matches(version)))
    }

    /// Everything the manifest alone decides:
    /// - `contractVersion` is [`ADMIN_CONTRACT_VERSION`]; at most 64 of each list;
    /// - permissions: unique ids, labels, default roles;
    /// - operations: unique, declared in `capabilities` with the same `mode`,
    ///   a declared permission, `auditTarget` a pointer, and none of them used
    ///   by a client contribution (an admin operation is never a user one);
    /// - navigation: unique ids, an existing page, a kit icon name;
    /// - pages: unique ids, a declared permission, a valid document whose
    ///   actions, rows source and detail call admin operations only; `list`
    ///   has one rows block, `detail`/`form` none, `dashboard` 1..=12 widgets
    ///   reading admin `query` operations and no rows;
    /// - `settings.uiHints` keys are top-level `configurationSchema`
    ///   properties;
    /// - `events.emit` are declared manifest events; `subscribe` shape only;
    /// - migrations: a version range `from`, a version `to` not above the
    ///   manifest version, no duplicate `from`.
    pub fn validate(&self, manifest: &PluginManifest) -> Result<()> {
        ensure(
            self.contract_version == ADMIN_CONTRACT_VERSION,
            "unsupported contractVersion",
        )?;
        ensure(
            [
                self.permissions.len(),
                self.operations.len(),
                self.navigation.len(),
                self.pages.len(),
            ]
            .iter()
            .all(|n| *n <= MAX_ADMIN_ITEMS),
            "too many admin items",
        )?;
        ensure(
            self.permissions.iter().all(|p| {
                segment(&p.id)
                    && crate::invocation::plugin_permission(&manifest.plugin_id, &p.id).is_some()
                    && label(&p.label)
                    && p.default_roles.len() <= 16
                    && p.default_roles.iter().all(|role| segment(role))
                    && unique(p.default_roles.iter().map(String::as_str))
            }) && unique(self.permissions.iter().map(|p| p.id.as_str())),
            "invalid permissions",
        )?;
        let permission = |id: &str| self.permissions.iter().any(|p| p.id == id);
        ensure(
            unique(self.operations.iter().map(|op| op.id.as_str())),
            "duplicate operation",
        )?;
        for op in &self.operations {
            ensure(
                self.operations
                    .iter()
                    .filter(|other| other.permission == op.permission)
                    .all(|other| other.scope == op.scope),
                "one staff permission cannot mix application and organization authority",
            )?;
            let declared = manifest
                .operation(&op.id)
                .ok_or_else(|| invalid("operation must be a declared capability operation"))?;
            ensure(
                declared.mode == op.mode,
                "operation mode must match its declaration",
            )?;
            ensure(
                permission(&op.permission),
                "operation permission must be declared",
            )?;
            ensure(
                op.audit_target.as_deref().is_none_or(crate::ui::valid_path),
                "auditTarget must be a JSON pointer",
            )?;
        }
        let admin_ops = self.operation_ids();
        let user_ops = manifest.client_operation_refs();
        ensure(
            admin_ops.is_disjoint(&user_ops),
            "an admin operation is used by a client contribution",
        )?;
        ensure(
            unique(self.navigation.iter().map(|n| n.id.as_str()))
                && self.navigation.iter().all(|n| {
                    segment(&n.id)
                        && label(&n.label)
                        && self.page(&n.page).is_some()
                        && n.icon
                            .as_deref()
                            .is_none_or(crate::manifest::is_ui_icon_name)
                        && n.pages.len() <= MAX_ADMIN_ITEMS
                        && (n.pages.is_empty() || n.pages.iter().any(|p| p.page == n.page))
                        && unique(n.pages.iter().map(|p| p.page.as_str()))
                        && n.pages.iter().all(|p| {
                            if !label(&p.label) || self.page(&p.page).is_none() {
                                return false;
                            }
                            let mut seen = BTreeSet::from([p.page.as_str()]);
                            let mut parent = p.parent_page.as_deref();
                            while let Some(id) = parent {
                                if !seen.insert(id) {
                                    return false;
                                }
                                let Some(member) = n.pages.iter().find(|member| member.page == id)
                                else {
                                    return false;
                                };
                                parent = member.parent_page.as_deref();
                            }
                            true
                        })
                }),
            "invalid navigation",
        )?;
        let mut workspace_pages = BTreeSet::new();
        ensure(
            self.navigation
                .iter()
                .filter(|n| !n.pages.is_empty())
                .all(|n| {
                    n.pages
                        .iter()
                        .all(|p| workspace_pages.insert(p.page.as_str()))
                }),
            "a page belongs to multiple admin workspaces",
        )?;
        ensure(
            unique(self.pages.iter().map(|p| p.id.as_str())),
            "duplicate page",
        )?;
        for page in &self.pages {
            ensure(segment(&page.id), "invalid page id")?;
            ensure(
                permission(&page.permission),
                "page permission must be declared",
            )?;
            page.document
                .validate()
                .map_err(|_| invalid("invalid page document"))?;
            if let Some(detail) = &page.detail {
                detail
                    .validate()
                    .map_err(|_| invalid("invalid page detail"))?;
                ensure(
                    admin_ops.contains(&detail.operation),
                    "page detail must call an admin operation",
                )?;
            }
            let documents = std::iter::once(&page.document)
                .chain(page.detail.as_ref().map(|detail| &detail.document));
            for document in documents {
                ensure(
                    document.actions.iter().all(|a| match &a.target {
                        UiActionTarget::Operation { operation } => admin_ops.contains(operation),
                        _ => false,
                    }),
                    "page actions must call admin operations",
                )?;
                ensure(
                    document
                        .rows_sources()
                        .all(|source| admin_ops.contains(source)),
                    "rows source must be an admin operation",
                )?;
                ensure(
                    document
                        .inputs
                        .iter()
                        .all(|input| input.id != crate::ui::ROWS_CURSOR_INPUT),
                    "input id `cursor` is reserved",
                )?;
            }
            let rows = page.document.rows_sources().count()
                + page
                    .detail
                    .as_ref()
                    .map_or(0, |d| d.document.rows_sources().count());
            let rows_in_document = page.document.rows_sources().count();
            match page.kind {
                AdminPageKind::List => ensure(
                    rows_in_document == 1 && rows == 1 && page.widgets.is_empty(),
                    "a list page has one rows block",
                )?,
                AdminPageKind::Detail | AdminPageKind::Form => ensure(
                    rows == 0 && page.widgets.is_empty(),
                    "detail and form pages have no rows or widgets",
                )?,
                AdminPageKind::Dashboard => ensure(
                    rows == 0
                        && (1..=12).contains(&page.widgets.len())
                        && page.widgets.iter().all(|w| {
                            label(&w.label)
                                && crate::ui::valid_path(&w.path)
                                && self
                                    .operation(&w.source)
                                    .is_some_and(|op| op.mode == OperationMode::Query)
                        }),
                    "a dashboard has 1..=12 widgets reading admin queries",
                )?,
            }
        }
        if let Some(settings) = &self.settings {
            let properties = manifest
                .configuration_schema
                .as_ref()
                .and_then(|schema| schema.get("properties"))
                .and_then(Value::as_object);
            ensure(
                settings.ui_hints.iter().all(|(key, hint)| {
                    properties.is_some_and(|p| p.contains_key(key))
                        && hint.group.as_deref().is_none_or(segment)
                }),
                "uiHints keys must be configurationSchema properties",
            )?;
        }
        if let Some(events) = &self.events {
            ensure(
                events.emit.len() <= MAX_ADMIN_ITEMS
                    && events.subscribe.len() <= MAX_ADMIN_ITEMS
                    && events.emit.iter().all(|e| {
                        manifest
                            .events
                            .iter()
                            .any(|d| d.id == e.id && d.schema_version == e.schema_version)
                    })
                    && events
                        .subscribe
                        .iter()
                        .all(|e| e.schema_version > 0 && !e.id.is_empty() && e.id.len() <= 192)
                    && unique(events.emit.iter().map(|e| e.id.as_str()))
                    && unique(events.subscribe.iter().map(|e| e.id.as_str())),
                "invalid events",
            )?;
        }
        if let Some(lifecycle) = &self.lifecycle {
            let current = semver::Version::parse(&manifest.version)
                .map_err(|_| invalid("invalid manifest version"))?;
            ensure(
                lifecycle.migrations.len() <= MAX_ADMIN_ITEMS
                    && lifecycle.migrations.iter().all(|m| {
                        m.from.len() <= 128
                            && semver::VersionReq::parse(&m.from).is_ok()
                            && semver::Version::parse(&m.to).is_ok_and(|to| to <= current)
                    })
                    && unique(lifecycle.migrations.iter().map(|m| m.from.as_str())),
                "invalid migrations",
            )?;
        }
        Ok(())
    }
}
