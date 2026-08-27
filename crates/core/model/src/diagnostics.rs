//! Diagnostics emitted by capability negotiation, runtime, policy, and retention.

use std::collections::BTreeMap;
use std::time::SystemTime;

use crate::ids::{DiagnosticId, TraceId};
use crate::process::ProcessIdentity;

const LLM_DIAGNOSTIC_STREAM_KEY_MAX_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    CapabilityRejected,
    OpportunisticUnbound,
    BootstrapPartial,
    BootstrapGap,
    IdentityUnverified,
    IdentityMismatch,
    RuntimeDropped,
    RuntimeFailure,
    RuntimeFatal,
    PolicyFiltered,
    PolicyRedacted,
    PolicyTruncated,
    TracePurged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Stable persisted reason code for one LLM pipeline failure or discard.
///
/// Values are assigned explicitly by [`Self::as_u16`]. The ranges identify
/// the owning component: 2xxx correlation, 3xxx HTTP/1, 4xxx HTTP/2,
/// 5xxx lifecycle, 6xxx provider codecs, 7xxx tool projection, and 8xxx
/// WebSocket transport.
pub enum LlmPipelineDiagnosticCode {
    CorrelationStreamCapacityEvicted,
    CorrelationSequenceExhausted,
    PendingRequestCapacityEvicted,
    PendingResponseCapacityEvicted,
    ConfirmedHttpExchangeCapacityEvicted,
    ActiveResponseBindingCapacityEvicted,
    DamagedResponseBindingCapacityEvicted,
    LateHttpFailureBindingCapacityEvicted,
    DamagedHttpResponseCapacityEvicted,
    PendingTrajectoryCapacityEvicted,
    ActionVersionCapacityEvicted,
    ActionVersionSequenceExhausted,
    LateHttpFailureBindingMissing,
    Http1InvalidHead,
    Http1InvalidContentLength,
    Http1InvalidChunkSize,
    Http1InvalidChunkTerminator,
    Http1DecoderBufferCapacityExceeded,
    Http1ProtocolDecodeFailed,
    Http1AssemblyBufferCapacityExceeded,
    Http1AssemblySegmentCapacityExceeded,
    Http1ConfirmedGap,
    Http1OperationIncomplete,
    Http1TransportReset,
    Http1IncompleteRequestUnprojectableAtClose,
    Http1UnclassifiedBytesDiscardedAtClose,
    Http2ProtocolDecodeFailed,
    Http2PeerStreamReset,
    Http2AssemblyBufferCapacityExceeded,
    Http2AssemblySegmentCapacityExceeded,
    Http2ConfirmedGap,
    Http2OperationIncomplete,
    Http2IncompleteRequestUnprojectableAtClose,
    Http2ConfirmedResponseUnprojectableAtEndStream,
    DesynchronizedBytesDiscarded,
    ProviderCodecDecodeFailed,
    ToolCallsJsonInvalid,
    ToolCallNameMissing,
    ToolResultRequestMissing,
    ToolResultCallIdMissing,
    ToolResultCallUnmatched,
    ToolResultCallAmbiguous,
    ToolStateCapacityEvicted,
    WebSocketConnectionCapacityEvicted,
    WebSocketDecodeFailed,
    WebSocketPendingFrameBufferExceeded,
    WebSocketResponseBytesExceeded,
    WebSocketActiveResponseSuperseded,
    WebSocketLifecycleInterrupted,
}

