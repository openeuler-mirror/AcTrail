//! Storage backend configuration and factory.

mod builder;
mod config;
mod parser;

pub use builder::open_storage_backend;
pub use config::{StorageBackendKind, StorageConfig};
pub use sqlite_storage::{
    EventRecordLayout, SQLITE_MAX_EVENT_RECORD_BLOCK_EVENTS,
    SQLITE_MAX_EVENT_RECORD_BLOCK_UNCOMPRESSED_BYTES,
};
