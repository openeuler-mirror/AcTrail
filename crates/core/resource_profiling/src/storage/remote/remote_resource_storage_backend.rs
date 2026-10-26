use super::{
    RemoteResourceProtocol, RemoteResourceStorageConfig,
    protocol::{InfluxdbLineProtocolClient, PrometheusRemoteWriteClient},
};
use crate::{
    model::ResourceSample,
    storage::{ResourceStorageBackend, ResourceStorageError},
};
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use std::time::Duration;
pub struct RemoteResourceStorageBackend {
    config: RemoteResourceStorageConfig,
    client: Client,
}
impl RemoteResourceStorageBackend {
    pub(super) fn new(config: RemoteResourceStorageConfig) -> Result<Self, ResourceStorageError> {
        let mut headers = HeaderMap::new();
        for (key, value) in &config.headers {
            let key = HeaderName::from_bytes(key.as_bytes())
                .map_err(|_| ResourceStorageError("invalid resource remote header name".into()))?;
            let value = HeaderValue::from_str(value)
                .map_err(|_| ResourceStorageError("invalid resource remote header value".into()))?;
            headers.insert(key, value);
        }
        let client = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|_| {
                ResourceStorageError("resource HTTP client initialization failed".into())
            })?;
        Ok(Self { config, client })
    }
}
impl ResourceStorageBackend for RemoteResourceStorageBackend {
    fn write(
        &mut self,
        samples: &[ResourceSample],
        timeout: Duration,
    ) -> Result<(), ResourceStorageError> {
        let request = self
            .client
            .post(&self.config.endpoint)
            .timeout(timeout.min(Duration::from_millis(self.config.timeout_ms)));
        let request = match self.config.protocol {
            RemoteResourceProtocol::PrometheusRemoteWrite => request
                .header("Content-Type", "application/x-protobuf")
                .header("Content-Encoding", "snappy")
                .header("X-Prometheus-Remote-Write-Version", "0.1.0")
                .body(PrometheusRemoteWriteClient::encode(samples)?),
            RemoteResourceProtocol::InfluxdbLineProtocol => request
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(InfluxdbLineProtocolClient::encode(samples)?),
        };
        let response = request.send().map_err(|e| {
            ResourceStorageError(
                if e.is_timeout() {
                    "resource remote timeout"
                } else {
                    "resource remote request failed"
                }
                .into(),
            )
        })?;
        if !response.status().is_success() {
            return Err(ResourceStorageError(format!(
                "resource remote HTTP {}",
                response.status().as_u16()
            )));
        }
        Ok(())
    }
}