impl LlmPipelineDiagnosticCode {
    pub const fn as_u16(self) -> u16 {
        match self {
            Self::CorrelationStreamCapacityEvicted => 2001,
            Self::CorrelationSequenceExhausted => 2002,
            Self::PendingRequestCapacityEvicted => 2010,
            Self::PendingResponseCapacityEvicted => 2011,
            Self::ConfirmedHttpExchangeCapacityEvicted => 2012,
            Self::ActiveResponseBindingCapacityEvicted => 2013,
            Self::DamagedResponseBindingCapacityEvicted => 2014,
            Self::LateHttpFailureBindingCapacityEvicted => 2015,
            Self::DamagedHttpResponseCapacityEvicted => 2016,
            Self::PendingTrajectoryCapacityEvicted => 2017,
            Self::ActionVersionCapacityEvicted => 2018,
            Self::ActionVersionSequenceExhausted => 2019,
            Self::LateHttpFailureBindingMissing => 2020,
            Self::Http1InvalidHead => 3001,
            Self::Http1InvalidContentLength => 3002,
            Self::Http1InvalidChunkSize => 3003,
            Self::Http1InvalidChunkTerminator => 3004,
            Self::Http1DecoderBufferCapacityExceeded => 3005,
            Self::Http1ProtocolDecodeFailed => 3006,
            Self::Http1AssemblyBufferCapacityExceeded => 3010,
            Self::Http1AssemblySegmentCapacityExceeded => 3011,
            Self::Http1ConfirmedGap => 3012,
            Self::Http1OperationIncomplete => 3013,
            Self::Http1TransportReset => 3014,
            Self::Http1IncompleteRequestUnprojectableAtClose => 3020,
            Self::Http1UnclassifiedBytesDiscardedAtClose => 3021,
            Self::Http2ProtocolDecodeFailed => 4001,
            Self::Http2PeerStreamReset => 4002,
            Self::Http2AssemblyBufferCapacityExceeded => 4010,
            Self::Http2AssemblySegmentCapacityExceeded => 4011,
            Self::Http2ConfirmedGap => 4012,
            Self::Http2OperationIncomplete => 4013,
            Self::Http2IncompleteRequestUnprojectableAtClose => 4020,
            Self::Http2ConfirmedResponseUnprojectableAtEndStream => 4021,
            Self::DesynchronizedBytesDiscarded => 5001,
            Self::ProviderCodecDecodeFailed => 6001,
            Self::ToolCallsJsonInvalid => 7001,
            Self::ToolCallNameMissing => 7002,
            Self::ToolResultRequestMissing => 7003,
            Self::ToolResultCallIdMissing => 7004,
            Self::ToolResultCallUnmatched => 7005,
            Self::ToolResultCallAmbiguous => 7006,
            Self::ToolStateCapacityEvicted => 7010,
            Self::WebSocketConnectionCapacityEvicted => 8001,
            Self::WebSocketDecodeFailed => 8002,
            Self::WebSocketPendingFrameBufferExceeded => 8010,
            Self::WebSocketResponseBytesExceeded => 8011,
            Self::WebSocketActiveResponseSuperseded => 8020,
            Self::WebSocketLifecycleInterrupted => 8021,
        }
    }

