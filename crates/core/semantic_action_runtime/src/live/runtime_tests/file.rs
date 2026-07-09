use std::time::{Duration, SystemTime};

use config_core::daemon::{AgentInvocationConfig, FileObservationConfig, SemanticRetentionConfig};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::EventId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    SemanticActionCompleteness, SemanticActionKind, SemanticActionLinkRole, SemanticActionStatus,
};

use super::LiveSemanticActionRuntime;
use super::test_support::*;

#[path = "file/bulk_read.rs"]
mod bulk_read_tests;

#[path = "file/enumerate.rs"]
mod enumerate_tests;

#[test]
fn startup_file_read_is_projected_and_linked_when_agent_is_observed_later() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    assert!(
        runtime
            .observe_event(&file_event(
                FILE_OPEN_EVENT_ID,
                agent.clone(),
                "open",
                TEST_FILE_FD as i32,
                None,
            ))
            .actions
            .is_empty()
    );
    let read_output = runtime.observe_event(&file_event(
        FILE_READ_EVENT_ID,
        agent.clone(),
        "read",
        TEST_FILE_READ_BYTES as i32,
        Some(TEST_FILE_READ_BYTES),
    ));
    assert!(read_output.raw_event_consumed);
    assert!(
        read_output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileRead)
    );
    assert!(
        read_output
            .links
            .iter()
            .all(|link| link.role != SemanticActionLinkRole::AgentPerformedAction)
    );

    let close_output = runtime.observe_event(&file_event(
        FILE_CLOSE_EVENT_ID,
        agent.clone(),
        "close",
        0,
        None,
    ));
    assert!(close_output.raw_event_consumed);
    assert!(
        close_output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileRead)
    );

    let agent_update = runtime.observe_payload_segment(&llm_payload_segment(agent));
    let complete_read = agent_update
        .actions
        .iter()
        .find(|action| {
            action.kind == SemanticActionKind::FileRead
                && action.completeness == SemanticActionCompleteness::Complete
        })
        .expect("payload boundary should release the detailed file.read action");
    assert_eq!(complete_read.status, SemanticActionStatus::Success);
    assert_eq!(
        complete_read.completeness,
        SemanticActionCompleteness::Complete
    );
    assert!(complete_read.evidence.iter().any(|evidence| {
        evidence.id == FILE_OPEN_EVENT_ID.get() && evidence.role == "file.open"
    }));
    assert!(complete_read.evidence.iter().any(|evidence| {
        evidence.id == FILE_READ_EVENT_ID.get() && evidence.role == "file.read"
    }));
    assert!(complete_read.evidence.iter().any(|evidence| {
        evidence.id == FILE_CLOSE_EVENT_ID.get() && evidence.role == "file.close"
    }));
    let llm_call = agent_update
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("LLM payload should project a call action");
    let file_read_link = agent_update
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == complete_read.action_id
        })
        .expect("previous startup file.read should link under the observed agent");
    let llm_call_link = agent_update
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::AgentPerformedAction
                && link.child_action_id == llm_call.action_id
        })
        .expect("current LLM call should link under the observed agent");

    assert_eq!(
        file_read_link
            .attributes
            .get(AGENT_ACTION_SEQUENCE_ATTR)
            .map(String::as_str),
        Some(FIRST_AGENT_ACTION_SEQUENCE)
    );
    assert_eq!(
        llm_call_link
            .attributes
            .get(AGENT_ACTION_SEQUENCE_ATTR)
            .map(String::as_str),
        Some(SECOND_AGENT_ACTION_SEQUENCE)
    );
}

#[test]
fn command_process_file_read_links_under_command_invocation() {
    let mut runtime = runtime();
    let agent = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let command_process =
        ProcessIdentity::new(WRAPPER_PID, WRAPPER_START_TICKS, WRAPPER_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        agent.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));
    runtime.observe_payload_segment(&llm_payload_segment(agent.clone()));
    runtime.observe_event(&fork_event(
        AGENT_FORK_EVENT_ID,
        command_process.clone(),
        agent,
    ));
    let command_output = runtime.observe_event(&exec_event(
        WRAPPER_EXEC_EVENT_ID,
        command_process.clone(),
        None,
        "/usr/bin/bash",
    ));
    let command = command_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::CommandInvocation)
        .expect("child exec should project command.invocation");

    runtime.observe_event(&file_event(
        FILE_OPEN_EVENT_ID,
        command_process.clone(),
        "open",
        TEST_FILE_FD as i32,
        None,
    ));
    let read_output = runtime.observe_event(&file_event(
        FILE_READ_EVENT_ID,
        command_process.clone(),
        "read",
        TEST_FILE_READ_BYTES as i32,
        Some(TEST_FILE_READ_BYTES),
    ));
    assert!(read_output.raw_event_consumed);
    assert!(
        read_output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileRead)
    );
    runtime.observe_event(&file_event(
        FILE_CLOSE_EVENT_ID,
        command_process.clone(),
        "close",
        0,
        None,
    ));
    let exit_output = runtime.observe_event(&exit_event(ROOT_EXIT_EVENT_ID, command_process, 0));
    let file_read = exit_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileRead)
        .expect("process exit boundary should release file.read");
    let link = exit_output
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsFileAccess
                && link.child_action_id == file_read.action_id
        })
        .expect("file.read should link under the same-process command invocation");

    assert_eq!(link.parent_action_id, command.action_id);
}

