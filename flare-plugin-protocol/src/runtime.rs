//! Leased desired-state projections and their resolved runtime form.
//!
//! [`PluginProjection`] is what the platform sends: who may run what, by manifest digest. It holds
//! no manifest, deployment or configuration, so a snapshot stays small. Transport adapters MUST
//! authenticate the platform worker and its exact tenant before accepting one.
//!
//! [`PluginRuntimeSnapshot`] is what Core works with after resolving a projection against its own
//! registry: parsed manifests, deployments and acknowledged configuration. It never goes on the
//! wire. Publisher verification belongs to the registry; [`ResolvedPlugin::new`] checks exact
//! digests, immutable identities and least-privilege bounds once per resolution.
use crate::{
    PluginManifest, ProtocolError, Result,
    authorization::{PluginInvocation, TenantPluginGrant},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

pub const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SNAPSHOT_PLUGINS: usize = 128;
/// Bindings of one projection: a binding stays listed from its approval until Core confirmed its
/// last deny, so an application holds more of them than serving plugins.
pub const MAX_SNAPSHOT_BINDINGS: usize = 256;
pub const MAX_PROJECTION_LEASE_MS: i64 = 600_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginDeployment {
    pub deployment_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub manifest_digest: String,
    pub artifact_digest: String,
    /// Operator-owned shared service URL. Never supplied by a tenant/manifest.
    pub grpc_authority: String,
}
impl PluginDeployment {
    pub fn validate(&self, manifest: &PluginManifest, digest: &str) -> Result<()> {
        if self.deployment_id.is_empty()
            || self.deployment_id.len() > 128
            || self.plugin_id != manifest.plugin_id
            || self.plugin_version != manifest.version
            || self.manifest_digest != digest
            || manifest
                .artifact
                .as_ref()
                .is_none_or(|a| a.digest != self.artifact_digest)
            || !matches!(
                manifest.runtime,
                crate::manifest::PluginRuntime::Grpc { .. }
                    | crate::manifest::PluginRuntime::Wasm { .. }
            )
            || !valid_grpc_authority(&self.grpc_authority)
        {
            return Err(ProtocolError::Denied("deployment binding mismatch"));
        }
        Ok(())
    }
}
pub fn valid_grpc_authority(value: &str) -> bool {
    let Some(authority) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return false;
    };
    !authority.is_empty()
        && value.len() <= 512
        && authority.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        })
}
pub fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
pub fn configuration_digest(value: &serde_json::Value) -> Result<String> {
    if !value.is_object() {
        return Err(ProtocolError::Denied("configuration must be an object"));
    }
    // serde_json's preserve_order is a Cargo feature selected by consumers;
    // canonical digests must not depend on that feature or input key order.
    fn canonical(value: &serde_json::Value, depth: usize) -> Result<serde_json::Value> {
        if depth > 32 {
            return Err(ProtocolError::Denied("configuration nesting exceeds limit"));
        }
        Ok(match value {
            serde_json::Value::Object(object) => {
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort_unstable();
                let mut ordered = serde_json::Map::new();
                for key in keys {
                    ordered.insert(key.clone(), canonical(&object[key], depth + 1)?);
                }
                serde_json::Value::Object(ordered)
            }
            serde_json::Value::Array(values) => serde_json::Value::Array(
                values
                    .iter()
                    .map(|v| canonical(v, depth + 1))
                    .collect::<Result<_>>()?,
            ),
            _ => value.clone(),
        })
    }
    let bytes =
        serde_json::to_vec(&canonical(value, 0)?).map_err(|_| ProtocolError::InvalidJson)?;
    if bytes.len() > 16384 {
        return Err(ProtocolError::Denied("configuration exceeds limit"));
    }
    Ok(digest_bytes(&bytes))
}
/// What the platform says about an installation besides its grant. `uninstalled` keeps a recovery
/// surface until `export_until_ms`; `revoked` carries no authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectedLifecycle {
    Installed,
    Uninstalled,
    Revoked,
}

