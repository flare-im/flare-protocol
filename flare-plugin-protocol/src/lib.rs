//! Shared plugin contracts. No IM/Social business logic, transport, persistence,
//! process orchestration or commercial authorization lives in this crate.

pub mod admin;
pub mod authorization;
pub mod command;
pub mod configuration;
pub mod data;
pub mod event;
pub mod invocation;
pub mod lifecycle;
pub mod manifest;
pub mod package;
pub mod registry;
pub mod resource;
pub mod runtime;
pub mod service;
pub mod signature;
pub mod state;
mod tagged;
pub mod ui;

pub use manifest::PluginManifest;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("invalid manifest field {field}: {reason}")]
    InvalidManifest { field: String, reason: String },
    #[error("manifest exceeds {0} bytes")]
    ManifestTooLarge(usize),
    #[error("invalid manifest JSON")]
    InvalidJson,
    #[error("manifest signature is invalid or publisher key is not trusted")]
    InvalidSignature,
    #[error("invalid plugin package: {0}")]
    InvalidPackage(&'static str),
    #[error("plugin version is immutable: {0}")]
    VersionConflict(String),
    #[error("no compatible dependency resolution for {0}")]
    UnresolvedDependency(String),
    #[error("plugin dependency cycle: {0}")]
    DependencyCycle(String),
    #[error("capability namespace is already owned: {0}")]
    NamespaceConflict(String),
    #[error("plugin identity belongs to another publisher: {0}")]
    PublisherConflict(String),
    #[error("plugin invocation denied: {0}")]
    Denied(&'static str),
    #[error("invalid lifecycle transition")]
    InvalidTransition,
    #[error("stale lifecycle revision")]
    StaleRevision,
}

pub type Result<T> = std::result::Result<T, ProtocolError>;
