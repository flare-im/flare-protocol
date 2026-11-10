//! A read-only observation of an immutable, idempotent plugin intent.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CommandOutcome {
    Committed,
    NotFound,
}
/// The immutable identity and payload of an authenticated pending intent.
/// A read-only observer must verify all three against its original command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandObservationRequest {
    pub command_id: String,
    pub capability_id: String,
    pub payload: serde_json::Value,
}

/// The original command's answer, including a terminal business rejection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedCommandResult {
    pub success: bool,
    pub data: serde_json::Value,
}

/// The query receives the original payload. The plugin verifies its business
/// idempotency key and returns its original operation plus a canonical digest of
/// that exact payload. NotFound is a snapshot, never permission to replace intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandObservation<T = serde_json::Value> {
    pub command_id: String,
    pub capability_id: String,
    pub request_digest: String,
    pub outcome: CommandOutcome,
    pub result: Option<T>,
}
