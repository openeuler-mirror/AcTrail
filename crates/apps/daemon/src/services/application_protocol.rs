//! Application protocol analyzers over retained plaintext payload segments.

use std::collections::BTreeMap;

use config_core::daemon::{ApplicationProtocolConfig, SseDataPolicy};
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
    http1: http1::Http1Analyzer,
    http2: http2::Http2Analyzer,
    known_stream_protocols: BTreeMap<StreamProtocolKey, StreamProtocol>,
}

impl ApplicationProtocolAnalyzer {
    pub(super) fn new(config: ApplicationProtocolConfig) -> Self {
        Self {
            http1: http1::Http1Analyzer::new(config.clone()),
            http2: http2::Http2Analyzer::new(config.clone()),
            known_stream_protocols: BTreeMap::new(),
            config,
        }
    }

    pub(super) fn analyze(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        self.analyze_with_config(segment, self.config.clone(), false)
    }

    pub(super) fn analyze_summary_only(
        &mut self,
        segment: &PayloadSegment,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        self.analyze_with_config(segment, summary_only_config(&self.config), true)
    }

    fn analyze_with_config(
        &mut self,
        segment: &PayloadSegment,
        config: ApplicationProtocolConfig,
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
            return self.analyze_known_protocol(segment, protocol, &config, summary_only);
        }

        let mut drafts = Vec::new();
        if config.http1_enabled {
            drafts.extend(
                self.http1
                    .analyze_with_config(segment, &config, summary_only)?,
            );
            if recognized_http1(&drafts) {
                self.known_stream_protocols
                    .insert(stream_key, StreamProtocol::Http1);
                return Ok(drafts);
            }
        }
        if config.http2_enabled {
            drafts.extend(
                self.http2
                    .analyze_with_config(segment, &config, summary_only)?,
            );
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
        summary_only: bool,
    ) -> Result<Vec<ApplicationEventDraft>, String> {
        match protocol {
            StreamProtocol::Http1 if config.http1_enabled => {
                self.http1
                    .analyze_with_config(segment, config, summary_only)
            }
            StreamProtocol::Http2 if config.http2_enabled => {
                self.http2
                    .analyze_with_config(segment, config, summary_only)
            }
            _ => Ok(Vec::new()),
        }
    }
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
