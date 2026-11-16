//! Serve the owning runtime and attach bounded history to each pending action.
use crate::{config::Config, history::HistoryReader, remote::DecisionClient};
use serde_json::{Value, json};
use std::io::{Read, Write};

pub(crate) struct GreyProxy {
    history: HistoryReader,
    remote: DecisionClient,
    request_max_bytes: usize,
}

impl GreyProxy {
    pub fn run(config: Config) -> Result<(), String> {
        let proxy = Self {
            history: HistoryReader::open(&config)?,
            remote: DecisionClient::new(&config),
            request_max_bytes: config.http.request_max_bytes as usize,
        };
        let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
        // Readiness follows storage opening, so plugin loading fails on unusable storage.
        println!("http://{}/analyze", server.server_addr());
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        for mut request in server.incoming_requests() {
            let result = proxy.handle(&mut request);
            let response = match result {
                Ok(body) => tiny_http::Response::from_data(body),
                Err(error) => tiny_http::Response::from_string(json!({"error":error}).to_string())
                    .with_status_code(502),
            };
            let _ = request.respond(response);
        }
        Ok(())
    }

    fn handle(&self, request: &mut tiny_http::Request) -> Result<Vec<u8>, String> {
        if request.method() != &tiny_http::Method::Put || request.url() != "/analyze" {
            return Err("expected PUT /analyze".into());
        }
        let mut bytes = Vec::new();
        request
            .as_reader()
            .take(self.request_max_bytes as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > self.request_max_bytes {
            return Err("decision request exceeds host byte limit".into());
        }
        let mut body: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if !body.is_object() {
            return Err("expected decision object".into());
        }
        body["history"] = self.history.read(&body);
        // Process coordinates are used locally for lookup, not repeated in the external payload.
        body.as_object_mut()
            .expect("decision object checked")
            .retain(|key, value| {
                matches!(
                    key.as_str(),
                    "decision_id" | "operation" | "target" | "argv" | "history"
                ) && !value.is_null()
            });
        // Account for each action once, keeping serialization linear in history size.
        let actions = body["history"]["actions"].take();
        body["history"]["actions"] = json!([]);
        let mut size = serde_json::to_vec(&body).map_err(|e| e.to_string())?.len();
        if size > self.request_max_bytes {
            return Err("enhanced decision exceeds host byte limit".into());
        }
        let mut selected = Vec::new();
        for action in actions.as_array().ok_or("history actions missing")? {
            let cost = serde_json::to_vec(action).map_err(|e| e.to_string())?.len()
                + usize::from(!selected.is_empty());
            if size.saturating_add(cost) > self.request_max_bytes {
                body["history"]["truncated"] = true.into();
                break;
            }
            size += cost;
            selected.push(action.clone());
        }
        body["history"]["actions"] = selected.into();
        self.remote
            .decide(&serde_json::to_vec(&body).map_err(|e| e.to_string())?)
    }
}
