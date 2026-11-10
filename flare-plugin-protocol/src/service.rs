//! Business-neutral service delegation. Core signs both grant scopes and the
//! exact payload; a plugin runtime key cannot mint a Core delegation.
use crate::{
    ProtocolError, Result,
    runtime::{ResolvedPlugin, digest_bytes},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const SERVICE_HEADER: &str = "x-flare-plugin-service";
pub const DISPATCH_OPERATION: &str = "plugin.service.dispatch";
pub const MAX_SERVICE_TTL_MS: i64 = 30_000;
/// Delegation includes an actor ticket and bounded business input, unlike the
/// small read/state resource envelope. It must not enlarge ordinary resources.
pub const MAX_SERVICE_REQUEST_BYTES: usize = 64 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ServiceMode {
    User,
    Recovery,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceOperation {
    pub operation: String,
    pub mode: ServiceMode,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginServiceRequest {
    pub target_plugin_id: String,
    pub operation: String,
    pub business_ref: String,
    pub mode: ServiceMode,
    /// The dedicated Core ticket of the original call, never its bearer token.
    pub actor_ticket: Option<String>,
    pub input: Value,
}
impl PluginServiceRequest {
    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.target_plugin_id)
            || !valid_id(&self.operation)
            || !valid_id(&self.business_ref)
            || !self.input.is_object()
            || self
                .actor_ticket
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 16384)
            || (self.mode == ServiceMode::User) != self.actor_ticket.is_some()
        {
            return Err(ProtocolError::Denied("invalid service request"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServicePrincipal {
    pub plugin_id: String,
    pub version: String,
    pub manifest_digest: String,
    pub deployment_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub configuration_digest: String,
}
impl ServicePrincipal {
    pub fn projected(p: &ResolvedPlugin) -> Self {
        Self {
            plugin_id: p.grant.plugin_id.clone(),
            version: p.grant.plugin_version.clone(),
            manifest_digest: p.grant.manifest_digest.clone(),
            deployment_id: p.deployment.deployment_id.clone(),
            grant_id: p.grant.grant_id.clone(),
            grant_revision: p.grant.revision,
            configuration_digest: p.configuration_digest.clone(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if ![
            &self.plugin_id,
            &self.version,
            &self.deployment_id,
            &self.grant_id,
        ]
        .into_iter()
        .all(|s| valid_id(s))
            || self.grant_revision == 0
            || self.grant_revision > i64::MAX as u64
            || ![&self.manifest_digest, &self.configuration_digest]
                .into_iter()
                .all(|s| crate::manifest::is_sha256_digest(s))
        {
            return Err(ProtocolError::Denied("invalid service principal"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginServiceClaims {
    pub tenant_id: String,
    pub source: ServicePrincipal,
    pub target: ServicePrincipal,
    pub actor_id: Option<String>,
    /// Bound by the original user or Core event ticket; absent for recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub origin_operation: Option<String>,
    pub mode: ServiceMode,
    pub operation: String,
    pub business_ref: String,
    pub request_id: String,
    pub input_digest: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
}
impl PluginServiceClaims {
    pub fn validate(&self, now: i64) -> Result<()> {
        self.source.validate()?;
        self.target.validate()?;
        if self.tenant_id.is_empty()
            || self.tenant_id.len() > 32
            || !self
                .tenant_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || self.source.plugin_id == self.target.plugin_id
            || ![&self.operation, &self.business_ref, &self.request_id]
                .into_iter()
                .all(|s| valid_id(s))
            || !crate::manifest::is_sha256_digest(&self.input_digest)
            || self.issued_at_ms < 0
            || self.issued_at_ms > now + 5000
            || self.expires_at_ms <= now
            || self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms - self.issued_at_ms > MAX_SERVICE_TTL_MS
            || match self.mode {
                ServiceMode::User => {
                    self.actor_id.as_ref().is_none_or(|s| !valid_id(s))
                        || self.origin_operation.as_ref().is_none_or(|s| !valid_id(s))
                        || self.conversation_id.as_ref().is_some_and(|s| !valid_id(s))
                }
                ServiceMode::Recovery => {
                    self.actor_id.is_some()
                        || self.origin_operation.is_some()
                        || self.conversation_id.is_some()
                }
            }
        {
            return Err(ProtocolError::Denied("invalid service delegation"));
        }
        Ok(())
    }
    pub fn matches_input(&self, input: &Value) -> Result<()> {
        if self.input_digest == input_digest(input)? {
            Ok(())
        } else {
            Err(ProtocolError::Denied("service input changed"))
        }
    }
}
/// Canonical object digest, independent of serde_json's preserve_order feature.
pub fn input_digest(input: &Value) -> Result<String> {
    fn ordered(v: &Value, depth: usize) -> Result<String> {
        if depth > 32 {
            return Err(ProtocolError::Denied("service nesting"));
        }
        Ok(match v {
            Value::Object(m) => {
                let mut keys = m.keys().collect::<Vec<_>>();
                keys.sort();
                let pairs = keys
                    .into_iter()
                    .map(|k| {
                        Ok(format!(
                            "{}:{}",
                            serde_json::to_string(k).map_err(|_| ProtocolError::InvalidJson)?,
                            ordered(&m[k], depth + 1)?
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                format!("{{{}}}", pairs.join(","))
            }
            Value::Array(a) => format!(
                "[{}]",
                a.iter()
                    .map(|v| ordered(v, depth + 1))
                    .collect::<Result<Vec<_>>>()?
                    .join(",")
            ),
            _ => v.to_string(),
        })
    }
    Ok(digest_bytes(ordered(input, 0)?.as_bytes()))
}
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginServiceResponse {
    pub ok: bool,
    pub error_code: Option<String>,
    pub data: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::{
        PluginCallSigner, PluginCallVerifier, PluginRuntimeClaims, PluginRuntimeSigner,
    };
    use crate::resource::HostCapability;
    use serde_json::json;
    fn claims() -> PluginServiceClaims {
        let principal = |id: &str| ServicePrincipal {
            plugin_id: id.into(),
            version: "0.1.0".into(),
            manifest_digest: format!("sha256:{}", "a".repeat(64)),
            deployment_id: format!("{id}-deploy"),
            grant_id: format!("{id}-grant"),
            grant_revision: 1,
            configuration_digest: format!("sha256:{}", "b".repeat(64)),
        };
        PluginServiceClaims {
            tenant_id: "app".into(),
            source: principal("sender"),
            target: principal("wallet"),
            actor_id: Some("alice".into()),
            conversation_id: Some("group-1".into()),
            origin_operation: Some("sender.commit".into()),
            mode: ServiceMode::User,
            operation: "wallet.open".into(),
            business_ref: "packet".into(),
            request_id: "request".into(),
            input_digest: input_digest(&json!({"amount":"12"})).unwrap(),
            issued_at_ms: 1000,
            expires_at_ms: 31000,
        }
    }
    #[test]
    fn canonical_digest_binds_economic_input_and_deep_payload_is_bounded() {
        assert_eq!(
            input_digest(&json!({"b":2,"a":{"d":4,"c":3}})).unwrap(),
            input_digest(&serde_json::from_str("{\"a\":{\"c\":3,\"d\":4},\"b\":2}").unwrap())
                .unwrap()
        );
        let c = claims();
        assert!(c.matches_input(&json!({"amount":"12"})).is_ok());
        assert!(c.matches_input(&json!({"amount":"13"})).is_err());
        let mut deep = json!({});
        for _ in 0..34 {
            deep = json!({"nested":deep});
        }
        assert!(input_digest(&deep).is_err());
    }
    #[test]
    fn delegation_enforces_actor_mode_audience_and_short_expiry() {
        let c = claims();
        assert!(c.validate(2000).is_ok());
        assert!(c.validate(31000).is_err());
        let mut changed = c.clone();
        changed.expires_at_ms += 1;
        assert!(changed.validate(2000).is_err());
        changed = c.clone();
        changed.mode = ServiceMode::Recovery;
        assert!(changed.validate(2000).is_err());
        changed.actor_id = None;
        changed.origin_operation = None;
        changed.conversation_id = None;
        assert!(changed.validate(2000).is_ok());
        changed.target.plugin_id = changed.source.plugin_id.clone();
        assert!(changed.validate(2000).is_err());
    }
    #[test]
    fn only_core_service_signature_domain_is_accepted() {
        let signer = PluginCallSigner::new("core".into(), [1; 32]).unwrap();
        let mut verifier = PluginCallVerifier::default();
        verifier.insert("core".into(), signer.public_key()).unwrap();
        let c = claims();
        let ticket = signer.sign_service(&c, 2000).unwrap();
        assert_eq!(verifier.verify_service(&ticket, 2000).unwrap(), c);
        assert!(verifier.verify(&ticket, 2000).is_err());
        // Even deliberately using the same test key cannot cross the workload
        // signature domain. Production deployments use different provisioned keys.
        let workload = PluginRuntimeSigner::new("core".into(), [1; 32]).unwrap();
        let r = PluginRuntimeClaims {
            tenant_id: "app".into(),
            plugin_id: "sender".into(),
            plugin_version: "0.1.0".into(),
            manifest_digest: c.source.manifest_digest.clone(),
            deployment_id: c.source.deployment_id.clone(),
            grant_id: c.source.grant_id.clone(),
            grant_revision: 1,
            capability: HostCapability::ServiceDispatch,
            request_id: "request".into(),
            request_digest: c.input_digest.clone(),
            issued_at_ms: 1000,
            expires_at_ms: 31000,
        };
        let wrong = workload.sign(&r, 2000).unwrap();
        assert!(verifier.verify_service(&wrong, 2000).is_err());
    }
}
