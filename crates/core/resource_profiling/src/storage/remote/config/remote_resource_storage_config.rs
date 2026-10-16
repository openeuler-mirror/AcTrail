use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteResourceProtocol {
    PrometheusRemoteWrite,
    InfluxdbLineProtocol,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RemoteResourceStorageConfig {
    pub protocol: RemoteResourceProtocol,
    pub endpoint: String,
    pub headers: BTreeMap<String, String>,
    pub timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    protocol: RemoteResourceProtocol,
    endpoint: String,
    headers: Option<BTreeMap<String, String>>,
    timeout_ms: Option<u64>,
}
impl RemoteResourceStorageConfig {
    pub fn parse(table: &toml::Table) -> Result<Self, String> {
        let raw: RawConfig = toml::Value::Table(table.clone())
            .try_into()
            .map_err(|e| format!("resource remote: {e}"))?;
        let url =
            reqwest::Url::parse(&raw.endpoint).map_err(|_| "invalid resource remote endpoint")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err("resource remote requires HTTP(S) endpoint".into());
        }
        let timeout_ms = raw.timeout_ms.unwrap_or(1000);
        if timeout_ms == 0 {
            return Err("resource remote timeout must be positive".into());
        }
        Ok(Self {
            protocol: raw.protocol,
            endpoint: raw.endpoint,
            headers: raw.headers.unwrap_or_default(),
            timeout_ms,
        })
    }
}
