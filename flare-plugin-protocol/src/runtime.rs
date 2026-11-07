//! Full, leased runtime projections. Transport adapters MUST authenticate the
//! platform worker and its exact tenant before accepting these values. Publisher
//! verification and approval belong to the source; consumers verify exact bytes,
//! immutable identities and least-privilege bounds without querying a market.
use crate::{
    PluginManifest, ProtocolError, Result,
    authorization::{PluginInvocation, TenantPluginGrant},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SNAPSHOT_PLUGINS: usize = 128;
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectedPlugin {
    pub grant: TenantPluginGrant,
    /// Exact publisher-signed bytes retained by the control plane.
    pub manifest_json: String,
    pub deployment: PluginDeployment,
    pub configuration_digest: String,
    /// The client-visible subset of the tenant configuration whose digest is
    /// `configuration_digest`: its top-level properties the manifest's
    /// `configurationSchema` marks `x-flare-client-visible` (see
    /// [`PluginManifest::client_visible_configuration`]). Hosts send it with the
    /// client catalog and still keep only the keys the manifest marks visible.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub client_configuration: serde_json::Map<String, serde_json::Value>,
}
impl ProjectedPlugin {
    pub fn manifest(&self) -> Result<PluginManifest> {
        let manifest = PluginManifest::from_json(self.manifest_json.as_bytes())?;
        let digest = digest_bytes(self.manifest_json.as_bytes());
        self.deployment.validate(&manifest, &digest)?;
        let g = &self.grant;
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
            || g.manifest_digest != digest
            || !g.approved_operations.is_subset(&operations)
            || !g.approved_permissions.is_subset(&permissions)
            || !g.approved_data_scopes.is_subset(&scopes)
            || !crate::manifest::is_sha256_digest(&self.configuration_digest)
        {
            return Err(ProtocolError::Denied("projected grant exceeds manifest"));
        }
        Ok(manifest)
    }
    pub fn authorize(&self, invocation: &PluginInvocation<'_>) -> Result<PluginManifest> {
        let manifest = self.manifest()?;
        self.grant
            .authorize_manifest(&manifest, &self.grant.manifest_digest, invocation)?;
        Ok(manifest)
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginRuntimeSnapshot {
    pub tenant_id: String,
    /// Catalog-owned immutable application owner; never an employee membership or caller input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    pub version: u64,
    pub issued_at_ms: i64,
    pub valid_until_ms: i64,
    pub plugins: Vec<ProjectedPlugin>,
}
impl PluginRuntimeSnapshot {
    pub fn from_json(bytes: &[u8], now_ms: i64) -> Result<Self> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(ProtocolError::Denied("runtime projection exceeds limit"));
        }
        let snapshot: Self =
            serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidJson)?;
        snapshot.validate(now_ms)?;
        Ok(snapshot)
    }
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        if self.tenant_id.is_empty()
            || self.organization_id.as_deref().is_some_and(|id| {
                id.is_empty()
                    || id.len() > 128
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            })
            || self.tenant_id.len() > 32
            || !self
                .tenant_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            || self.version == 0
            || self.version > i64::MAX as u64
            || self.issued_at_ms < 0
            || self.issued_at_ms > now_ms.saturating_add(30_000)
            || self.valid_until_ms <= now_ms
            || self.valid_until_ms <= self.issued_at_ms
            || self.valid_until_ms.saturating_sub(self.issued_at_ms) > MAX_PROJECTION_LEASE_MS
            || self.plugins.len() > MAX_SNAPSHOT_PLUGINS
        {
            return Err(ProtocolError::Denied(
                "invalid runtime projection lease or identity",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut active = BTreeSet::new();
        let mut namespaces: BTreeMap<String, String> = BTreeMap::new();
        for plugin in &self.plugins {
            let g = &plugin.grant;
            let manifest = plugin.manifest()?;
            if g.tenant_id != self.tenant_id
                || g.valid_from_ms < self.issued_at_ms
                || g.valid_until_ms > self.valid_until_ms
                || g.valid_from_ms >= g.valid_until_ms
                || !ids.insert(&g.grant_id)
                || (g.enabled && !g.revoked && !active.insert(&g.plugin_id))
            {
                return Err(ProtocolError::Denied(
                    "projection tenant, duplicate grant or lease mismatch",
                ));
            }
            for capability in manifest.capabilities {
                if namespaces.iter().any(|(namespace, owner)| {
                    owner != &g.plugin_id
                        && (namespace == &capability.id
                            || namespace.starts_with(&format!("{}.", capability.id))
                            || capability.id.starts_with(&format!("{namespace}.")))
                }) {
                    return Err(ProtocolError::NamespaceConflict(capability.id));
                }
                namespaces.insert(capability.id, g.plugin_id.clone());
            }
        }
        Ok(())
    }
}
