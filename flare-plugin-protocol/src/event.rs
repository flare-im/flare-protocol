//! Core's asynchronous observation contract. An event authenticates what happened;
//! it is never an ordinary user invocation or a user bearer credential.
use crate::{ProtocolError, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MESSAGE_CREATED: &str = "im.message.created";
pub const MESSAGE_EVENTS_SCOPE: &str = "messages.events.read";
pub const MESSAGE_BODY_SCOPE: &str = "messages.body.read";
pub const MAX_EVENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginEventSubscription {
    pub event_type: String,
    pub schema_version: u32,
    pub operation: String,
    pub permission_ids: Vec<String>,
    #[serde(default)]
    pub include_body: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginMessageEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub schema_version: u32,
    pub app: String,
    /// The ingest-assigned message time, in epoch milliseconds.
    pub time: i64,
    pub data: MessageCreatedData,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageCreatedData {
    pub server_message_id: String,
    pub conversation_id: String,
    pub actor_id: String,
    pub conversation_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}
impl PluginMessageEvent {
    pub fn validate(&self) -> Result<()> {
        if self.event_type != MESSAGE_CREATED
            || self.schema_version != 1
            || self.time <= 0
            || ![
                &self.id,
                &self.app,
                &self.data.server_message_id,
                &self.data.conversation_id,
                &self.data.actor_id,
            ]
            .into_iter()
            .all(|id| crate::resource::valid_resource_id(id))
            || !matches!(self.data.conversation_type.as_str(), "group" | "single")
            || self.app == "*"
            || self.data.text.as_ref().is_some_and(|s| s.len() > 48 * 1024)
        {
            return Err(ProtocolError::Denied("invalid message event"));
        }
        Ok(())
    }
}
/// Core signs the exact event bytes and audience, grant, configuration, actor and conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginEventClaims {
    pub tenant_id: String,
    pub plugin_id: String,
    pub plugin_version: String,
    pub manifest_digest: String,
    pub deployment_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub configuration_digest: String,
    /// The current application's published features, signed by Core. The host
    /// derives operation gates from its verified manifest, never from event JSON.
    pub enabled_features: BTreeSet<String>,
    /// Stable activation of the approved source grant/configuration, independent of leases.
    pub active_since_ms: i64,
    pub operation: String,
    pub event_id: String,
    pub request_digest: String,
    pub actor_id: String,
    pub conversation_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}
impl PluginEventClaims {
    pub fn validate(&self, now: i64) -> Result<()> {
        if ![
            &self.tenant_id,
            &self.plugin_id,
            &self.plugin_version,
            &self.deployment_id,
            &self.grant_id,
            &self.operation,
            &self.event_id,
            &self.actor_id,
            &self.conversation_id,
        ]
        .into_iter()
        .all(|id| !id.is_empty() && id.len() <= 192 && !id.chars().any(char::is_control))
            || self.tenant_id == "*"
            || self.grant_revision == 0
            || self.active_since_ms < 0
            || self.active_since_ms > now.saturating_add(30_000)
            || self.enabled_features.len() > crate::manifest::MAX_PLUGIN_FEATURES
            || self
                .enabled_features
                .iter()
                .any(|id| id.is_empty() || id.len() > 64 || id.chars().any(char::is_control))
            || self.issued_at_ms < 0
            || self.issued_at_ms > now.saturating_add(30_000)
            || self.expires_at_ms <= now
            || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms.saturating_sub(self.issued_at_ms)
                > crate::invocation::MAX_CALL_TTL_MS
            || [
                &self.manifest_digest,
                &self.configuration_digest,
                &self.request_digest,
            ]
            .into_iter()
            .any(|d| !crate::manifest::is_sha256_digest(d))
        {
            return Err(ProtocolError::Denied("invalid event ticket"));
        }
        Ok(())
    }
    pub fn matches_event(&self, bytes: &[u8]) -> Result<PluginMessageEvent> {
        if bytes.len() > MAX_EVENT_BYTES
            || crate::runtime::digest_bytes(bytes) != self.request_digest
        {
            return Err(ProtocolError::Denied("event payload mismatch"));
        }
        let event: PluginMessageEvent =
            serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidJson)?;
        event.validate()?;
        if event.app != self.tenant_id
            || event.id != self.event_id
            || event.data.actor_id != self.actor_id
            || event.data.conversation_id != self.conversation_id
            || event.time < self.active_since_ms
        {
            return Err(ProtocolError::Denied("event identity mismatch"));
        }
        Ok(event)
    }
}

impl crate::runtime::ResolvedPlugin {
    /// The current authenticated projection must approve the callback and every privacy scope.
    pub fn authorize_event(
        &self,
        event_type: &str,
        operation: &str,
        actor: &str,
        now: i64,
    ) -> Result<&PluginEventSubscription> {
        let subscription = self
            .manifest()
            .event_subscriptions
            .iter()
            .find(|s| {
                s.event_type == event_type && s.schema_version == 1 && s.operation == operation
            })
            .ok_or(ProtocolError::Denied("event subscription missing"))?;
        if self.closed().operations.contains(operation) {
            return Err(ProtocolError::Denied("event feature closed"));
        }
        self.authorize(&crate::authorization::PluginInvocation {
            tenant_id: &self.grant.tenant_id,
            actor_id: actor,
            operation,
            actor_permissions: &self.grant.approved_permissions,
            platform_enabled: true,
            now_ms: now,
        })?;
        if !subscription
            .permission_ids
            .iter()
            .all(|id| self.grant.approved_permissions.contains(id))
            || !self
                .grant
                .approved_data_scopes
                .contains(MESSAGE_EVENTS_SCOPE)
            || subscription.include_body
                && !self.grant.approved_data_scopes.contains(MESSAGE_BODY_SCOPE)
        {
            return Err(ProtocolError::Denied(
                "event subscription scope not approved",
            ));
        }
        Ok(subscription)
    }
}
