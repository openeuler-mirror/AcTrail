use std::time::SystemTime;

use config_core::daemon::DiagnosticLogLevel;
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::{ProfileName, TraceName};
use model_core::payload::{
    PayloadContentState, PayloadDirection, PayloadOperationCompletionState, PayloadSourceBoundary,
    PayloadStreamKey, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;
use payload_event::RawPayloadSegment;
use storage_core::PayloadSegmentQuery;

use crate::profiles::DaemonProfileRegistry;

const TEST_SYNC_CHILD_PID_OFFSET: u32 = 10_000;
const TLS_SYNC_PAYLOAD_SEQUENCE: u64 = 1;
const TLS_SYNC_OPERATION_OFFSET: u64 = 0;
const TLS_SYNC_PAYLOAD: &[u8] = b"hello";

#[test]
fn tls_sync_payload_persists_without_child_membership() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-tls-sync-membership-test-{}.sqlite",
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
        super::application_protocol_disabled(),
        super::resource_metrics_disabled(),
        super::workload_diagnostics_disabled(),
        super::export_runtime_disabled(),
        super::enforcement_disabled(),
    )
    .unwrap();

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let root = ProcessIdentity::new(std::process::id(), 1, 1);
    super::create_active_trace(
        &mut wiring,
        trace_id,
        root,
        ProfileName::new("tls-sync"),
        TraceName::new("tls-sync"),
        vec![CapabilityRequest::new(
            Capability::TlsPlaintextPayload,
            RequestMode::Required,
        )],
        "tls-sync",
        vec![Capability::TlsPlaintextPayload],
    );
    let sync_process = ProcessIdentity::new(sync_child_pid(), 0, 0);
    wiring
        .attach_service
        .process_payload_segment_impl(
            &wiring.trace_runtime,
            raw_tls_sync_segment(trace_id, sync_process.clone()),
        )
        .unwrap();

    let segments = wiring
        .attach_service
        .storage
        .list_payload_segments(
            trace_id,
            PayloadSegmentQuery {
                segment_id: None,
                direction: None,
                limit: None,
                include_bytes: true,
            },
        )
        .unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].process, sync_process);
    assert_eq!(
        segments[0].source_boundary,
        PayloadSourceBoundary::TlsUserSpace
    );
    assert!(segments[0].bytes.is_empty());
    assert_eq!(segments[0].captured_size, TLS_SYNC_PAYLOAD.len() as u64);
    assert_eq!(segments[0].original_size, TLS_SYNC_PAYLOAD.len() as u64);
}

fn sync_child_pid() -> u32 {
    std::process::id()
        .checked_add(TEST_SYNC_CHILD_PID_OFFSET)
        .unwrap_or(TEST_SYNC_CHILD_PID_OFFSET)
}

fn raw_tls_sync_segment(
    trace_id: model_core::ids::TraceId,
    process: ProcessIdentity,
) -> RawPayloadSegment {
    RawPayloadSegment {
        trace_id,
        observed_at: SystemTime::UNIX_EPOCH,
        process,
        source_boundary: PayloadSourceBoundary::TlsUserSpace,
        content_state: PayloadContentState::Plaintext,
        direction: PayloadDirection::Outbound,
        stream_key: PayloadStreamKey::new("tls-sync-test"),
        sequence: TLS_SYNC_PAYLOAD_SEQUENCE,
        original_size: TLS_SYNC_PAYLOAD.len() as u64,
        captured_size: TLS_SYNC_PAYLOAD.len() as u64,
        operation_id: TLS_SYNC_PAYLOAD_SEQUENCE,
        operation_offset: TLS_SYNC_OPERATION_OFFSET,
        operation_original_size: TLS_SYNC_PAYLOAD.len() as u64,
        operation_captured_size: TLS_SYNC_PAYLOAD.len() as u64,
        operation_completion_state: PayloadOperationCompletionState::Success,
        truncation: PayloadTruncationState::Complete,
        library: "openssl".to_string(),
        symbol: "SSL_write".to_string(),
        protocol_hint: None,
        bytes: TLS_SYNC_PAYLOAD.to_vec(),
    }
}
