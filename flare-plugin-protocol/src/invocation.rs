//! Short-lived, target-bound plugin tickets. Deliberately not an IM signed
//! context or a platform control assertion: other IM/Social APIs cannot accept
//! this ticket as a user token. Signing keys must be independent of Cell keys.
//!
//! These credentials share one envelope format and differ in their signing
//! domain, so none verifies as another:
//! - [`PluginCallClaims`]: Core → plugin, a user invoked an operation;
//! - [`PluginStaffCallClaims`]: Core → plugin, a tenant staff member invoked an
//!   admin operation (`admin.operations`) from the console (`dispatch_admin`);
//! - [`PluginSystemCallClaims`]: Core → plugin, the platform itself calls a
//!   platform callback of the plugin (the entity lifecycle `unbound`), with no
//!   user or staff behind it;
//! - [`PluginTenantStateClaims`]: Core → plugin, Core's reconciler delivers one
//!   tenant state to the deployment (`PluginRuntime.ApplyTenant`);
//! - [`PluginDataClaims`]: Core → plugin, Core forwards one platform data job
//!   request ([`crate::data`]) to a deployment;
//! - [`PluginRuntimeClaims`]: plugin → Core, the plugin deployment itself calls a
//!   host write capability with no user or staff call in flight.
use crate::{ProtocolError, Result, resource::HostCapability};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
pub const PLUGIN_CALL_HEADER: &str = "x-flare-plugin-call";
/// Header of the [`PluginRuntimeClaims`] credential on a resource-bridge call.
pub const PLUGIN_RUNTIME_HEADER: &str = "x-flare-plugin-runtime";
pub const MAX_CALL_TTL_MS: i64 = 120_000;
pub const MAX_RUNTIME_CALL_TTL_MS: i64 = 60_000;
/// Permission codes are at most 128 bytes across the signed context, the
/// platform permission catalog and audit (`plugin:` + plugin id + `:` +
/// permission id, see [`plugin_permission`]).
pub const MAX_PERMISSION_CODE_BYTES: usize = 128;
/// Bytes of the operator's stated reason a staff ticket carries.
pub const MAX_STAFF_REASON_BYTES: usize = 512;

const CALL_DOMAIN: &str = "flare.plugin.call.v1";
const STAFF_CALL_DOMAIN: &str = "flare.plugin.staff-call.v1";
const SYSTEM_CALL_DOMAIN: &str = "flare.plugin.system-call.v1";
const RUNTIME_DOMAIN: &str = "flare.plugin.runtime.v1";
const TENANT_STATE_DOMAIN: &str = "flare.plugin.tenant-state.v1";
const DATA_DOMAIN: &str = "flare.plugin.data.v1";

/// The staff permission code of the admin permission `permission` (an
/// [`crate::admin::AdminPermission`] id) of `plugin_id`:
/// `plugin:<pluginId>:<permission>`. `None` when either part is empty or
/// holds `:`, or the code would exceed [`MAX_PERMISSION_CODE_BYTES`].
pub fn plugin_permission(plugin_id: &str, permission: &str) -> Option<String> {
    let code = format!("plugin:{plugin_id}:{permission}");
    (!plugin_id.is_empty()
        && !permission.is_empty()
        && !plugin_id.contains(':')
        && !permission.contains(':')
        && code.len() <= MAX_PERMISSION_CODE_BYTES)
        .then_some(code)
}

/// The plugin id and permission id of a `plugin:<pluginId>:<permission>` code
/// (see [`plugin_permission`]).
pub fn plugin_permission_target(code: &str) -> Option<(&str, &str)> {
    let (plugin_id, permission) = code.strip_prefix("plugin:")?.rsplit_once(':')?;
    (plugin_permission(plugin_id, permission).as_deref() == Some(code))
        .then_some((plugin_id, permission))
}

fn bounded_ids(values: &[&str]) -> bool {
    values
        .iter()
        .all(|v| !v.is_empty() && v.len() <= 192 && !v.chars().any(char::is_control))
}