/// One plugin of a [`PluginProjection`]. The manifest digest, `enabled` and `suspended` live in the
/// grant, the only place a host reads them; the accessors below name them for readers of the
/// projection. The grant's `revision` is the platform's desired revision: Core serves the revision
/// and configuration the runtime last acknowledged, from its own state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectedPlugin {
    pub grant: TenantPluginGrant,
    pub lifecycle: ProjectedLifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_until_ms: Option<i64>,
    /// The application's feature states by [`crate::manifest::PluginFeature::id`].
    /// A feature without a key is unpublished and closed (see
    /// [`PluginManifest::closed_operations`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub features: BTreeMap<String, bool>,
}
impl ProjectedPlugin {
    pub fn manifest_digest(&self) -> &str {
        &self.grant.manifest_digest
    }
    pub fn enabled(&self) -> bool {
        self.grant.enabled
    }
    pub fn suspended(&self) -> bool {
        self.grant.suspended
    }
}

/// Where a binding is on its way, for the reconciler. `pending` has not been acknowledged yet;
/// `uninstalled` keeps its recovery surface until `export_until_ms`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingLifecycle {
    Pending,
    Installed,
    Uninstalled,
    Revoked,
}

/// What Core's reconciler converges one binding to. [`PluginProjection::plugins`] says what may be
/// served; this says what the plugin process must be told, including the bindings that serve
/// nothing: one still installing, a disabled one with no recovery surface, and a terminal one whose
/// deny Core has not confirmed. The configuration is not here: Core holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectedBinding {
    pub binding_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub manifest_digest: String,
    pub organization_id: String,
    pub revision: u64,
    pub lifecycle: BindingLifecycle,
    /// The tenant's choice. Serving also needs `lifecycle`, `suspended` and `owned`.
    pub enabled: bool,
    /// Suspended by the platform: the binding or its version.
    pub suspended: bool,
    pub configured: bool,
    /// The binding's organization still owns the application. False is a deny.
    pub owned: bool,
    /// The target of an offline upgrade whose data migration has not run: a disabled one cannot
    /// validate its configuration against the old schema yet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_until_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub approved_operations: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub approved_permissions: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub approved_data_scopes: BTreeSet<String>,
    /// As [`ProjectedPlugin::features`]; the configuration-gated ones decide the effective
    /// configuration ([`crate::PluginManifest::effective_configuration`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub features: BTreeMap<String, bool>,
}

/// The application's desired plugin state, as Core persists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginProjection {
    pub tenant_id: String,
    /// Catalog-owned immutable application owner; never an employee membership or caller input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    pub version: u64,
    pub issued_at_ms: i64,
    pub valid_until_ms: i64,
    pub plugins: Vec<ProjectedPlugin>,
    /// What the reconciler converges to. A binding missing here keeps what Core last held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<ProjectedBinding>,
}
impl PluginProjection {
    pub fn from_json(bytes: &[u8], now_ms: i64) -> Result<Self> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(ProtocolError::Denied("runtime projection exceeds limit"));
        }
        let projection: Self =
            serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidJson)?;
        projection.validate(now_ms)?;
        Ok(projection)
    }
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        valid_lease(
            &self.tenant_id,
            self.organization_id.as_deref(),
            self.version,
            (self.issued_at_ms, self.valid_until_ms),
            self.plugins.len(),
            now_ms,
        )?;
        let mut ids = BTreeSet::new();
        let mut active = BTreeSet::new();
        for plugin in &self.plugins {
            let g = &plugin.grant;
            if g.grant_id.is_empty()
                || g.grant_id.len() > 128
                || g.revision == 0
                || g.revision > i64::MAX as u64
                || !crate::manifest::is_sha256_digest(&g.manifest_digest)
            {
                return Err(ProtocolError::Denied("invalid projected grant"));
            }
            grant_in_lease(
                g,
                &self.tenant_id,
                (self.issued_at_ms, self.valid_until_ms),
                &mut ids,
                &mut active,
            )?;
        }
        if self.bindings.len() > MAX_SNAPSHOT_BINDINGS {
            return Err(ProtocolError::Denied("runtime projection exceeds limit"));
        }
        let mut bindings = BTreeSet::new();
        for binding in &self.bindings {
            if binding.binding_id.is_empty()
                || binding.binding_id.len() > 128
                || binding.organization_id.len() > 128
                || binding.revision == 0
                || binding.revision > i64::MAX as u64
                || !crate::manifest::is_sha256_digest(&binding.manifest_digest)
                || !bindings.insert(&binding.binding_id)
            {
                return Err(ProtocolError::Denied("invalid projected binding"));
            }
        }
        Ok(())
    }
}

