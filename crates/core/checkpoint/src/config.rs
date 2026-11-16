//! Integrations loaded by the daemon. Each launch selects its own instances and configuration.

use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendEntry {
    /// What launch scopes name as their `backend_instance`.
    pub name: String,
    /// The integration's dynamic library.
    pub library: PathBuf,
    /// Daemon-owned runtime configuration, passed as opaque JSON to the integration.
    pub config: PathBuf,
}
