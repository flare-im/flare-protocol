//! Operator data jobs share the existing plugin endpoint, but never user tickets. Core signs each
//! call: the ticket's request digest binds every field, including offset and fence.
use crate::{ProtocolError, Result, runtime::digest_bytes};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const OPERATION: &str = "flare.plugin.data.v1";
pub const MAX_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_EXPORT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 96 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataRequest {
    pub app_id: String,
    pub deployment_id: String,
    pub manifest_digest: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub job_id: String,
    pub fence: u64,
    pub action: DataAction,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum DataAction {
    Export,
    /// Requires durable tenant admission to be closed and drained.
    Snapshot,
    Quiesce,
    Resume,
    /// Irreversibly fences the source after a verified target commit.
    Retire,
    Read {
        offset: u64,
        digest: String,
    },
    Import {
        artifact: DataArtifact,
    },
    Write {
        offset: u64,
        digest: String,
        content_base64: String,
    },
    Commit {
        digest: String,
    },
    Discard {
        digest: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataArtifact {
    pub job_id: String,
    pub app_id: String,
    pub digest: String,
    pub bytes: u64,
    pub expires_at_ms: i64,
    pub schema_version: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataChunk {
    pub artifact: DataArtifact,
    pub offset: u64,
    pub content_base64: String,
}
impl DataRequest {
    pub fn validate(&self) -> Result<()> {
        fn key(s: &str, max: usize) -> bool {
            !s.is_empty()
                && s.len() <= max
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        }
        if !key(&self.app_id, 32)
            || !key(&self.deployment_id, 128)
            || !key(&self.grant_id, 128)
            || !key(&self.job_id, 128)
            || self.grant_revision == 0
            || self.grant_revision > i64::MAX as u64
            || self.fence == 0
            || self.fence > i64::MAX as u64
            || !crate::manifest::is_sha256_digest(&self.manifest_digest)
        {
            return Err(ProtocolError::Denied("invalid data job"));
        }
        match &self.action {
            DataAction::Read { offset, digest } | DataAction::Write { offset, digest, .. }
                if *offset > MAX_EXPORT_BYTES as u64
                    || !crate::manifest::is_sha256_digest(digest) =>
            {
                return Err(ProtocolError::Denied("invalid data chunk"));
            }
            DataAction::Import { artifact }
                if artifact.app_id != self.app_id
                    || artifact.job_id != self.job_id
                    || artifact.bytes == 0
                    || artifact.bytes > MAX_EXPORT_BYTES as u64
                    || artifact.schema_version == 0
                    || !crate::manifest::is_sha256_digest(&artifact.digest) =>
            {
                return Err(ProtocolError::Denied("invalid import artifact"));
            }
            DataAction::Commit { digest } | DataAction::Discard { digest }
                if !crate::manifest::is_sha256_digest(digest) =>
            {
                return Err(ProtocolError::Denied("invalid import digest"));
            }
            _ => (),
        }
        if let DataAction::Write { content_base64, .. } = &self.action {
            use base64::Engine;
            if content_base64.len() > MAX_CHUNK_BYTES.div_ceil(3) * 4
                || base64::engine::general_purpose::STANDARD
                    .decode(content_base64)
                    .map_or(true, |v| v.is_empty() || v.len() > MAX_CHUNK_BYTES)
            {
                return Err(ProtocolError::Denied("invalid import chunk"));
            }
        }
        Ok(())
    }
    /// What Core's data ticket names ([`crate::invocation::PluginDataClaims`]).
    pub fn digest(&self) -> Result<String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| ProtocolError::InvalidJson)?;
        Ok(digest_bytes(&bytes))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn purpose_binds_every_request_and_fence() {
        let mut a = DataRequest {
            app_id: "201".into(),
            deployment_id: "dep".into(),
            manifest_digest: digest_bytes(b"manifest"),
            grant_id: "grant".into(),
            grant_revision: 1,
            job_id: "job".into(),
            fence: 1,
            action: DataAction::Export,
        };
        let digest = a.digest().unwrap();
        a.fence = 2;
        assert_ne!(a.digest().unwrap(), digest);
        a.fence = 1;
        a.action = DataAction::Read {
            offset: 0,
            digest: digest_bytes(b"body"),
        };
        assert_ne!(a.digest().unwrap(), digest);
        a.app_id = "../201".into();
        assert!(a.digest().is_err());
    }
}