    pub const fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            2001 => Self::CorrelationStreamCapacityEvicted,
            2002 => Self::CorrelationSequenceExhausted,
            2010 => Self::PendingRequestCapacityEvicted,
            2011 => Self::PendingResponseCapacityEvicted,
            2012 => Self::ConfirmedHttpExchangeCapacityEvicted,
            2013 => Self::ActiveResponseBindingCapacityEvicted,
            2014 => Self::DamagedResponseBindingCapacityEvicted,
            2015 => Self::LateHttpFailureBindingCapacityEvicted,
            2016 => Self::DamagedHttpResponseCapacityEvicted,
            2017 => Self::PendingTrajectoryCapacityEvicted,
            2018 => Self::ActionVersionCapacityEvicted,
            2019 => Self::ActionVersionSequenceExhausted,
            2020 => Self::LateHttpFailureBindingMissing,
            3001 => Self::Http1InvalidHead,
            3002 => Self::Http1InvalidContentLength,
            3003 => Self::Http1InvalidChunkSize,
            3004 => Self::Http1InvalidChunkTerminator,
            3005 => Self::Http1DecoderBufferCapacityExceeded,
            3006 => Self::Http1ProtocolDecodeFailed,
            3010 => Self::Http1AssemblyBufferCapacityExceeded,
            3011 => Self::Http1AssemblySegmentCapacityExceeded,
            3012 => Self::Http1ConfirmedGap,
            3013 => Self::Http1OperationIncomplete,
            3014 => Self::Http1TransportReset,
            3020 => Self::Http1IncompleteRequestUnprojectableAtClose,
            3021 => Self::Http1UnclassifiedBytesDiscardedAtClose,
            4001 => Self::Http2ProtocolDecodeFailed,
            4002 => Self::Http2PeerStreamReset,
            4010 => Self::Http2AssemblyBufferCapacityExceeded,
            4011 => Self::Http2AssemblySegmentCapacityExceeded,
            4012 => Self::Http2ConfirmedGap,
            4013 => Self::Http2OperationIncomplete,
            4020 => Self::Http2IncompleteRequestUnprojectableAtClose,
            4021 => Self::Http2ConfirmedResponseUnprojectableAtEndStream,
            5001 => Self::DesynchronizedBytesDiscarded,
            6001 => Self::ProviderCodecDecodeFailed,
            7001 => Self::ToolCallsJsonInvalid,
            7002 => Self::ToolCallNameMissing,
            7003 => Self::ToolResultRequestMissing,
            7004 => Self::ToolResultCallIdMissing,
            7005 => Self::ToolResultCallUnmatched,
            7006 => Self::ToolResultCallAmbiguous,
            7010 => Self::ToolStateCapacityEvicted,
            8001 => Self::WebSocketConnectionCapacityEvicted,
            8002 => Self::WebSocketDecodeFailed,
            8010 => Self::WebSocketPendingFrameBufferExceeded,
            8011 => Self::WebSocketResponseBytesExceeded,
            8020 => Self::WebSocketActiveResponseSuperseded,
            8021 => Self::WebSocketLifecycleInterrupted,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CorrelationStreamCapacityEvicted => "correlation_stream_capacity_evicted",
            Self::CorrelationSequenceExhausted => "correlation_sequence_exhausted",
            Self::PendingRequestCapacityEvicted => "pending_request_capacity_evicted",
            Self::PendingResponseCapacityEvicted => "pending_response_capacity_evicted",
            Self::ConfirmedHttpExchangeCapacityEvicted => {
                "confirmed_http_exchange_capacity_evicted"
            }
            Self::ActiveResponseBindingCapacityEvicted => {
                "active_response_binding_capacity_evicted"
            }
            Self::DamagedResponseBindingCapacityEvicted => {
                "damaged_response_binding_capacity_evicted"
            }
            Self::LateHttpFailureBindingCapacityEvicted => {
                "late_http_failure_binding_capacity_evicted"
            }
            Self::DamagedHttpResponseCapacityEvicted => "damaged_http_response_capacity_evicted",
            Self::PendingTrajectoryCapacityEvicted => "pending_trajectory_capacity_evicted",
            Self::ActionVersionCapacityEvicted => "action_version_capacity_evicted",
            Self::ActionVersionSequenceExhausted => "action_version_sequence_exhausted",
            Self::LateHttpFailureBindingMissing => "late_http_failure_binding_missing",
            Self::Http1InvalidHead => "http1_invalid_head",
            Self::Http1InvalidContentLength => "http1_invalid_content_length",
            Self::Http1InvalidChunkSize => "http1_invalid_chunk_size",
            Self::Http1InvalidChunkTerminator => "http1_invalid_chunk_terminator",
            Self::Http1DecoderBufferCapacityExceeded => "http1_decoder_buffer_capacity_exceeded",
            Self::Http1ProtocolDecodeFailed => "http1_protocol_decode_failed",
            Self::Http1AssemblyBufferCapacityExceeded => "http1_assembly_buffer_capacity_exceeded",
            Self::Http1AssemblySegmentCapacityExceeded => {
                "http1_assembly_segment_capacity_exceeded"
            }
            Self::Http1ConfirmedGap => "http1_confirmed_gap",
            Self::Http1OperationIncomplete => "http1_operation_incomplete",
            Self::Http1TransportReset => "http1_transport_reset",
            Self::Http1IncompleteRequestUnprojectableAtClose => {
                "http1_incomplete_request_unprojectable_at_close"
            }
            Self::Http1UnclassifiedBytesDiscardedAtClose => {
                "http1_unclassified_bytes_discarded_at_close"
            }
            Self::Http2ProtocolDecodeFailed => "http2_protocol_decode_failed",
            Self::Http2PeerStreamReset => "http2_peer_stream_reset",
            Self::Http2AssemblyBufferCapacityExceeded => "http2_assembly_buffer_capacity_exceeded",
            Self::Http2AssemblySegmentCapacityExceeded => {
                "http2_assembly_segment_capacity_exceeded"
            }
            Self::Http2ConfirmedGap => "http2_confirmed_gap",
            Self::Http2OperationIncomplete => "http2_operation_incomplete",
            Self::Http2IncompleteRequestUnprojectableAtClose => {
                "http2_incomplete_request_unprojectable_at_close"
            }
            Self::Http2ConfirmedResponseUnprojectableAtEndStream => {
                "http2_confirmed_response_unprojectable_at_end_stream"
            }
            Self::DesynchronizedBytesDiscarded => "desynchronized_bytes_discarded",
            Self::ProviderCodecDecodeFailed => "provider_codec_decode_failed",
            Self::ToolCallsJsonInvalid => "tool_calls_json_invalid",
            Self::ToolCallNameMissing => "tool_call_name_missing",
            Self::ToolResultRequestMissing => "tool_result_request_missing",
            Self::ToolResultCallIdMissing => "tool_result_call_id_missing",
            Self::ToolResultCallUnmatched => "tool_result_call_unmatched",
            Self::ToolResultCallAmbiguous => "tool_result_call_ambiguous",
            Self::ToolStateCapacityEvicted => "tool_state_capacity_evicted",
            Self::WebSocketConnectionCapacityEvicted => "websocket_connection_capacity_evicted",
            Self::WebSocketDecodeFailed => "websocket_decode_failed",
            Self::WebSocketPendingFrameBufferExceeded => "websocket_pending_frame_buffer_exceeded",
            Self::WebSocketResponseBytesExceeded => "websocket_response_bytes_exceeded",
            Self::WebSocketActiveResponseSuperseded => "websocket_active_response_superseded",
            Self::WebSocketLifecycleInterrupted => "websocket_lifecycle_interrupted",
        }
    }

    pub const fn is_runtime_drop(self) -> bool {
        !matches!(
            self,
            Self::CorrelationSequenceExhausted
                | Self::ActionVersionSequenceExhausted
                | Self::LateHttpFailureBindingMissing
                | Self::Http1InvalidHead
                | Self::Http1InvalidContentLength
                | Self::Http1InvalidChunkSize
                | Self::Http1InvalidChunkTerminator
                | Self::Http1DecoderBufferCapacityExceeded
                | Self::Http1ProtocolDecodeFailed
                | Self::Http1IncompleteRequestUnprojectableAtClose
                | Self::Http2ProtocolDecodeFailed
                | Self::Http2IncompleteRequestUnprojectableAtClose
                | Self::Http2ConfirmedResponseUnprojectableAtEndStream
                | Self::ProviderCodecDecodeFailed
                | Self::ToolCallsJsonInvalid
                | Self::ToolCallNameMissing
                | Self::ToolResultRequestMissing
                | Self::ToolResultCallIdMissing
                | Self::ToolResultCallUnmatched
                | Self::ToolResultCallAmbiguous
                | Self::WebSocketDecodeFailed
                | Self::WebSocketLifecycleInterrupted
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LlmPipelineDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LlmPipelineDiagnosticStage {
    Classifier,
    Correlation,
    Http1,
    Http2,
    Lifecycle,
    ProviderCodec,
    ToolProjection,
    WebSocket,
}

impl LlmPipelineDiagnosticStage {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Classifier => 1,
            Self::Correlation => 2,
            Self::Http1 => 3,
            Self::Http2 => 4,
            Self::Lifecycle => 5,
            Self::ProviderCodec => 6,
            Self::ToolProjection => 7,
            Self::WebSocket => 8,
        }
    }

    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Classifier,
            2 => Self::Correlation,
            3 => Self::Http1,
            4 => Self::Http2,
            5 => Self::Lifecycle,
            6 => Self::ProviderCodec,
            7 => Self::ToolProjection,
            8 => Self::WebSocket,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Classifier => "classifier",
            Self::Correlation => "correlation",
            Self::Http1 => "http1",
            Self::Http2 => "http2",
            Self::Lifecycle => "lifecycle",
            Self::ProviderCodec => "provider_codec",
            Self::ToolProjection => "tool_projection",
            Self::WebSocket => "websocket",
        }
    }
}