fn valid_lease(
    tenant_id: &str,
    organization_id: Option<&str>,
    version: u64,
    (issued_at_ms, valid_until_ms): (i64, i64),
    plugins: usize,
    now_ms: i64,
) -> Result<()> {
    if tenant_id.is_empty()
        || organization_id.is_some_and(|id| {
            id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        || tenant_id.len() > 32
        || !tenant_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        || version == 0
        || version > i64::MAX as u64
        || issued_at_ms < 0
        || issued_at_ms > now_ms.saturating_add(crate::authorization::CLOCK_SKEW_MARGIN_MS)
        || valid_until_ms <= now_ms
        || valid_until_ms <= issued_at_ms
        || valid_until_ms.saturating_sub(issued_at_ms) > MAX_PROJECTION_LEASE_MS
        || plugins > MAX_SNAPSHOT_PLUGINS
    {
        return Err(ProtocolError::Denied(
            "invalid runtime projection lease or identity",
        ));
    }
    Ok(())
}

fn grant_in_lease<'a>(
    g: &'a TenantPluginGrant,
    tenant_id: &str,
    (issued_at_ms, valid_until_ms): (i64, i64),
    ids: &mut BTreeSet<&'a String>,
    active: &mut BTreeSet<&'a String>,
) -> Result<()> {
    if g.tenant_id != tenant_id
        || g.valid_from_ms < issued_at_ms
        || g.valid_until_ms > valid_until_ms
        || g.valid_from_ms >= g.valid_until_ms
        || !ids.insert(&g.grant_id)
        || (g.enabled && !g.revoked && !active.insert(&g.plugin_id))
    {
        return Err(ProtocolError::Denied(
            "projection tenant, duplicate grant or lease mismatch",
        ));
    }
    Ok(())
}

/// What a plugin's closed features take away, worked out once per resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClosedFeatures {
    pub operations: BTreeSet<String>,
    pub service_operations: BTreeSet<String>,
    pub contributions: BTreeSet<String>,
}

/// A projected plugin joined with Core's registry. Built only through [`ResolvedPlugin::new`], which
/// checks the identities and bounds, so a holder reads the manifest without parsing or re-checking.
#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub grant: TenantPluginGrant,
    pub deployment: PluginDeployment,
    pub configuration_digest: String,
    /// The client-visible subset of the configuration whose digest is `configuration_digest`: its
    /// top-level properties the manifest's `configurationSchema` marks `x-flare-client-visible`
    /// (see [`PluginManifest::client_visible_configuration`]). Hosts send it with the client
    /// catalog and still keep only the keys the manifest marks visible.
    pub client_configuration: serde_json::Map<String, serde_json::Value>,
    /// The application's feature states by [`crate::manifest::PluginFeature::id`].
    pub features: BTreeMap<String, bool>,
    manifest: Arc<PluginManifest>,
    closed: ClosedFeatures,
}
impl ResolvedPlugin {
    /// `manifest` comes from a [`ManifestCache`] or another check of `grant.manifest_digest`.
    pub fn new(
        grant: TenantPluginGrant,
        manifest: Arc<PluginManifest>,
        deployment: PluginDeployment,
        configuration_digest: String,
        client_configuration: serde_json::Map<String, serde_json::Value>,
        features: BTreeMap<String, bool>,
    ) -> Result<Self> {
        deployment.validate(&manifest, &grant.manifest_digest)?;
        let g = &grant;
        let operations = manifest.operation_ids();
        let permissions: BTreeSet<_> = manifest.permissions.iter().map(|p| p.id.clone()).collect();
        let scopes: BTreeSet<_> = manifest
            .permissions
            .iter()
            .flat_map(|p| p.data_scopes.iter().cloned())
            .collect();
        if g.grant_id.is_empty()
            || g.grant_id.len() > 128
            || g.revision == 0
            || g.revision > i64::MAX as u64
            || g.plugin_id != manifest.plugin_id
            || g.plugin_version != manifest.version
            || !g.approved_operations.is_subset(&operations)
            || !g.approved_permissions.is_subset(&permissions)
            || !g.approved_data_scopes.is_subset(&scopes)
            || !crate::manifest::is_sha256_digest(&configuration_digest)
        {
            return Err(ProtocolError::Denied("projected grant exceeds manifest"));
        }
        let closed = ClosedFeatures {
            operations: manifest.closed_operations(&features),
            service_operations: manifest.closed_service_operations(&features),
            contributions: manifest.closed_contributions(&features),
        };
        Ok(Self {
            grant,
            deployment,
            configuration_digest,
            client_configuration,
            features,
            manifest,
            closed,
        })
    }
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    pub fn closed(&self) -> &ClosedFeatures {
        &self.closed
    }
    pub fn authorize(&self, invocation: &PluginInvocation<'_>) -> Result<()> {
        self.grant
            .authorize_manifest(&self.manifest, &self.grant.manifest_digest, invocation)
    }
}