fn lifetime_ok(issued_at_ms: i64, expires_at_ms: i64, now_ms: i64, max_ttl_ms: i64) -> bool {
    issued_at_ms >= 0
        && issued_at_ms <= now_ms.saturating_add(30_000)
        && expires_at_ms > now_ms
        && expires_at_ms > issued_at_ms
        && expires_at_ms.saturating_sub(issued_at_ms) <= max_ttl_ms
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginCallClaims {
    pub tenant_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub manifest_digest: String,
    pub deployment_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub configuration_digest: String,
    pub actor_id: String,
    pub operation: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}
impl PluginCallClaims {
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        if !bounded_ids(&[
            &self.tenant_id,
            &self.plugin_id,
            &self.plugin_version,
            &self.deployment_id,
            &self.grant_id,
            &self.actor_id,
            &self.operation,
            &self.request_id,
        ]) || self.tenant_id == "*"
            || self.grant_revision == 0
            || !lifetime_ok(
                self.issued_at_ms,
                self.expires_at_ms,
                now_ms,
                MAX_CALL_TTL_MS,
            )
            || !crate::manifest::is_sha256_digest(&self.manifest_digest)
            || !crate::manifest::is_sha256_digest(&self.configuration_digest)
            || self
                .conversation_id
                .as_ref()
                .is_some_and(|id| !crate::resource::valid_resource_id(id))
        {
            return Err(ProtocolError::Denied("invalid plugin invocation ticket"));
        }
        Ok(())
    }
}

/// Ticket for an admin operation invoked by a tenant staff member. Core signs
/// it only after it verified a `Staff` signed context for exactly this tenant
/// holding `permission`, a [`plugin_permission`] code of this plugin (the
/// permission the operation declares in the manifest's `admin` section), and
/// only for an operation in [`crate::PluginManifest::admin_operations`]. The
/// plugin verifies it with [`PluginCallVerifier::verify_staff`], checks that
/// `permission` is the one its manifest declares for `operation` and that an
/// operation declaring `requiresReason` carries `reason`, and still applies
/// its own rules. It carries no conversation: admin work is tenant-scoped and
/// names its entity in the input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginStaffCallClaims {
    pub tenant_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub manifest_digest: String,
    pub deployment_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub configuration_digest: String,
    pub staff_id: String,
    /// The staff member's organization; `None` for platform staff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    /// Explicit management authority, separate from the employee's organization membership.
    #[serde(default, skip_serializing_if = "PluginAdminScope::is_application")]
    pub scope: PluginAdminScope,
    /// `plugin:<pluginId>:<permission>` of this ticket's plugin
    /// ([`plugin_permission`]).
    pub permission: String,
    pub operation: String,
    pub request_id: String,
    /// The reason the staff member stated for this call, as the gateway signed
    /// it into the staff context (`1..=512` bytes, no control characters).
    /// Core requires it for an operation declaring `requiresReason`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PluginAdminScope {
    #[default]
    Application,
    Organization {
        organization_id: String,
    },
}
impl PluginAdminScope {
    pub fn is_application(&self) -> bool {
        matches!(self, Self::Application)
    }
    pub fn organization_id(&self) -> Option<&str> {
        match self {
            Self::Application => None,
            Self::Organization { organization_id } => Some(organization_id),
        }
    }
}
/// Whether `reason` is a stated reason a staff ticket may carry: not blank, at
/// most [`MAX_STAFF_REASON_BYTES`] bytes, no control characters.
pub fn valid_staff_reason(reason: &str) -> bool {
    !reason.trim().is_empty()
        && reason.len() <= MAX_STAFF_REASON_BYTES
        && !reason.chars().any(char::is_control)
}
impl PluginStaffCallClaims {
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        if !bounded_ids(&[
            &self.tenant_id,
            &self.plugin_id,
            &self.plugin_version,
            &self.deployment_id,
            &self.grant_id,
            &self.staff_id,
            &self.operation,
            &self.request_id,
        ]) || self.tenant_id == "*"
            || self.grant_revision == 0
            || self.org_id.as_deref().is_some_and(|id| !bounded_ids(&[id]))
            || self.scope.organization_id().is_some_and(|id| {
                !bounded_ids(&[id]) || self.org_id.as_deref().is_some_and(|member| member != id)
            })
            || self
                .reason
                .as_deref()
                .is_some_and(|reason| !valid_staff_reason(reason))
            || plugin_permission_target(&self.permission)
                .is_none_or(|(plugin_id, _)| plugin_id != self.plugin_id)
            || !lifetime_ok(
                self.issued_at_ms,
                self.expires_at_ms,
                now_ms,
                MAX_CALL_TTL_MS,
            )
            || !crate::manifest::is_sha256_digest(&self.manifest_digest)
            || !crate::manifest::is_sha256_digest(&self.configuration_digest)
        {
            return Err(ProtocolError::Denied("invalid plugin staff ticket"));
        }
        Ok(())
    }
}