impl LlmPipelineDiagnosticSeverity {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Info => 1,
            Self::Warning => 2,
            Self::Error => 3,
        }
    }

    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Info,
            2 => Self::Warning,
            3 => Self::Error,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Bounded, payload-free diagnostic emitted by the live LLM pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LlmPipelineDiagnostic {
    trace_id: TraceId,
    process: ProcessIdentity,
    stream_key: Option<String>,
    stage: LlmPipelineDiagnosticStage,
    code: LlmPipelineDiagnosticCode,
    severity: LlmPipelineDiagnosticSeverity,
    observed_at: SystemTime,
    discarded_bytes: Option<u64>,
    discarded_entries: Option<u64>,
}

impl LlmPipelineDiagnostic {
    pub fn new(
        trace_id: TraceId,
        process: &ProcessIdentity,
        observed_at: SystemTime,
        code: LlmPipelineDiagnosticCode,
        severity: LlmPipelineDiagnosticSeverity,
        stage: LlmPipelineDiagnosticStage,
    ) -> Self {
        Self {
            trace_id,
            process: *process,
            stream_key: None,
            stage,
            code,
            severity,
            observed_at,
            discarded_bytes: None,
            discarded_entries: None,
        }
    }

    pub fn with_stream_key(mut self, stream_key: &str) -> Self {
        let mut chars = stream_key.chars();
        let mut bounded = chars
            .by_ref()
            .take(LLM_DIAGNOSTIC_STREAM_KEY_MAX_CHARS)
            .collect::<String>();
        if chars.next().is_some() {
            bounded.push_str("...");
        }
        self.stream_key = Some(bounded);
        self
    }

