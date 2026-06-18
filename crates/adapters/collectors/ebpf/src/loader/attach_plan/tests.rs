use std::collections::BTreeSet;

use config_core::daemon::{
    EbpfCollectorConfig, OPERATOR_CONFIG_TEMPLATE, OperatorConfig, PayloadConfig,
};
use model_core::capability::{Capability, CapabilityRequest, RequestMode};

use super::AttachPlan;

#[test]
fn proc_lifecycle_request_skips_file_and_mmap_programs() {
    let config = config();
    let payload = payload_config();
    let plan = AttachPlan::from_requests(
        &[CapabilityRequest::new(
            Capability::ProcLifecycle,
            RequestMode::Required,
        )],
        &config,
        &payload,
    );

    assert!(
        plan.should_load_program("handle_sched_process_exec")
            .expect("mapped program")
    );
    assert!(
        !plan
            .should_load_program("handle_signal_generate")
            .expect("mapped program")
    );
    assert!(
        !plan
            .should_load_program("handle_sys_enter_openat")
            .expect("mapped program")
    );
    assert!(
        !plan
            .should_load_program("handle_sys_enter_mmap")
            .expect("mapped program")
    );
}

#[test]
fn baseline_has_no_implicit_proc_lifecycle() {
    let plan = AttachPlan::baseline();

    assert!(
        !plan
            .should_load_program("handle_sched_process_exec")
            .expect("mapped program")
    );
    assert!(!plan.contains(&Capability::ProcLifecycle));
}

#[test]
fn net_transport_loads_fd_io_but_not_file_path_programs() {
    let config = config();
    let payload = payload_config();
    let plan = AttachPlan::from_requests(
        &[
            CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
            CapabilityRequest::new(Capability::NetTransport, RequestMode::Required),
        ],
        &config,
        &payload,
    );

    assert!(
        plan.should_load_program("handle_sys_enter_write")
            .expect("mapped program")
    );
    assert!(
        !plan
            .should_load_program("handle_sys_enter_openat")
            .expect("mapped program")
    );
}

#[test]
fn fs_access_basic_can_skip_file_path_programs() {
    let mut config = config();
    let payload = payload_config();
    config.file_path_capture_enabled = false;
    let plan = AttachPlan::from_requests(
        &[
            CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
            CapabilityRequest::new(Capability::FsAccessBasic, RequestMode::Required),
        ],
        &config,
        &payload,
    );

    assert!(
        plan.should_load_program("handle_sys_enter_write")
            .expect("mapped program")
    );
    assert!(
        !plan
            .should_load_program("handle_sys_enter_openat")
            .expect("mapped program")
    );

    let attached = plan.attached_capabilities(&[
        "handle_sys_enter_read".to_string(),
        "handle_sys_exit_read".to_string(),
        "handle_sys_enter_write".to_string(),
        "handle_sys_exit_write".to_string(),
    ]);
    assert!(attached.contains(&Capability::FsAccessBasic));
}

#[test]
fn fs_access_basic_loads_file_path_programs_when_enabled() {
    let mut config = config();
    let payload = payload_config();
    config.file_path_capture_enabled = true;
    let plan = AttachPlan::from_requests(
        &[
            CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
            CapabilityRequest::new(Capability::FsAccessBasic, RequestMode::Required),
        ],
        &config,
        &payload,
    );

    assert!(
        plan.should_load_program("handle_sys_enter_openat")
            .expect("mapped program")
    );
    assert!(
        plan.should_load_program("handle_sys_enter_openat2")
            .expect("mapped program")
    );
    assert!(
        plan.should_load_program("handle_sys_enter_close")
            .expect("mapped program")
    );
    assert!(
        plan.should_load_program("handle_sched_process_exec")
            .expect("mapped program")
    );

    let attached = plan.attached_capabilities(&[
        "handle_sched_process_fork".to_string(),
        "handle_sched_process_exec".to_string(),
        "handle_sched_process_exit".to_string(),
        "handle_sys_enter_read".to_string(),
        "handle_sys_exit_read".to_string(),
        "handle_sys_enter_write".to_string(),
        "handle_sys_exit_write".to_string(),
    ]);
    assert!(!attached.contains(&Capability::FsAccessBasic));
    assert!(!attached.contains(&Capability::ProcLifecycle));
}

