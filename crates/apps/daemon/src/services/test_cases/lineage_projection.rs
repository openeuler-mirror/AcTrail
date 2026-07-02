use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use config_core::daemon::DiagnosticLogLevel;
use model_core::capability::{Capability, CapabilityRequest, RequestMode};
use model_core::ids::{CollectorName, ProfileName, TraceName};
use model_core::process::{ExitObservationSource, MembershipState, ProcessIdentity};
use model_core::trace::TraceLifecycleState;
use semantic_action::{SemanticActionKind, SemanticActionLinkRole};

use crate::profiles::DaemonProfileRegistry;

#[test]
fn terminal_reconcile_projects_late_exec_through_process_lineage() {
    let storage_path = std::env::temp_dir().join(format!(
        "actrail-lineage-terminal-test-{}.sqlite",
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
        super::TraceFinalizationConfig::default(),
        super::workload_diagnostics_disabled(),
        super::export_runtime_disabled(),
        super::enforcement_disabled(),
        super::CommandControlConfig::default(),
    )
    .unwrap();

    let trace_id = wiring.trace_runtime.reserve_trace_id();
    let root = ProcessIdentity::new(910_100, 10_100, 10_100);
    let helper = ProcessIdentity::new(910_101, 10_101, 10_101);
    let base64 = ProcessIdentity::new(910_102, 10_102, 10_102);
    super::create_active_trace(
        &mut wiring,
        trace_id,
        root.clone(),
        ProfileName::new("lineage-terminal"),
        TraceName::new("lineage-terminal"),
        vec![CapabilityRequest::new(
            Capability::ProcLifecycle,
            RequestMode::Required,
        )],
        "ebpf",
        vec![Capability::ProcLifecycle],
    );

    wiring
        .attach_service
        .process_live_event_batch(
            &mut wiring.trace_runtime,
            vec![
                raw_exec(
                    root.clone(),
                    None,
                    "/bin/bash",
                    "/bin/bash -c source snapshot",
                    1,
                ),
                raw_fork(helper.clone(), root.clone(), 2),
                raw_exit(helper.clone(), 0, 3),
                raw_exit(root.clone(), 0, 4),
            ],
        )
        .unwrap();
    assert_eq!(
        wiring
            .trace_runtime
            .get_trace(trace_id)
            .unwrap()
            .trace
            .lifecycle_state,
        TraceLifecycleState::Exited
    );

    wiring
        .attach_service
        .process_live_event_batch(
            &mut wiring.trace_runtime,
            vec![raw_exec(
                base64.clone(),
                Some(helper),
                "/usr/bin/base64",
                "base64 -d",
                5,
            )],
        )
        .unwrap();
    wiring
        .attach_service
        .reconcile_draining_memberships_impl(&mut wiring.trace_runtime)
        .unwrap();
    wiring
        .attach_service
        .finalize_terminal_traces_impl(&wiring.trace_runtime)
        .unwrap();

    let memberships = wiring
        .attach_service
        .storage
        .trace_memberships(trace_id)
        .unwrap();
    let base64_membership = memberships
        .iter()
        .find(|membership| membership.identity == base64)
        .expect("late base64 membership persisted");
    assert_eq!(base64_membership.state, MembershipState::Exited);
    assert_eq!(
        base64_membership
            .exit_status
            .as_ref()
            .and_then(|status| status.source),
        Some(ExitObservationSource::Reconciled)
    );

    let actions = wiring
        .attach_service
        .storage
        .list_semantic_actions(trace_id)
        .unwrap();
    let root_command = actions
        .iter()
        .find(|action| {
            action.kind == SemanticActionKind::CommandInvocation && action.process == root
        })
        .expect("root command action");
    let base64_command = actions
        .iter()
        .find(|action| {
            action.kind == SemanticActionKind::CommandInvocation && action.process == base64
        })
        .expect("base64 command action");
    let links = wiring
        .attach_service
        .storage
        .list_semantic_action_links(trace_id)
        .unwrap();
    assert!(links.iter().any(|link| {
        link.parent_action_id == root_command.action_id
            && link.child_action_id == base64_command.action_id
            && link.role == SemanticActionLinkRole::CommandContainsCommandInvocation
    }));
}

fn raw_exec(
    process: ProcessIdentity,
    parent: Option<ProcessIdentity>,
    executable: &str,
    command_line: &str,
    observed_second: u64,
) -> RawCollectorEvent {
    let mut metadata = BTreeMap::new();
    metadata.insert("executable".to_string(), executable.to_string());
    metadata.insert("command_line".to_string(), command_line.to_string());
    if let Some(parent) = &parent {
        metadata.insert("ppid".to_string(), parent.pid.to_string());
    }
    raw_process_event(process, "exec", parent, metadata, observed_second)
}

fn raw_fork(
    process: ProcessIdentity,
    parent: ProcessIdentity,
    observed_second: u64,
) -> RawCollectorEvent {
    raw_process_event(
        process,
        "fork",
        Some(parent),
        BTreeMap::new(),
        observed_second,
    )
}

fn raw_exit(process: ProcessIdentity, code: i32, observed_second: u64) -> RawCollectorEvent {
    raw_process_event(
        process,
        "exit",
        None,
        BTreeMap::from([("exit_code".to_string(), code.to_string())]),
        observed_second,
    )
}

fn raw_process_event(
    process: ProcessIdentity,
    operation: &str,
    parent: Option<ProcessIdentity>,
    metadata: BTreeMap<String, String>,
    observed_second: u64,
) -> RawCollectorEvent {
    RawCollectorEvent {
        envelope: RawEventEnvelope {
            observed_at: SystemTime::UNIX_EPOCH + Duration::from_secs(observed_second),
            process,
            collector: CollectorName::new("test-process"),
        },
        payload: RawObservationPayload::Process {
            operation: operation.to_string(),
            parent,
            metadata,
        },
    }
}
