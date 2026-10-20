//! A localized termination emits one diagnostic at its owning boundary.
use super::protocol::Http2Protocol;
use crate::llm_pipeline::assembly::router::PayloadStreamGroupKey;
use crate::llm_pipeline::projection::ProjectionBatch;
use crate::llm_pipeline::stream::finalizer::StreamFinalizationReason;
use model_core::diagnostics::{
    LlmPipelineDiagnostic, LlmPipelineDiagnosticCode as Code, LlmPipelineDiagnosticSeverity,
    LlmPipelineDiagnosticStage,
};
use std::time::SystemTime;

impl Http2Protocol {
    pub(super) fn diagnose(
        reason: StreamFinalizationReason,
        key: &PayloadStreamGroupKey,
        at: SystemTime,
        stream_id: Option<u32>,
        output: &mut ProjectionBatch,
    ) {
        let code = match reason {
            StreamFinalizationReason::ProtocolDecodeFailed => Code::Http2ProtocolDecodeFailed,
            StreamFinalizationReason::Http2StreamReset => Code::Http2PeerStreamReset,
            StreamFinalizationReason::BufferBytesExceeded => {
                Code::Http2AssemblyBufferCapacityExceeded
            }
            StreamFinalizationReason::SegmentRangesExceeded => {
                Code::Http2AssemblySegmentCapacityExceeded
            }
            StreamFinalizationReason::Http2ActiveStreamCapacity => {
                Code::Http2ActiveStreamsCapacityExceeded
            }
            StreamFinalizationReason::ConfirmedGap => Code::Http2ConfirmedGap,
            StreamFinalizationReason::OperationIncomplete => Code::Http2OperationIncomplete,
            _ => return,
        };
        let stream_key = stream_id.map_or_else(
            || key.stream_key.clone(),
            |id| format!("{}#h2:{id}", key.stream_key),
        );
        output.diagnostics.push(
            LlmPipelineDiagnostic::new(
                key.trace_id,
                &key.process,
                at,
                code,
                LlmPipelineDiagnosticSeverity::Warning,
                LlmPipelineDiagnosticStage::Http2,
            )
            .with_stream_key(&stream_key),
        );
    }
}