/// A projection resolved against Core's registry. In-process only.
#[derive(Debug, Clone)]
pub struct PluginRuntimeSnapshot {
    pub tenant_id: String,
    /// Catalog-owned immutable application owner; never an employee membership or caller input.
    pub organization_id: Option<String>,
    pub version: u64,
    pub issued_at_ms: i64,
    pub valid_until_ms: i64,
    pub plugins: Vec<ResolvedPlugin>,
}
impl PluginRuntimeSnapshot {
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        valid_lease(
            &self.tenant_id,
            self.organization_id.as_deref(),
            self.version,
            (self.issued_at_ms, self.valid_until_ms),
            self.plugins.len(),
            now_ms,
        )?;
        let mut ids = BTreeSet::new();
        let mut active = BTreeSet::new();
        let mut namespaces: BTreeMap<&str, &str> = BTreeMap::new();
        for plugin in &self.plugins {
            let g = &plugin.grant;
            grant_in_lease(
                g,
                &self.tenant_id,
                (self.issued_at_ms, self.valid_until_ms),
                &mut ids,
                &mut active,
            )?;
            for capability in &plugin.manifest().capabilities {
                if namespaces.iter().any(|(namespace, owner)| {
                    *owner != g.plugin_id
                        && (*namespace == capability.id
                            || namespace.starts_with(&format!("{}.", capability.id))
                            || capability.id.starts_with(&format!("{namespace}.")))
                }) {
                    return Err(ProtocolError::NamespaceConflict(capability.id.clone()));
                }
                namespaces.insert(&capability.id, &g.plugin_id);
            }
        }
        Ok(())
    }
}

/// Parsed manifests by digest. A digest names exact bytes, so an entry is never invalidated, and a
/// reader pays for parsing once per process. The registry holds few versions: nothing is evicted.
#[derive(Debug, Default)]
pub struct ManifestCache {
    parsed: RwLock<HashMap<String, Arc<PluginManifest>>>,
}
impl ManifestCache {
    pub fn get(&self, digest: &str) -> Option<Arc<PluginManifest>> {
        self.parsed
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(digest)
            .cloned()
    }
    /// Parses `raw`, which must hash to `digest`, and keeps it.
    pub fn insert(&self, digest: &str, raw: &[u8]) -> Result<Arc<PluginManifest>> {
        if digest_bytes(raw) != digest {
            return Err(ProtocolError::Denied("manifest digest mismatch"));
        }
        let manifest = Arc::new(PluginManifest::from_json(raw)?);
        self.parsed
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(digest.to_owned(), Arc::clone(&manifest));
        Ok(manifest)
    }
}
