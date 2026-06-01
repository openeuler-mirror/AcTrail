//! Plugin manifest contracts for policy extensions.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub api_version: String,
    pub requires_admin_runtime: bool,
}
