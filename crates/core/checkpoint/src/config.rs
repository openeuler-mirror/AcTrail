//! Module configuration: the backends a Trace is composed of, each with its library and its own
//! configuration. The order of the entries is the order they are called in.

use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendEntry {
    /// What launch scopes name as their `backend_instance`.
    pub name: String,
    /// The integration's dynamic library.
    pub library: PathBuf,
    /// The integration's own configuration, passed to it unparsed.
    pub config: PathBuf,
}
