//! Optional LLM wire-codec plugins.

use std::sync::Arc;

use plugin_system::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRequest,
    LlmCodecSseEvent,
};
use serde_json::Value;

use crate::payload_projection::http::HttpRequestParts;

#[derive(Clone, Default)]
pub struct LlmCodecRegistry {
    plugins: Vec<Arc<dyn LlmCodecPlugin>>,
}

impl LlmCodecRegistry {
    pub fn register(&mut self, plugin: Arc<dyn LlmCodecPlugin>) -> Result<(), String> {
        let instance_id = plugin.instance_id();
        if self
            .plugins
            .iter()
            .any(|existing| existing.instance_id() == instance_id)
        {
            return Err(format!(
                "LLM codec plugin instance {instance_id} already exists"
            ));
        }
        self.plugins.push(plugin);
        Ok(())
    }

    pub fn unregister(&mut self, instance_id: &str) -> bool {
        let before = self.plugins.len();
        self.plugins
            .retain(|plugin| plugin.instance_id() != instance_id);
        self.plugins.len() != before
    }

    pub fn statuses(&self) -> Vec<LlmCodecPluginStatus> {
        self.plugins
            .iter()
            .map(|plugin| LlmCodecPluginStatus {
                instance_id: plugin.instance_id().to_string(),
                plugin_id: plugin.plugin_id().to_string(),
            })
            .collect()
    }

    pub(super) fn decode_request(&self, http: &HttpRequestParts) -> Option<LlmCodecDecoded> {
        let request = LlmCodecRequest {
            method: http.method.as_deref(),
            authority: http.authority.as_deref(),
            path: http.path.as_deref(),
            body: &http.body,
        };
        for plugin in &self.plugins {
            match plugin.decode_request(request.clone()) {
                Ok(LlmCodecOutcome::Decoded(decoded)) => return Some(decoded),
                Ok(LlmCodecOutcome::NoMatch) | Err(_) => {}
            }
        }
        None
    }

    pub(super) fn decode_sse_event(&self, event: &SseCodecEvent) -> Option<LlmCodecDecoded> {
        let input = LlmCodecSseEvent {
            index: event.index,
            event_type: event.event_type.as_deref(),
            id: event.id.as_deref(),
            data: &event.data,
        };
        for plugin in &self.plugins {
            match plugin.decode_sse_event(input.clone()) {
                Ok(LlmCodecOutcome::Decoded(decoded)) => return Some(decoded),
                Ok(LlmCodecOutcome::NoMatch) | Err(_) => {}
            }
        }
        None
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SseCodecEvent {
    pub(super) index: usize,
    pub(super) event_type: Option<String>,
    pub(super) id: Option<String>,
    pub(super) data: String,
    pub(super) json: Option<Value>,
    pub(super) done_marker: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NormalizedSseEvent {
    pub(super) index: usize,
    pub(super) event_type: Option<String>,
    pub(super) id: Option<String>,
    pub(super) data: String,
    pub(super) json: Option<Value>,
    pub(super) done_marker: bool,
}