/// Ticket for a platform callback Core makes on its own: today the entity
/// lifecycle `unbound` operation of a message-content contribution
/// ([`crate::manifest::UiEntityLifecycle`]), when the card bound to `entity`
/// was recalled or could not be posted. No user or staff stands behind it, so
/// it carries no actor or conversation, and it verifies only with
/// [`PluginCallVerifier::verify_system`] (user and staff tickets never verify
/// there, nor it as them). The plugin still checks that `operation` is the
/// `unbound` operation its manifest declares and handles the call
/// idempotently per entity (Core delivers at least once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginSystemCallClaims {
    pub tenant_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub manifest_digest: String,
    pub deployment_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub configuration_digest: String,
    pub operation: String,
    /// The entity whose card went away ([`crate::state::valid_entity_id`]).
    pub entity: String,
    /// [`crate::manifest::UNBOUND_REASON_RECALLED`] or
    /// [`crate::manifest::UNBOUND_REASON_POST_FAILED`].
    pub reason: String,
    pub request_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}
impl PluginSystemCallClaims {
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        if !bounded_ids(&[
            &self.tenant_id,
            &self.plugin_id,
            &self.plugin_version,
            &self.deployment_id,
            &self.grant_id,
            &self.operation,
            &self.request_id,
        ]) || self.tenant_id == "*"
            || self.grant_revision == 0
            || !crate::state::valid_entity_id(&self.entity)
            || !matches!(
                self.reason.as_str(),
                crate::manifest::UNBOUND_REASON_RECALLED
                    | crate::manifest::UNBOUND_REASON_POST_FAILED
            )
            || !lifetime_ok(
                self.issued_at_ms,
                self.expires_at_ms,
                now_ms,
                MAX_CALL_TTL_MS,
            )
            || !crate::manifest::is_sha256_digest(&self.manifest_digest)
            || !crate::manifest::is_sha256_digest(&self.configuration_digest)
        {
            return Err(ProtocolError::Denied("invalid plugin system ticket"));
        }
        Ok(())
    }
}

/// Core's system ticket for one `PluginRuntime.ApplyTenant`. It names the exact
/// state it covers, so it cannot carry another revision, configuration or
/// enable decision, and it verifies only with
/// [`PluginCallVerifier::verify_tenant_state`]. The host compares every field
/// with the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginTenantStateClaims {
    pub tenant_id: String,
    pub organization_id: String,
    pub deployment_id: String,
    pub manifest_digest: String,
    pub grant_id: String,
    pub revision: u64,
    pub configuration_digest: String,
    pub enabled: bool,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}
impl PluginTenantStateClaims {
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        if !bounded_ids(&[
            &self.tenant_id,
            &self.organization_id,
            &self.deployment_id,
            &self.grant_id,
        ]) || self.tenant_id == "*"
            || self.revision == 0
            || !lifetime_ok(
                self.issued_at_ms,
                self.expires_at_ms,
                now_ms,
                MAX_RUNTIME_CALL_TTL_MS,
            )
            || !crate::manifest::is_sha256_digest(&self.manifest_digest)
            || !crate::manifest::is_sha256_digest(&self.configuration_digest)
        {
            return Err(ProtocolError::Denied("invalid plugin tenant state ticket"));
        }
        Ok(())
    }
}

