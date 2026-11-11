//! Send enhanced decision requests using the existing host HTTP limits.
use crate::config::Config;
use std::time::Duration;

pub(crate) struct DecisionClient {
    agent: ureq::Agent,
    endpoint: String,
    response_max_bytes: u64,
}

impl DecisionClient {
    pub fn new(config: &Config) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .proxy(None)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_millis(config.http.timeout_ms)))
                .build()
                .into(),
            endpoint: config.grey_proxy.endpoint.clone(),
            response_max_bytes: config.http.response_max_bytes.into(),
        }
    }

    pub fn decide(&self, body: &[u8]) -> Result<Vec<u8>, String> {
        let mut response = self
            .agent
            .put(&self.endpoint)
            .header("Content-Type", "application/json")
            .send(body)
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("external decision HTTP {}", response.status()));
        }
        response
            .body_mut()
            .with_config()
            .limit(self.response_max_bytes)
            .read_to_vec()
            .map_err(|e| e.to_string())
    }
}