#[test]
fn tty_write_is_consumed_by_summary_without_file_modify_duplication() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let event = tty_write_event(FILE_READ_EVENT_ID, process, SystemTime::UNIX_EPOCH);

    let output = runtime.observe_event(&event);

    assert!(output.raw_event_consumed);
    assert!(!output.retain_event);
    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileModify)
    );
    let tty = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileTtyIo)
        .expect("tty write should project a file.tty_io summary");
    assert!(tty.evidence.is_empty());
    assert_eq!(
        tty.attributes.get("file.bytes_written").cloned(),
        Some(TEST_FILE_READ_BYTES.to_string())
    );
}

#[test]
fn tty_summary_is_throttled_between_flush_intervals() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    let first = runtime.observe_event(&tty_write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 100),
        process.clone(),
        SystemTime::UNIX_EPOCH,
    ));
    assert!(first.raw_event_consumed);
    assert!(
        first
            .actions
            .iter()
            .any(|action| action.kind == SemanticActionKind::FileTtyIo)
    );

    let second = runtime.observe_event(&tty_write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 101),
        process.clone(),
        SystemTime::UNIX_EPOCH + Duration::from_millis(4999),
    ));
    assert!(second.raw_event_consumed);
    assert!(!second.retain_event);
    assert!(
        second
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileTtyIo)
    );

    let third = runtime.observe_event(&tty_write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 102),
        process,
        SystemTime::UNIX_EPOCH + Duration::from_millis(5000),
    ));
    let tty = third
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileTtyIo)
        .expect("tty summary should flush after the configured interval");
    assert_eq!(tty.completeness, SemanticActionCompleteness::Partial);
    assert_eq!(
        tty.attributes
            .get("file.tty.event_count")
            .map(String::as_str),
        Some("3")
    );
    let expected_bytes_written = (TEST_FILE_READ_BYTES * 3).to_string();
    assert_eq!(
        tty.attributes.get("file.bytes_written").map(String::as_str),
        Some(expected_bytes_written.as_str())
    );

    let finalized = runtime.finalize_trace(
        TRACE_ID,
        SystemTime::UNIX_EPOCH + Duration::from_millis(6000),
    );
    let complete = finalized
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileTtyIo)
        .expect("trace finalization should complete the tty summary");
    assert_eq!(complete.completeness, SemanticActionCompleteness::Complete);
    assert_eq!(
        complete
            .attributes
            .get("file.tty.event_count")
            .map(String::as_str),
        Some("3")
    );
}

#[test]
fn tty_truncate_is_consumed_by_summary_without_file_modify_duplication() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let event = tty_file_event(
        FILE_READ_EVENT_ID,
        process,
        "truncate",
        4,
        None,
        SystemTime::UNIX_EPOCH,
    );

    let output = runtime.observe_event(&event);

    assert!(output.raw_event_consumed);
    assert!(!output.retain_event);
    assert!(
        output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileModify)
    );
    let tty = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileTtyIo)
        .expect("tty truncate should project only a file.tty_io summary");
    assert_eq!(
        tty.attributes
            .get("file.tty.event_count")
            .map(String::as_str),
        Some("1")
    );
}

#[test]
fn tty_unlisted_operation_is_consumed_without_direct_file_action() {
    let mut file_observation = FileObservationConfig::default();
    file_observation.tty.operations = vec!["write".to_string()];
    let mut runtime = LiveSemanticActionRuntime::new(
        AgentInvocationConfig {
            enabled: true,
            commands: vec!["xiaoo".to_string()],
        },
        SemanticRetentionConfig::default(),
        file_observation,
    );
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let event = tty_file_event(
        FILE_READ_EVENT_ID,
        process,
        "truncate",
        4,
        None,
        SystemTime::UNIX_EPOCH,
    );

    let output = runtime.observe_event(&event);

    assert!(output.raw_event_consumed);
    assert!(!output.retain_event);
    assert!(output.actions.is_empty());
}

#[test]
fn tty_error_event_is_summary_only_without_raw_retention() {
    let mut runtime = runtime();
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);
    let event = tty_file_event(
        FILE_READ_EVENT_ID,
        process,
        "open",
        -6,
        None,
        SystemTime::UNIX_EPOCH,
    );

    let output = runtime.observe_event(&event);

    assert!(output.raw_event_consumed);
    assert!(!output.retain_event);
    let tty = output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileTtyIo)
        .expect("tty error should still update the summary action");
    assert_eq!(tty.status, SemanticActionStatus::Error);
    assert_eq!(
        tty.attributes
            .get("file.tty.error_count")
            .map(String::as_str),
        Some("1")
    );
}

fn tty_write_event(
    event_id: EventId,
    process: ProcessIdentity,
    observed_at: SystemTime,
) -> DomainEvent {
    tty_file_event(
        event_id,
        process,
        "write",
        TEST_FILE_READ_BYTES as i32,
        Some(TEST_FILE_READ_BYTES),
        observed_at,
    )
}

fn tty_file_event(
    event_id: EventId,
    process: ProcessIdentity,
    operation: &str,
    result: i32,
    size: Option<u64>,
    observed_at: SystemTime,
) -> DomainEvent {
    let mut event = file_event(event_id, process, operation, result, size);
    event.envelope.observed_at = observed_at;
    let EventPayload::File(payload) = &mut event.payload else {
        unreachable!("file_event returns a file payload");
    };
    payload.path = Some("/dev/tty".to_string());
    payload
        .metadata
        .insert("fd_target".to_string(), "/dev/tty".to_string());
    event
}
