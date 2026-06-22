use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use config_core::daemon::{ApplicationProtocolConfig, DiagnosticLogLevel, SseDataPolicy};
use export_core::{
    ExportError, ExportPublishResult, ExportRuntime, SemanticActionExportRecord,
    SemanticActionExportRoute,
};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::diagnostics::{DiagnosticKind, DiagnosticSeverity};
use model_core::ids::{CollectorName, ProfileName, TraceName};
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use semantic_action::SemanticActionKind;

use crate::profiles::DaemonProfileRegistry;

const FAILING_EXPORTER_NAME: &str = "test-failing-exporter";
const FAILING_EXPORTER_QUEUE_CAPACITY: u32 = 4;
const FAILING_EXPORTER_ERROR_CODE: &str = "test_exporter_unavailable";
const FAILING_EXPORTER_ERROR_MESSAGE: &str = "forced exporter failure";
const RECORDING_EXPORTER_NAME: &str = "test-recording-exporter";
const LIVE_EXPORT_PAYLOAD_SEQUENCE: u64 = 1;
const LIVE_EXPORT_OPERATION_OFFSET: u64 = 0;

#[test]
fn live_exporter_failure_is_diagnostic_not_payload_failure() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-live-export-failure-test-{}.sqlite",
        std::process::id()
    ));
    let profiles = DaemonProfileRegistry::new();
    let mut wiring = super::super::build_runtime_wiring(
        &super::test_storage_config(storage_path.clone()),
        profiles,
        super::ebpf_config(false),
        super::payload_config(true),
        super::DEFAULT_ACTIVE_TRACE_MAX,
        DiagnosticLogLevel::Info,
        super::seccomp_notify_disabled(),
        super::process_seccomp_disabled(),
        super::agent_invocation_disabled(),
        super::SemanticRetentionConfig::default(),
        super::FileObservationConfig::default(),
        ApplicationProtocolConfig {
            enabled: true,
            http1_enabled: true,
            http2_enabled: false,
            capture_host: true,
            sse_enabled: false,
            sse_data_policy: SseDataPolicy::Disabled,
            sse_max_buffer_bytes: super::TEST_HTTP_BUFFER_BYTES,
            sse_max_data_bytes: super::TEST_HTTP_BUFFER_BYTES,
            http2_max_frame_bytes: super::TEST_HTTP2_MAX_FRAME_BYTES,
            http2_max_connection_buffer_bytes: super::TEST_HTTP_BUFFER_BYTES,
            http2_emit_data_preview: false,
            http2_max_data_preview_bytes: super::TEST_HTTP2_PREVIEW_BYTES,
        },
        super::resource_metrics_disabled(),
        super::workload_diagnostics_disabled(),
        super::export_runtime_disabled(),
        super::enforcement_disabled(),
    )
    .unwrap();
    wiring.attach_service.export_runtime = ExportRuntime::new(vec![Box::new(FailingRoute)]);

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let process = ProcessIdentity::new(std::process::id(), 1, 1);
    super::create_active_trace(
        &mut wiring,
        trace_id,
        process.clone(),
        ProfileName::new("live-export"),
        TraceName::new("live-export"),
        vec![
            CapabilityRequest::new(Capability::TlsPlaintextPayload, RequestMode::Required),
            CapabilityRequest::new(
                Capability::NetApplicationPlaintextHttp,
                RequestMode::Required,
            ),
        ],
        super::APPLICATION_PROTOCOL_COLLECTOR,
        vec![Capability::NetApplicationPlaintextHttp],
    );

    let body = r#"{"model":"test-model","messages":[{"role":"user","content":"hello"}]}"#;
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: api.local\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    wiring
        .attach_service
        .process_payload_segment_impl(
            &wiring.trace_runtime,
            raw_tls_http_segment(trace_id, process, request.into_bytes()),
        )
        .unwrap();

    let actions = wiring
        .attach_service
        .storage
        .list_semantic_actions(trace_id)
        .unwrap();
    assert!(actions.iter().any(|action| {
        action.kind == SemanticActionKind::LlmRequest
            && action
                .attributes
                .get("llm.request.model")
                .map(String::as_str)
                == Some("test-model")
    }));

    let diagnostics = wiring
        .attach_service
        .storage
        .list_diagnostics(trace_id)
        .unwrap();
    let export_diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.kind == DiagnosticKind::RuntimeDropped
                && diagnostic.metadata.get("exporter").map(String::as_str)
                    == Some(FAILING_EXPORTER_NAME)
        })
        .collect::<Vec<_>>();
    assert_eq!(export_diagnostics.len(), 1);
    let diagnostic = export_diagnostics[0];
    let expected_reason =
        format!("{FAILING_EXPORTER_ERROR_CODE}: {FAILING_EXPORTER_ERROR_MESSAGE}");
    let expected_dropped_records = actions.len().to_string();
    let expected_queue_capacity = FAILING_EXPORTER_QUEUE_CAPACITY.to_string();
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
    assert_eq!(
        diagnostic.metadata.get("reason").map(String::as_str),
        Some(expected_reason.as_str())
    );
    assert_eq!(
        diagnostic
            .metadata
            .get("dropped_records")
            .map(String::as_str),
        Some(expected_dropped_records.as_str())
    );
    assert_eq!(
        diagnostic
            .metadata
            .get("queue_capacity")
            .map(String::as_str),
        Some(expected_queue_capacity.as_str())
    );
}

