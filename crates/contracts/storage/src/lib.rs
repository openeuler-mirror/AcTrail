//! Shared storage configuration, defaults, and payload retention limits.

mod config;
mod constants;
mod parser;
mod payload_retention;
mod sqlite;

pub use config::{StorageBackendKind, StorageConfig};
pub use constants::*;
pub use payload_retention::PayloadRetentionLimits;
pub use sqlite::{EventRecordLayout, SqliteStorageConfig};
