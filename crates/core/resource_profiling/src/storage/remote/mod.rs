mod config;
mod protocol;
mod remote_resource_storage_backend;
mod remote_resource_storage_factory;
pub use config::{RemoteResourceProtocol, RemoteResourceStorageConfig};
pub use remote_resource_storage_backend::RemoteResourceStorageBackend;
pub use remote_resource_storage_factory::RemoteResourceStorageFactory;
