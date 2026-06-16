//! Application protocol analyzers over retained plaintext payload segments.

use std::collections::BTreeMap;

use config_core::daemon::{ApplicationProtocolConfig, SemanticRetentionConfig, SseDataPolicy};
use model_core::event::ApplicationPayload;
use model_core::ids::TraceId;
use model_core::payload::{
    PayloadContentState, PayloadSegment, PayloadSourceBoundary, PayloadStreamKey,
};
use model_core::process::ProcessIdentity;

#[path = "application_protocol/http1.rs"]
mod http1;
#[path = "application_protocol/http2.rs"]
mod http2;

pub(super) const COLLECTOR_NAME: &str = "application-protocol-analyzer";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ApplicationEventDraft {
    pub payload: ApplicationPayload,
}

pub(super) struct ApplicationProtocolAnalyzer {
    config: ApplicationProtocolConfig,
    semantic_retention: SemanticRetentionConfig,
    http1: http1::Http1Analyzer,
    http2: http2::Http2Analyzer,
    known_stream_protocols: BTreeMap<StreamProtocolKey, StreamProtocol>,
}

impl ApplicationProtocolAnalyzer {
    #[cfg(test)]
    pub(super) fn new(config: ApplicationProtocolConfig) -> Self {
        Self::new_with_retention(config, SemanticRetentionConfig::default())
    }

    pub(super) fn new_with_retention(
        config: ApplicationProtocolConfig,
        semantic_retention: SemanticRetentionConfig,
    ) -> Self {
        Self {
            http1: http1::Http1Analyzer::new(config.clone()),
            http2: http2::Http2Analyzer::new(config.clone()),
            known_stream_protocols: BTreeMap::new(),
            config,
            semantic_retention,
        }
    }

    #[cfg(test)]
    pub(super) fn analyze(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        self.analyze_with_semantic_context(segment, false, false)
    }

    pub(super) fn analyze_with_semantic_context(
        &mut self,
        segment: &PayloadSegment,
        consumed_by_llm: bool,
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        let config = if summary_only {
            summary_only_config(&self.config)
        } else {
            self.config.clone()
        };
        self.analyze_with_config(segment, config, consumed_by_llm, summary_only)
    }

    fn analyze_with_config(
        &mut self,
        segment: &PayloadSegment,
        config: ApplicationProtocolConfig,
        consumed_by_llm: bool,
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        if !config.enabled
            || segment.content_state != PayloadContentState::Plaintext
            || !matches!(
                segment.source_boundary,
                PayloadSourceBoundary::TlsUserSpace | PayloadSourceBoundary::Syscall
            )
        {
            return Ok(Vec::new());
        }

        let stream_key = stream_protocol_key(segment);
        if let Some(protocol) = self.known_stream_protocols.get(&stream_key).copied() {
            return self.analyze_known_protocol(
                segment,
                protocol,
                &config,
                consumed_by_llm,
                summary_only,
            );
        }

        let mut drafts = Vec::new();
        if config.http1_enabled {
            drafts.extend(self.http1.analyze_with_config(
                segment,
                &config,
                &self.semantic_retention,
                consumed_by_llm,
                summary_only,
            )?);
            if recognized_http1(&drafts) {
                self.known_stream_protocols
                    .insert(stream_key, StreamProtocol::Http1);
                return Ok(drafts);
            }
        }
        if config.http2_enabled {
            drafts.extend(self.http2.analyze_with_config(
                segment,
                &config,
                &self.semantic_retention,
                summary_only,
            )?);
            if recognized_http2(&drafts) {
                self.http1.forget_stream(segment);
                self.known_stream_protocols
                    .insert(stream_key, StreamProtocol::Http2);
            }
        }
        Ok(drafts)
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.known_stream_protocols
            .retain(|key, _| key.trace_id != trace_id);
        self.http1.forget_trace(trace_id);
        self.http2.forget_trace(trace_id);
    }

    #[cfg(test)]
    fn known_stream_protocol_count(&self) -> usize {
        self.known_stream_protocols.len()
    }

    #[cfg(test)]
    fn buffered_http1_stream_count(&self) -> usize {
        self.http1.buffered_stream_count()
    }

    fn analyze_known_protocol(
        &mut self,
        segment: &PayloadSegment,
        protocol: StreamProtocol,
        config: &ApplicationProtocolConfig,
        consumed_by_llm: bool,
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        match protocol {
            StreamProtocol::Http1 if config.http1_enabled => self.http1.analyze_with_config(
                segment,
                config,
                &self.semantic_retention,
                consumed_by_llm,
                summary_only,
            ),
            StreamProtocol::Http2 if config.http2_enabled => self.http2.analyze_with_config(
                segment,
                config,
                &self.semantic_retention,
                summary_only,
            ),
            _ => Ok(Vec::new()),
        }
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

fn summary_only_config(config: &ApplicationProtocolConfig) -> ApplicationProtocolConfig {
    let mut summary = config.clone();
    summary.sse_enabled = false;
    summary.sse_data_policy = SseDataPolicy::Disabled;
    summary.http2_emit_data_preview = false;
    summary
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamProtocol {
    Http1,
    Http2,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StreamProtocolKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: PayloadStreamKey,
}

fn stream_protocol_key(segment: &PayloadSegment) -> StreamProtocolKey {
    StreamProtocolKey {
        trace_id: segment.trace_id,
        process: segment.process.clone(),
        stream_key: segment.stream_key.clone(),
    }
}

fn recognized_http1(drafts: &[ApplicationEventDraft]) -> bool {
    drafts.iter().any(|draft| {
        let protocol = draft.payload.protocol.as_str();
        matches!(protocol, "http/1.0" | "http/1.1" | "sse")
    })
}

fn recognized_http2(drafts: &[ApplicationEventDraft]) -> bool {
    drafts
        .iter()
        .any(|draft| draft.payload.protocol.as_str() == "h2")
}

#[cfg(test)]
#[path = "application_protocol/tests.rs"]
mod tests;