/// Core's system ticket for one data job request. `request_digest` is
/// [`crate::data::DataRequest::digest`], so it covers no other offset, fence or action; it verifies
/// only with [`PluginCallVerifier::verify_data`]. The host compares the named fields with the
/// request and its own deployment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginDataClaims {
    pub tenant_id: String,
    pub deployment_id: String,
    pub manifest_digest: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub request_digest: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}
impl PluginDataClaims {
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        if !bounded_ids(&[&self.tenant_id, &self.deployment_id, &self.grant_id])
            || self.tenant_id == "*"
            || self.grant_revision == 0
            || !lifetime_ok(
                self.issued_at_ms,
                self.expires_at_ms,
                now_ms,
                MAX_RUNTIME_CALL_TTL_MS,
            )
            || !crate::manifest::is_sha256_digest(&self.manifest_digest)
            || !crate::manifest::is_sha256_digest(&self.request_digest)
        {
            return Err(ProtocolError::Denied("invalid plugin data ticket"));
        }
        Ok(())
    }
}

/// Credential the plugin deployment signs with its own runtime key for a host
/// write when no user or staff call is in flight (a deadline closing a poll).
/// It authorizes one capability for one exact request body: Core recomputes
/// `requestDigest` over the bytes it received. Core maps the key id to the
/// deployment it was provisioned for; the grant must still be enabled at that
/// revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginRuntimeClaims {
    pub tenant_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub manifest_digest: String,
    pub deployment_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    /// A write capability ([`HostCapability::is_write`]).
    pub capability: HostCapability,
    pub request_id: String,
    /// `sha256:<hex>` of the exact resource request bytes.
    pub request_digest: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}
impl PluginRuntimeClaims {
    pub fn validate(&self, now_ms: i64) -> Result<()> {
        if !bounded_ids(&[
            &self.tenant_id,
            &self.plugin_id,
            &self.plugin_version,
            &self.deployment_id,
            &self.grant_id,
            &self.request_id,
        ]) || self.tenant_id == "*"
            || self.grant_revision == 0
            || !self.capability.is_write()
            || !lifetime_ok(
                self.issued_at_ms,
                self.expires_at_ms,
                now_ms,
                MAX_RUNTIME_CALL_TTL_MS,
            )
            || !crate::manifest::is_sha256_digest(&self.manifest_digest)
            || !crate::manifest::is_sha256_digest(&self.request_digest)
        {
            return Err(ProtocolError::Denied("invalid plugin runtime credential"));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Envelope {
    key_id: String,
    payload: String,
    signature: String,
}
fn seal<T: Serialize>(domain: &str, key_id: &str, key: &SigningKey, claims: &T) -> Result<String> {
    let payload =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).map_err(|_| ProtocolError::InvalidJson)?);
    let signature = URL_SAFE_NO_PAD.encode(key.sign(&message(domain, key_id, &payload)).to_bytes());
    Ok(URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Envelope {
            key_id: key_id.to_owned(),
            payload,
            signature,
        })
        .map_err(|_| ProtocolError::InvalidJson)?,
    ))
}
/// Verify the envelope under `domain` with the key `lookup` returns for its key
/// id, and decode the claims. Returns the key id with the claims.
fn open<'k, T: DeserializeOwned>(
    domain: &str,
    token: &str,
    lookup: impl FnOnce(&str) -> Option<&'k VerifyingKey>,
) -> Result<(String, T)> {
    if token.len() > 16384 {
        return Err(ProtocolError::InvalidSignature);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| ProtocolError::InvalidSignature)?;
    let envelope: Envelope =
        serde_json::from_slice(&bytes).map_err(|_| ProtocolError::InvalidSignature)?;
    let key = lookup(&envelope.key_id).ok_or(ProtocolError::InvalidSignature)?;
    let signature = Signature::from_slice(
        &URL_SAFE_NO_PAD
            .decode(&envelope.signature)
            .map_err(|_| ProtocolError::InvalidSignature)?,
    )
    .map_err(|_| ProtocolError::InvalidSignature)?;
    key.verify_strict(
        &message(domain, &envelope.key_id, &envelope.payload),
        &signature,
    )
    .map_err(|_| ProtocolError::InvalidSignature)?;
    let claims = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(&envelope.payload)
            .map_err(|_| ProtocolError::InvalidSignature)?,
    )
    .map_err(|_| ProtocolError::InvalidSignature)?;
    Ok((envelope.key_id, claims))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicKeyEntry {
    key_id: String,
    #[serde(default)]
    deployment_id: Option<String>,
    public_key_base64: String,
}
fn public_keys(raw: &str) -> Result<Vec<(PublicKeyEntry, [u8; 32])>> {
    if raw.len() > 32768 {
        return Err(ProtocolError::InvalidSignature);
    }
    let keys: Vec<PublicKeyEntry> =
        serde_json::from_str(raw).map_err(|_| ProtocolError::InvalidSignature)?;
    if keys.is_empty() || keys.len() > 128 {
        return Err(ProtocolError::InvalidSignature);
    }
    keys.into_iter()
        .map(|key| {
            let public = base64::engine::general_purpose::STANDARD
                .decode(&key.public_key_base64)
                .ok()
                .and_then(|v| v.try_into().ok())
                .ok_or(ProtocolError::InvalidSignature)?;
            Ok((key, public))
        })
        .collect()
}

