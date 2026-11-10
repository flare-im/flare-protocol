use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::signature::VerifiedManifest;
use crate::{ProtocolError, Result};

/// How far an issuer's clock may run ahead of the host that checks its timestamps. The platform
/// stamps a projection with its database clock and Core compares with its own: a start up to this
/// far in the host's future is already in effect, so a projection change never closes the plugins
/// it leaves unchanged while the two clocks disagree. Only a start is relaxed; an end, a
/// revocation and a disable stay exact. The same bound limits how far ahead `issuedAtMs` may be.
pub const CLOCK_SKEW_MARGIN_MS: i64 = 30_000;

/// A durable approval projection, distinct from tenant enable and emergency
/// flags. The adapter must verify its control-plane signature before use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TenantPluginGrant {
    pub grant_id: String,
    pub revision: u64,
    pub tenant_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub manifest_digest: String,
    pub approved_operations: BTreeSet<String>,
    pub approved_permissions: BTreeSet<String>,
    pub approved_data_scopes: BTreeSet<String>,
    pub enabled: bool,
    pub revoked: bool,
    /// Stopped by the platform, whatever the tenant's `enabled` says.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suspended: bool,
    pub valid_from_ms: i64,
    pub valid_until_ms: i64,
}

/// Identity must originate from verified transport context, never from plugin
/// payload/metadata. This type alone is not authentication authority.
#[derive(Clone, Copy)]
pub struct PluginInvocation<'a> {
    pub tenant_id: &'a str,
    pub actor_id: &'a str,
    pub operation: &'a str,
    pub actor_permissions: &'a BTreeSet<String>,
    pub platform_enabled: bool,
    pub now_ms: i64,
}

impl TenantPluginGrant {
    /// Whether the grant's validity window holds at `now_ms` on this host: its start within
    /// [`CLOCK_SKEW_MARGIN_MS`] of the host clock, its end strictly.
    pub fn in_effect(&self, now_ms: i64) -> bool {
        self.valid_from_ms.saturating_sub(CLOCK_SKEW_MARGIN_MS) <= now_ms
            && now_ms < self.valid_until_ms
    }

    pub fn authorize(
        &self,
        manifest: &VerifiedManifest,
        invocation: &PluginInvocation<'_>,
    ) -> Result<()> {
        let plugin = manifest.manifest();
        self.authorize_manifest(plugin, manifest.digest(), invocation)
    }

    /// Used only after an authenticated control-plane snapshot has been
    /// validated. This checks authorization, never authenticates a projection.
    pub(crate) fn authorize_manifest(
        &self,
        plugin: &crate::PluginManifest,
        digest: &str,
        invocation: &PluginInvocation<'_>,
    ) -> Result<()> {
        if self.grant_id.is_empty() || self.revision == 0 || invocation.actor_id.is_empty() {
            return Err(ProtocolError::Denied("missing identity or grant"));
        }
        if self.tenant_id.is_empty()
            || self.tenant_id == "*"
            || self.tenant_id != invocation.tenant_id
        {
            return Err(ProtocolError::Denied("tenant mismatch"));
        }
        if self.plugin_id != plugin.plugin_id
            || self.plugin_version != plugin.version
            || self.manifest_digest != digest
        {
            return Err(ProtocolError::Denied("version or manifest mismatch"));
        }
        if self.revoked || self.suspended || !self.enabled || !invocation.platform_enabled {
            return Err(ProtocolError::Denied("plugin disabled or revoked"));
        }
        if !self.in_effect(invocation.now_ms) {
            return Err(ProtocolError::Denied(
                "grant projection outside validity window",
            ));
        }
        let op = plugin
            .operation(invocation.operation)
            .ok_or(ProtocolError::Denied("undeclared operation"))?;
        if !self.approved_operations.contains(invocation.operation) {
            return Err(ProtocolError::Denied("operation not approved"));
        }
        for id in &op.permission_ids {
            if !self.approved_permissions.contains(id) || !invocation.actor_permissions.contains(id)
            {
                return Err(ProtocolError::Denied(
                    "permission not approved or actor lacks permission",
                ));
            }
            let permission = plugin
                .permissions
                .iter()
                .find(|permission| &permission.id == id)
                .ok_or(ProtocolError::Denied("undeclared permission"))?;
            if permission
                .data_scopes
                .iter()
                .any(|scope| !self.approved_data_scopes.contains(scope))
            {
                return Err(ProtocolError::Denied("data scope not approved"));
            }
        }
        Ok(())
    }
}
