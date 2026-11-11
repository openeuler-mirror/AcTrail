//! Parse the configuration supplied by the owning WASM runtime.
use plugin_system::PluginHttpDeclaration;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HistoryConfig {
    pub endpoint: String,
    pub window_ms: u64,
    pub max_actions: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    pub grey_proxy: HistoryConfig,
    pub storage_path: std::path::PathBuf,
    pub http: PluginHttpDeclaration,
}

impl Config {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let config: Self = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        if config.grey_proxy.window_ms == 0 || config.grey_proxy.max_actions == 0 {
            return Err("grey_proxy window_ms and max_actions must be positive".into());
        }
        let uri = config
            .grey_proxy
            .endpoint
            .parse::<ureq::http::Uri>()
            .map_err(|e| e.to_string())?;
        if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.host().is_none() {
            return Err("grey_proxy endpoint must be an absolute HTTP(S) URL".into());
        }
        if !config
            .http
            .allowed_urls
            .contains(&config.grey_proxy.endpoint)
        {
            return Err("grey_proxy endpoint must appear in host.http.allowed_urls".into());
        }
        if config.http.timeout_ms == 0
            || config.http.request_max_bytes == 0
            || config.http.response_max_bytes == 0
        {
            return Err("host HTTP limits must be positive".into());
        }
        Ok(config)
    }
}
