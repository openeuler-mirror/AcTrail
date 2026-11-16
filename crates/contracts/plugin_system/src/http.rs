//! Shared HTTP endpoint permissions and transport limits.

use serde::{Deserialize, Serialize};

/// Permitted HTTP URLs and transport resource limits.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginHttpDeclaration {
    pub allowed_urls: Vec<String>,
    pub timeout_ms: u64,
    pub finish_reserve_ms: Option<u64>,
    pub request_max_bytes: u32,
    pub response_max_bytes: u32,
}
