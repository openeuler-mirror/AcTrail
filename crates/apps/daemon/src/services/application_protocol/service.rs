//! Application protocol analyzers over retained plaintext payload segments.

use config_core::daemon::{ApplicationProtocolConfig, SemanticRetentionConfig};
use model_core::event::ApplicationPayload;
use model_core::ids::TraceId;
use model_core::payload::PayloadStreamIdentity;

#[path = "http1.rs"]
mod http1;
#[path = "http2.rs"]
mod http2;

pub(super) const COLLECTOR_NAME: &str = "application-protocol-analyzer";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ApplicationEventDraft {
    pub payload: ApplicationPayload,
    pub metadata_partial: bool,
    pub observed_at: Option<std::time::SystemTime>,
    pub process: Option<model_core::process::ProcessIdentity>,
}

impl ApplicationEventDraft {
    pub(super) fn complete(payload: ApplicationPayload) -> Self {
        Self {
            payload,
            metadata_partial: false,
            observed_at: None,
            process: None,
        }
    }

    pub(super) fn partial(payload: ApplicationPayload) -> Self {
        Self {
            payload,
            metadata_partial: true,
            observed_at: None,
            process: None,
        }
    }

    pub(super) fn at(mut self, observed_at: std::time::SystemTime) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    pub(super) fn for_process(mut self, process: model_core::process::ProcessIdentity) -> Self {
        self.process = Some(process);
        self
    }
}

pub(super) struct ApplicationProtocolAnalyzer {
    config: ApplicationProtocolConfig,
    semantic_retention: SemanticRetentionConfig,
    http1: http1::Http1Observer,
}

impl ApplicationProtocolAnalyzer {
    pub(super) fn new_with_retention(
        config: ApplicationProtocolConfig,
        semantic_retention: SemanticRetentionConfig,
    ) -> Self {
        Self {
            http1: http1::Http1Observer::new(),
            config,
            semantic_retention,
        }
    }

    pub(super) fn observe_http1(
        &mut self,
        events: Vec<semantic_action_runtime::Http1MessageEvent>,
        consumed_by_llm: bool,
    ) -> Vec<ApplicationEventDraft> {
        if !self.config.enabled || !self.config.http1_enabled {
            return Vec::new();
        }
        self.http1.observe(
            events,
            &self.config,
            &self.semantic_retention,
            consumed_by_llm,
        )
    }

    pub(super) fn observe_http2(
        &self,
        frames: Vec<semantic_action_runtime::Http2FrameObservation>,
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        if !self.config.enabled || !self.config.http2_enabled {
            return Ok(Vec::new());
        }
        http2::Http2Observer::observe(frames, &self.config, &self.semantic_retention, summary_only)
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.http1.forget_trace(trace_id);
    }

    pub(super) fn forget_stream(&mut self, identity: &PayloadStreamIdentity) {
        self.http1.forget_stream(identity);
    }
}

pub(super) fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(b0 >> 2) as usize] as char);
        output.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}