    pub fn with_discarded_bytes(mut self, discarded_bytes: u64) -> Self {
        self.discarded_bytes = Some(discarded_bytes);
        self
    }

    pub fn with_discarded_entries(mut self, discarded_entries: u64) -> Self {
        self.discarded_entries = Some(discarded_entries);
        self
    }

    pub fn trace_id(&self) -> TraceId {
        self.trace_id
    }

    pub fn process(&self) -> &ProcessIdentity {
        &self.process
    }

    pub fn stream_key(&self) -> Option<&str> {
        self.stream_key.as_deref()
    }

    pub fn stage(&self) -> LlmPipelineDiagnosticStage {
        self.stage
    }

    pub fn code(&self) -> LlmPipelineDiagnosticCode {
        self.code
    }

    pub fn severity(&self) -> LlmPipelineDiagnosticSeverity {
        self.severity
    }

    pub fn observed_at(&self) -> SystemTime {
        self.observed_at
    }

    pub fn discarded_bytes(&self) -> Option<u64> {
        self.discarded_bytes
    }

    pub fn discarded_entries(&self) -> Option<u64> {
        self.discarded_entries
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticRecord {
    pub diagnostic_id: DiagnosticId,
    pub trace_id: Option<TraceId>,
    pub process: Option<ProcessIdentity>,
    pub kind: DiagnosticKind,
    pub severity: DiagnosticSeverity,
    pub emitted_at: SystemTime,
    pub message: String,
    pub metadata: BTreeMap<String, String>,
}

impl DiagnosticRecord {
    pub fn new(
        diagnostic_id: DiagnosticId,
        trace_id: Option<TraceId>,
        kind: DiagnosticKind,
        severity: DiagnosticSeverity,
        emitted_at: SystemTime,
        message: impl Into<String>,
    ) -> Self {
        Self {
            diagnostic_id,
            trace_id,
            process: None,
            kind,
            severity,
            emitted_at,
            message: message.into(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_process(mut self, process: ProcessIdentity) -> Self {
        self.process = Some(process);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}
