//! Optional LLM wire-codec plugins.

use std::cell::Cell;
use std::sync::Arc;

use plugin_system::{
    LlmCodecDecoded, LlmCodecOutcome, LlmCodecPlugin, LlmCodecPluginStatus, LlmCodecRequest,
    LlmCodecSseEvent,
};
use serde_json::Value;

use crate::llm_pipeline::transport::HttpRequestParts;

#[derive(Clone, Default)]
pub struct LlmCodecRegistry {
    plugins: Vec<Arc<dyn LlmCodecPlugin>>,
    revision: u64,
    failed_plugin_decodes: Cell<u64>,
}

impl LlmCodecRegistry {
    pub(in crate::llm_pipeline) fn revision(&self) -> u64 {
        self.revision
    }

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
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub fn unregister(&mut self, instance_id: &str) -> bool {
        let before = self.plugins.len();
        self.plugins
            .retain(|plugin| plugin.instance_id() != instance_id);
        let removed = self.plugins.len() != before;
        if removed {
            self.revision = self.revision.wrapping_add(1);
        }
        removed
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

    pub(in crate::llm_pipeline) fn decode_request(
        &self,
        http: &HttpRequestParts,
    ) -> Option<LlmCodecDecoded> {
        let request = LlmCodecRequest {
            method: http.method.as_deref(),
            authority: http.authority.as_deref(),
            path: http.path.as_deref(),
            body: &http.body,
        };
        for plugin in &self.plugins {
            match plugin.decode_request(request.clone()) {
                Ok(LlmCodecOutcome::Decoded(decoded)) => return Some(decoded),
                Ok(LlmCodecOutcome::NoMatch) => {}
                Err(_) => self.record_failed_plugin_decode(),
            }
        }
        None
    }

    pub(in crate::llm_pipeline) fn decode_sse_event(
        &self,
        event: &SseCodecEvent,
    ) -> Option<LlmCodecDecoded> {
        let input = LlmCodecSseEvent {
            index: event.index,
            event_type: event.event_type.as_deref(),
            id: event.id.as_deref(),
            data: &event.data,
        };
        for plugin in &self.plugins {
            match plugin.decode_sse_event(input.clone()) {
                Ok(LlmCodecOutcome::Decoded(decoded)) => return Some(decoded),
                Ok(LlmCodecOutcome::NoMatch) => {}
                Err(_) => self.record_failed_plugin_decode(),
            }
        }
        None
    }

    pub(in crate::llm_pipeline) fn take_failed_plugin_decodes(&self) -> u64 {
        self.failed_plugin_decodes.replace(0)
    }

    fn record_failed_plugin_decode(&self) {
        self.failed_plugin_decodes
            .set(self.failed_plugin_decodes.get().saturating_add(1));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct SseCodecEvent {
    pub(in crate::llm_pipeline) index: usize,
    pub(in crate::llm_pipeline) event_type: Option<String>,
    pub(in crate::llm_pipeline) id: Option<String>,
    pub(in crate::llm_pipeline) data: String,
    pub(in crate::llm_pipeline) json: Option<Value>,
    pub(in crate::llm_pipeline) done_marker: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::llm_pipeline) struct NormalizedSseEvent {
    pub(in crate::llm_pipeline) index: usize,
    pub(in crate::llm_pipeline) event_type: Option<String>,
    pub(in crate::llm_pipeline) id: Option<String>,
    pub(in crate::llm_pipeline) data: String,
    pub(in crate::llm_pipeline) json: Option<Value>,
    pub(in crate::llm_pipeline) done_marker: bool,
}