#[test]
fn fs_access_basic_context_does_not_grant_proc_lifecycle() {
    let mut config = config();
    let payload = payload_config();
    config.file_path_capture_enabled = true;
    let plan = AttachPlan::from_requests(
        &[CapabilityRequest::new(
            Capability::FsAccessBasic,
            RequestMode::Required,
        )],
        &config,
        &payload,
    );
    let attached = plan.attached_capabilities(&[
        "handle_sched_process_fork".to_string(),
        "handle_sched_process_exec".to_string(),
        "handle_sched_process_exit".to_string(),
        "handle_sys_enter_read".to_string(),
        "handle_sys_exit_read".to_string(),
        "handle_sys_enter_write".to_string(),
        "handle_sys_exit_write".to_string(),
        "handle_sys_enter_open".to_string(),
        "handle_sys_exit_open".to_string(),
        "handle_sys_enter_openat".to_string(),
        "handle_sys_exit_openat".to_string(),
        "handle_sys_enter_creat".to_string(),
        "handle_sys_exit_creat".to_string(),
        "handle_sys_enter_unlinkat".to_string(),
        "handle_sys_exit_unlinkat".to_string(),
        "handle_sys_enter_renameat".to_string(),
        "handle_sys_exit_renameat".to_string(),
        "handle_sys_enter_mkdirat".to_string(),
        "handle_sys_exit_mkdirat".to_string(),
        "handle_sys_enter_close".to_string(),
        "handle_sys_exit_close".to_string(),
        "handle_sys_enter_dup".to_string(),
        "handle_sys_exit_dup".to_string(),
        "handle_sys_enter_dup2".to_string(),
        "handle_sys_exit_dup2".to_string(),
        "handle_sys_enter_dup3".to_string(),
        "handle_sys_exit_dup3".to_string(),
        "handle_sys_enter_fcntl".to_string(),
        "handle_sys_exit_fcntl".to_string(),
        "handle_sys_enter_chdir".to_string(),
        "handle_sys_exit_chdir".to_string(),
        "handle_sys_enter_fchdir".to_string(),
        "handle_sys_exit_fchdir".to_string(),
    ]);

    assert!(attached.contains(&Capability::FsAccessBasic));
    assert!(!attached.contains(&Capability::ProcLifecycle));
}

#[test]
fn fs_mmap_loads_mmap_without_file_path_programs() {
    let config = config();
    let payload = payload_config();
    let plan = AttachPlan::from_requests(
        &[
            CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
            CapabilityRequest::new(Capability::FsMmap, RequestMode::Required),
        ],
        &config,
        &payload,
    );

    assert!(
        plan.should_load_program("handle_sys_enter_mmap")
            .expect("mapped program")
    );
    assert!(
        !plan
            .should_load_program("handle_sys_enter_openat")
            .expect("mapped program")
    );
    assert!(
        plan.should_load_program("handle_sched_process_exec")
            .expect("mapped program")
    );
}

#[test]
fn stdio_requires_enabled_payload_config() {
    let config = config();
    let mut disabled = payload_config();
    disabled.stdio.enabled = false;
    let disabled_plan = AttachPlan::from_requests(
        &[CapabilityRequest::new(
            Capability::StdioChunk,
            RequestMode::Required,
        )],
        &config,
        &disabled,
    );
    assert!(!disabled_plan.contains(&Capability::StdioChunk));

    let mut enabled = payload_config();
    enabled.stdio.enabled = true;
    let enabled_plan = AttachPlan::from_requests(
        &[CapabilityRequest::new(
            Capability::StdioChunk,
            RequestMode::Required,
        )],
        &config,
        &enabled,
    );
    assert!(enabled_plan.contains(&Capability::StdioChunk));
}

