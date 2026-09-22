use std::sync::Arc;
use std::time::SystemTime;

use super::Http2StreamAssembly;
use crate::llm_pipeline::assembly::router::PayloadStreamGroupKey;
use crate::llm_pipeline::projection::ProjectionBatch as LiveLlmOutput;
use crate::llm_pipeline::projection::projector::project_http2_stream_request;
use crate::llm_pipeline::provider::codec::LlmCodecRegistry;
use crate::llm_pipeline::stream::finalizer::{ResponseFinalizer, StreamFinalizationReason};
use crate::llm_pipeline::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use config_core::daemon::SemanticRetentionConfig;

impl Http2StreamAssembly {
    pub(super) fn project_request(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        stream_id: u32,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        if !self.end_stream || self.plain.buffer.is_empty() {
            return output;
        }
        let message_start = self.plain.base_offset;
        let message_end = message_start + self.plain.buffer.len();
        let segments = self.plain.segments_for_range(message_start, message_end);
        let Some(projection) = project_http2_stream_request(
            config,
            codecs,
            key,
            stream_id,
            message_start,
            &self.plain.buffer,
            Arc::clone(&self.body),
            &segments,
            true,
        ) else {
            return output;
        };
        output.actions.extend(projection.actions);

        output
            .llm_request_contents
            .extend(projection.llm_request_contents);
        output
            .llm_request_histories
            .extend(projection.llm_request_histories);
        output.llm_tool_results.extend(projection.llm_tool_results);
        self.plain.evict_encoded_len(projection.encoded_len);
        output
    }

    pub(super) fn materialize_incomplete_request(
        &mut self,
        config: &SemanticRetentionConfig,
        codecs: &LlmCodecRegistry,
        key: &PayloadStreamGroupKey,
        stream_id: u32,
        reason: StreamFinalizationReason,
        finished_at: SystemTime,
    ) -> LiveLlmOutput {
        let mut output = LiveLlmOutput::default();
        if self.plain.buffer.is_empty() {
            return output;
        }
        let buffered_bytes = self.plain.buffer.len();
        let retained_ranges = self.plain.segments.len();
        let message_start = self.plain.base_offset;
        let message_end = message_start.saturating_add(buffered_bytes);
        let segments = self.plain.segments_for_range(message_start, message_end);
        if let Some(mut projection) = project_http2_stream_request(
            config,
            codecs,
            key,
            stream_id,
            message_start,
            &self.plain.buffer,
            Arc::clone(&self.body),
            &segments,
            false,
        ) && !projection.actions.is_empty()
        {
            for action in &mut projection.actions {
                ResponseFinalizer::finalize_incomplete(action, reason, finished_at);
            }
            output.actions.extend(projection.actions);

            output
                .llm_request_contents
                .extend(projection.llm_request_contents);
            output
                .llm_request_histories
                .extend(projection.llm_request_histories);
            output.llm_tool_results.extend(projection.llm_tool_results);
            output.payload_segments.extend(projection.payload_segments);
            return output;
        }
        if reason == StreamFinalizationReason::CapturePolicyLimited {
            return output;
        }
        let diagnostic_stream_key = format!("{}#h2:{}", key.stream_key, stream_id);
        output.diagnostics.push(
            LlmPipelineDiagnostic::new(
                key.trace_id,
                &key.process,
                finished_at,
                LlmPipelineDiagnosticCode::Http2IncompleteRequestUnprojectableAtClose,
                LlmPipelineDiagnosticSeverity::Warning,
                LlmPipelineDiagnosticStage::Http2,
            )
            .with_stream_key(&diagnostic_stream_key)
            .with_discarded_bytes(u64::try_from(buffered_bytes).unwrap_or(u64::MAX))
            .with_discarded_entries(u64::try_from(retained_ranges).unwrap_or(u64::MAX)),
        );
        output
    }
}