#[test]
fn tty_summary_is_persisted_but_not_live_exported() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-tty-live-export-test-{}.sqlite",
        std::process::id()
    ));
    let profiles = DaemonProfileRegistry::new();
    let mut wiring = super::super::build_runtime_wiring(
        &super::test_storage_config(storage_path.clone()),
        profiles,
        super::ebpf_config(false),
        super::payload_config(false),
        super::DEFAULT_ACTIVE_TRACE_MAX,
        DiagnosticLogLevel::Info,
        super::seccomp_notify_disabled(),
        super::process_seccomp_disabled(),
        super::agent_invocation_disabled(),
        super::SemanticRetentionConfig::default(),
        super::FileObservationConfig::default(),
        super::application_protocol_disabled(),
        super::resource_metrics_disabled(),
        super::workload_diagnostics_disabled(),
        super::export_runtime_disabled(),
        super::enforcement_disabled(),
    )
    .unwrap();
    let exported = Arc::new(Mutex::new(Vec::new()));
    wiring.attach_service.export_runtime = ExportRuntime::new(vec![Box::new(RecordingRoute {
        actions: Arc::clone(&exported),
    })]);

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let process = ProcessIdentity::new(920_100, 20_100, 20_100);
    super::create_active_trace(
        &mut wiring,
        trace_id,
        process.clone(),
        ProfileName::new("tty-export"),
        TraceName::new("tty-export"),
        vec![CapabilityRequest::new(
            Capability::FsAccessBasic,
            RequestMode::Required,
        )],
        "ebpf",
        vec![Capability::FsAccessBasic],
    );

    wiring
        .attach_service
        .process_live_event_batch(&mut wiring.trace_runtime, vec![raw_tty_write(process)])
        .unwrap();

    let actions = wiring
        .attach_service
        .storage
        .list_semantic_actions(trace_id)
        .unwrap();
    assert!(
        actions
            .iter()
            .any(|action| action.kind == SemanticActionKind::FileTtyIo)
    );
    assert!(exported.lock().unwrap().is_empty());
    let _ = std::fs::remove_file(storage_path);
}

struct FailingRoute;

impl SemanticActionExportRoute for FailingRoute {
    fn name(&self) -> &'static str {
        FAILING_EXPORTER_NAME
    }

    fn publish(
        &self,
        record: SemanticActionExportRecord<'_>,
    ) -> Result<ExportPublishResult, ExportError> {
        assert_eq!(record.trace.trace_id, record.action.trace_id);
        assert!(
            record
                .links
                .iter()
                .all(|link| link.trace_id == record.action.trace_id)
        );
        Err(
            ExportError::new(FAILING_EXPORTER_ERROR_CODE, FAILING_EXPORTER_ERROR_MESSAGE)
                .with_queue_capacity(FAILING_EXPORTER_QUEUE_CAPACITY),
        )
    }
}

struct RecordingRoute {
    actions: Arc<Mutex<Vec<SemanticActionKind>>>,
}

impl SemanticActionExportRoute for RecordingRoute {
    fn name(&self) -> &'static str {
        RECORDING_EXPORTER_NAME
    }

    fn publish(
        &self,
        record: SemanticActionExportRecord<'_>,
    ) -> Result<ExportPublishResult, ExportError> {
        self.actions.lock().unwrap().push(record.action.kind);
        Ok(ExportPublishResult::delivered())
    }
}

fn raw_tty_write(process: ProcessIdentity) -> RawCollectorEvent {
    RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::UNIX_EPOCH,
            process,
            collector: CollectorName::new("test-file"),
        },
        payload: RawObservationPayload::File {
            operation: "write".to_string(),
            path: Some("/dev/tty".to_string()),
            metadata: BTreeMap::from([
                ("fd".to_string(), "1".to_string()),
                ("fd_target".to_string(), "/dev/tty".to_string()),
                ("result".to_string(), "128".to_string()),
                ("size".to_string(), "128".to_string()),
            ]),
        },
    }
}

fn raw_tls_http_segment(
    trace_id: model_core::ids::TraceId,
    process: ProcessIdentity,
    bytes: Vec<u8>,
) -> RawPayloadSegment {
    RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::UNIX_EPOCH,
        process,
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: PayloadDirection::Outbound,
        stream_key: PayloadStreamKey::new("live-export-http"),
        sequence: LIVE_EXPORT_PAYLOAD_SEQUENCE,
        original_size: bytes.len() as u64,
        captured_size: bytes.len() as u64,
        operation_id: LIVE_EXPORT_PAYLOAD_SEQUENCE,
        operation_offset: LIVE_EXPORT_OPERATION_OFFSET,
        operation_original_size: bytes.len() as u64,
        operation_captured_size: bytes.len() as u64,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        library: "openssl".to_string(),
        symbol: "SSL_write".to_string(),
        protocol_hint: Some("http/1.1".to_string()),
        bytes,
    }
}