#[test]
fn socket_payload_requires_enabled_payload_config() {
    let config = config();
    let mut disabled = payload_config();
    disabled.socket.enabled = false;
    let disabled_plan = AttachPlan::from_requests(
        &[CapabilityRequest::new(
            Capability::SocketPlaintextPayload,
            RequestMode::Required,
        )],
        &config,
        &disabled,
    );
    assert!(!disabled_plan.contains(&Capability::SocketPlaintextPayload));

    let mut enabled = payload_config();
    enabled.socket.enabled = true;
    let enabled_plan = AttachPlan::from_requests(
        &[CapabilityRequest::new(
            Capability::SocketPlaintextPayload,
            RequestMode::Required,
        )],
        &config,
        &enabled,
    );
    assert!(enabled_plan.contains(&Capability::SocketPlaintextPayload));
    assert!(
        enabled_plan
            .should_load_program("handle_sys_enter_sendto")
            .expect("mapped program")
    );
    assert!(
        enabled_plan
            .should_load_program("handle_sys_enter_write")
            .expect("mapped program")
    );
    assert!(
        enabled_plan
            .should_load_program("handle_sys_enter_writev")
            .expect("mapped program")
    );
    assert!(
        enabled_plan
            .should_load_program("handle_sys_enter_sendmsg")
            .expect("mapped program")
    );
}

#[test]
fn shared_fd_programs_do_not_grant_unrequested_capabilities() {
    let config = config();
    let mut payload = payload_config();
    payload.stdio.enabled = true;
    let plan = AttachPlan::from_requests(
        &[CapabilityRequest::new(
            Capability::StdioChunk,
            RequestMode::Required,
        )],
        &config,
        &payload,
    );
    let attached = plan.attached_capabilities(&[
        "handle_sys_enter_read".to_string(),
        "handle_sys_exit_read".to_string(),
        "handle_sys_enter_write".to_string(),
        "handle_sys_exit_write".to_string(),
    ]);

    assert!(attached.contains(&Capability::StdioChunk));
    assert!(!attached.contains(&Capability::NetTransport));
    assert!(!attached.contains(&Capability::IpcPipeFifo));
    assert!(!attached.contains(&Capability::IpcUnixSocket));
    assert!(!attached.contains(&Capability::FsAccessBasic));
}

#[test]
fn plan_requires_all_planned_capabilities_to_be_satisfied() {
    let config = config();
    let payload = payload_config();
    let plan = AttachPlan::from_requests(
        &[
            CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
            CapabilityRequest::new(Capability::FsMmap, RequestMode::Required),
        ],
        &config,
        &payload,
    );
    let attached = BTreeSet::from([Capability::ProcLifecycle]);

    assert!(!plan.is_satisfied_by(&attached));
}

#[test]
fn file_path_programs_attach_before_process_programs() {
    let mut config = config();
    let payload = payload_config();
    config.file_path_capture_enabled = true;
    let plan = AttachPlan::from_requests(
        &[
            CapabilityRequest::new(Capability::ProcLifecycle, RequestMode::Required),
            CapabilityRequest::new(Capability::FsAccessBasic, RequestMode::Required),
        ],
        &config,
        &payload,
    );

    assert!(
        plan.attach_priority("handle_sys_enter_openat")
            < plan.attach_priority("handle_sched_process_exec")
    );
}

fn config() -> EbpfCollectorConfig {
    OperatorConfig::parse(OPERATOR_CONFIG_TEMPLATE)
        .expect("operator config template parses")
        .ebpf_config
}

fn payload_config() -> PayloadConfig {
    OperatorConfig::parse(OPERATOR_CONFIG_TEMPLATE)
        .expect("operator config template parses")
        .payload_config
}
