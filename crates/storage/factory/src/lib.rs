//! Storage backend configuration and factory.

mod builder;
mod config;
mod parser;

pub use builder::open_storage_backend;
pub use config::{StorageBackendKind, StorageConfig};