pub struct PluginCallSigner {
    key_id: String,
    key: SigningKey,
}
impl PluginCallSigner {
    pub fn new(key_id: String, seed: [u8; 32]) -> Result<Self> {
        valid_key_id(&key_id)?;
        Ok(Self {
            key_id,
            key: SigningKey::from_bytes(&seed),
        })
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
    pub fn sign(&self, claims: &PluginCallClaims, now_ms: i64) -> Result<String> {
        claims.validate(now_ms)?;
        seal(CALL_DOMAIN, &self.key_id, &self.key, claims)
    }
    pub fn sign_service(
        &self,
        claims: &crate::service::PluginServiceClaims,
        now_ms: i64,
    ) -> Result<String> {
        claims.validate(now_ms)?;
        seal("flare.plugin.service/v1", &self.key_id, &self.key, claims)
    }
    pub fn sign_event(
        &self,
        claims: &crate::event::PluginEventClaims,
        now_ms: i64,
    ) -> Result<String> {
        claims.validate(now_ms)?;
        seal("flare.plugin.event.v1", &self.key_id, &self.key, claims)
    }
    pub fn sign_staff(&self, claims: &PluginStaffCallClaims, now_ms: i64) -> Result<String> {
        claims.validate(now_ms)?;
        seal(STAFF_CALL_DOMAIN, &self.key_id, &self.key, claims)
    }
    pub fn sign_system(&self, claims: &PluginSystemCallClaims, now_ms: i64) -> Result<String> {
        claims.validate(now_ms)?;
        seal(SYSTEM_CALL_DOMAIN, &self.key_id, &self.key, claims)
    }
    pub fn sign_tenant_state(
        &self,
        claims: &PluginTenantStateClaims,
        now_ms: i64,
    ) -> Result<String> {
        claims.validate(now_ms)?;
        seal(TENANT_STATE_DOMAIN, &self.key_id, &self.key, claims)
    }
    pub fn sign_data(&self, claims: &PluginDataClaims, now_ms: i64) -> Result<String> {
        claims.validate(now_ms)?;
        seal(DATA_DOMAIN, &self.key_id, &self.key, claims)
    }
}
#[derive(Default, Clone)]
pub struct PluginCallVerifier {
    keys: BTreeMap<String, VerifyingKey>,
}
impl PluginCallVerifier {
    /// `[{"key_id": "...", "public_key_base64": "..."}]`, at most 128 keys.
    pub fn from_public_keys_json(raw: &str) -> Result<Self> {
        let mut verifier = Self::default();
        for (key, public) in public_keys(raw)? {
            if key.deployment_id.is_some() {
                return Err(ProtocolError::InvalidSignature);
            }
            verifier.insert(key.key_id, public)?;
        }
        Ok(verifier)
    }
    pub fn insert(&mut self, id: String, key: [u8; 32]) -> Result<()> {
        valid_key_id(&id)?;
        let key = VerifyingKey::from_bytes(&key).map_err(|_| ProtocolError::InvalidSignature)?;
        if (self.keys.len() >= 128 && !self.keys.contains_key(&id))
            || self.keys.get(&id).is_some_and(|old| old != &key)
        {
            return Err(ProtocolError::InvalidSignature);
        }
        self.keys.insert(id, key);
        Ok(())
    }
    /// A user ticket. A staff ticket never verifies here.
    pub fn verify(&self, token: &str, now_ms: i64) -> Result<PluginCallClaims> {
        let (_, claims): (_, PluginCallClaims) = open(CALL_DOMAIN, token, |id| self.keys.get(id))?;
        claims.validate(now_ms)?;
        Ok(claims)
    }
    pub fn verify_service(
        &self,
        token: &str,
        now_ms: i64,
    ) -> Result<crate::service::PluginServiceClaims> {
        let (_, claims): (_, crate::service::PluginServiceClaims) =
            open("flare.plugin.service/v1", token, |id| self.keys.get(id))?;
        claims.validate(now_ms)?;
        Ok(claims)
    }
    pub fn verify_event(
        &self,
        token: &str,
        bytes: &[u8],
        now_ms: i64,
    ) -> Result<crate::event::PluginEventClaims> {
        let (_, claims): (_, crate::event::PluginEventClaims) =
            open("flare.plugin.event.v1", token, |id| self.keys.get(id))?;
        claims.validate(now_ms)?;
        claims.matches_event(bytes)?;
        Ok(claims)
    }
    /// Verify an event origin already bound to a payload, for restricted callbacks and
    /// service delegation. Never use this as an ordinary user invocation.
    pub fn verify_event_origin(
        &self,
        token: &str,
        now_ms: i64,
    ) -> Result<crate::event::PluginEventClaims> {
        let (_, claims): (_, crate::event::PluginEventClaims) =
            open("flare.plugin.event.v1", token, |id| self.keys.get(id))?;
        claims.validate(now_ms)?;
        Ok(claims)
    }
    /// A staff ticket. A user ticket never verifies here. The caller still
    /// checks plugin, deployment, operation (one of the manifest's
    /// `admin_operations`), request id and the tenant grant revision, exactly as
    /// for a user ticket.
    pub fn verify_staff(&self, token: &str, now_ms: i64) -> Result<PluginStaffCallClaims> {
        let (_, claims): (_, PluginStaffCallClaims) =
            open(STAFF_CALL_DOMAIN, token, |id| self.keys.get(id))?;
        claims.validate(now_ms)?;
        Ok(claims)
    }
    /// A platform system ticket. User and staff tickets never verify here,
    /// nor it as them. The caller still checks plugin, deployment, request id,
    /// the tenant grant revision, and that `operation` is the plugin's
    /// declared `entityLifecycle.unbound`.
    pub fn verify_system(&self, token: &str, now_ms: i64) -> Result<PluginSystemCallClaims> {
        let (_, claims): (_, PluginSystemCallClaims) =
            open(SYSTEM_CALL_DOMAIN, token, |id| self.keys.get(id))?;
        claims.validate(now_ms)?;
        Ok(claims)
    }
    /// A tenant state ticket. No other ticket verifies here, nor it as them.
    /// The caller compares it with the delivered state.
    pub fn verify_tenant_state(&self, token: &str, now_ms: i64) -> Result<PluginTenantStateClaims> {
        let (_, claims): (_, PluginTenantStateClaims) =
            open(TENANT_STATE_DOMAIN, token, |id| self.keys.get(id))?;
        claims.validate(now_ms)?;
        Ok(claims)
    }
    /// A data ticket. No other ticket verifies here, nor it as them. The caller compares it with
    /// the request.
    pub fn verify_data(&self, token: &str, now_ms: i64) -> Result<PluginDataClaims> {
        let (_, claims): (_, PluginDataClaims) = open(DATA_DOMAIN, token, |id| self.keys.get(id))?;
        claims.validate(now_ms)?;
        Ok(claims)
    }
    /// Whether this verifier holds the key `key_id` ([`ticket_key_id`]).
    pub fn has_key(&self, key_id: &str) -> bool {
        self.keys.contains_key(key_id)
    }
}

/// The key id a ticket names, read from its envelope without verifying
/// anything; `None` when `token` is not a ticket envelope. With
/// [`PluginCallVerifier::has_key`] a host tells a ticket signed under a key it
/// has not been given yet (Core rotated its signing key first) from one that
/// does not verify.
pub fn ticket_key_id(token: &str) -> Option<String> {
    if token.len() > 16384 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(token).ok()?;
    let envelope: Envelope = serde_json::from_slice(&bytes).ok()?;
    valid_key_id(&envelope.key_id).ok()?;
    Some(envelope.key_id)
}

/// Held by the plugin deployment; the key is provisioned per deployment and is
/// independent of publisher, Cell and call-signing keys.
pub struct PluginRuntimeSigner {
    key_id: String,
    key: SigningKey,
}
impl PluginRuntimeSigner {
    pub fn new(key_id: String, seed: [u8; 32]) -> Result<Self> {
        valid_key_id(&key_id)?;
        Ok(Self {
            key_id,
            key: SigningKey::from_bytes(&seed),
        })
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
    pub fn sign(&self, claims: &PluginRuntimeClaims, now_ms: i64) -> Result<String> {
        claims.validate(now_ms)?;
        seal(RUNTIME_DOMAIN, &self.key_id, &self.key, claims)
    }
}
/// Core side: each key verifies only credentials of the deployment it was
/// provisioned for.
#[derive(Default, Clone)]
pub struct PluginRuntimeVerifier {
    keys: BTreeMap<String, (String, VerifyingKey)>,
}
impl PluginRuntimeVerifier {
    /// `[{"key_id": "...", "deployment_id": "...", "public_key_base64": "..."}]`,
    /// at most 128 keys.
    pub fn from_public_keys_json(raw: &str) -> Result<Self> {
        let mut verifier = Self::default();
        for (key, public) in public_keys(raw)? {
            let deployment = key.deployment_id.ok_or(ProtocolError::InvalidSignature)?;
            verifier.insert(key.key_id, deployment, public)?;
        }
        Ok(verifier)
    }
    pub fn insert(&mut self, id: String, deployment_id: String, key: [u8; 32]) -> Result<()> {
        valid_key_id(&id)?;
        if !bounded_ids(&[&deployment_id]) || deployment_id.len() > 128 {
            return Err(ProtocolError::InvalidSignature);
        }
        let key = VerifyingKey::from_bytes(&key).map_err(|_| ProtocolError::InvalidSignature)?;
        if (self.keys.len() >= 128 && !self.keys.contains_key(&id))
            || self
                .keys
                .get(&id)
                .is_some_and(|old| old != &(deployment_id.clone(), key))
        {
            return Err(ProtocolError::InvalidSignature);
        }
        self.keys.insert(id, (deployment_id, key));
        Ok(())
    }
    /// The credential for exactly `request` (the resource request bytes).
    pub fn verify(&self, token: &str, request: &[u8], now_ms: i64) -> Result<PluginRuntimeClaims> {
        let mut deployment = None;
        let (_, claims): (_, PluginRuntimeClaims) = open(RUNTIME_DOMAIN, token, |id| {
            self.keys.get(id).map(|(owner, key)| {
                deployment = Some(owner.clone());
                key
            })
        })?;
        claims.validate(now_ms)?;
        if deployment.as_deref() != Some(claims.deployment_id.as_str())
            || claims.request_digest != crate::runtime::digest_bytes(request)
        {
            return Err(ProtocolError::InvalidSignature);
        }
        Ok(claims)
    }
}
fn valid_key_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        return Err(ProtocolError::InvalidSignature);
    }
    Ok(())
}
fn message(domain: &str, id: &str, payload: &str) -> Vec<u8> {
    format!("{domain}\n{id}\n{payload}").into_bytes()
}
