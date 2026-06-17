use config_core::daemon::{
    AgentInvocationConfig, FileBulkReadMode, FileObservationConfig, FileRawEventRetention,
    SemanticRetentionConfig,
};
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::EventId;
use model_core::process::ProcessIdentity;
use semantic_action::{
    FilePathSetState, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    SemanticActionLinkRole, attr_keys as attrs,
};

use super::super::test_support::*;
use super::super::{LiveSemanticActionOutput, LiveSemanticActionRuntime};

const ENUMERATE_MIN_UNIQUE_PATHS: u32 = 2;
const ENUMERATE_MIN_UNIQUE_PATHS_TEXT: &str = "2";
const ENUMERATE_MAX_PATHS_PER_SET: u32 = 4;
const BULK_MIN_UNIQUE_PATHS: u32 = 2;
const DIR_FD: u32 = 23;
const DIR_A: &str = "/root/projects/AcTrail/crates";
const DIR_B: &str = "/root/projects/AcTrail/crates/core";
const REGULAR_PATH: &str = "/root/projects/AcTrail/Cargo.toml";
const READ_PATH_A: &str = "/root/projects/AcTrail/AGENTS.md";
const READ_PATH_B: &str = "/root/projects/AcTrail/README.md";
const WRITE_PATH: &str = "/root/projects/AcTrail/tmp/action-sequence/temp_note.md";

#[test]
fn directory_enumeration_projects_fs_enumerate_and_path_set() {
    let mut runtime = enumerate_runtime(true);
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

    let first_open = runtime.observe_event(&directory_open_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 100),
        command_process.clone(),
        DIR_A,
    ));
    assert!(first_open.actions.is_empty());
    assert!(!first_open.raw_event_consumed);
    assert!(first_open.retain_event);

    let first_close = runtime.observe_event(&directory_close_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 101),
        command_process.clone(),
    ));
    assert!(first_close.actions.is_empty());
    assert!(!first_close.raw_event_consumed);

    let second_open = runtime.observe_event(&directory_open_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 102),
        command_process.clone(),
        DIR_B,
    ));
    assert!(second_open.raw_event_consumed);
    assert!(!second_open.retain_event);
    let enumerate = second_open
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FsEnumerate)
        .expect("second directory open should activate fs.enumerate");
    let enumerate_action_id = enumerate.action_id.clone();
    assert_eq!(enumerate.completeness, SemanticActionCompleteness::Partial);
    assert_eq!(
        enumerate
            .attributes
            .get(attrs::fs_enumerate::UNIQUE_PATH_COUNT)
            .map(String::as_str),
        Some(ENUMERATE_MIN_UNIQUE_PATHS_TEXT)
    );
    assert_eq!(
        enumerate
            .attributes
            .get(attrs::fs_enumerate::PATH_SET_STATE)
            .map(String::as_str),
        Some(FilePathSetState::Pending.as_str())
    );
    let command_link = second_open
        .links
        .iter()
        .find(|link| {
            link.role == SemanticActionLinkRole::CommandContainsFileAccess
                && link.child_action_id == enumerate.action_id
        })
        .expect("fs.enumerate should link under the same-process command invocation");
    assert_eq!(command_link.parent_action_id, command.action_id);

    let second_close = runtime.observe_event(&directory_close_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 103),
        command_process.clone(),
    ));
    assert!(second_close.raw_event_consumed);
    assert!(second_close.actions.is_empty());

    let write_output = runtime.observe_event(&write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 104),
        command_process,
        WRITE_PATH,
    ));
    let completed = completed_enumerate_action(&write_output, &enumerate_action_id)
        .expect("real file write should complete the current fs.enumerate");
    assert_eq!(completed.completeness, SemanticActionCompleteness::Complete);
    assert_eq!(
        completed
            .attributes
            .get(attrs::fs_enumerate::CLOSE_COUNT)
            .map(String::as_str),
        Some("2")
    );
    let enumerate_index = write_output
        .actions
        .iter()
        .position(|action| action.action_id == completed.action_id)
        .expect("completed fs.enumerate should be in the write output");
    let file_modify_index = write_output
        .actions
        .iter()
        .position(|action| action.kind == SemanticActionKind::FileModify)
        .expect("write should still project file.modify");
    assert!(enumerate_index < file_modify_index);
    assert_eq!(write_output.file_path_sets.len(), 1);
    let path_set = &write_output.file_path_sets[0];
    assert_eq!(path_set.action_id, enumerate_action_id);
    assert_eq!(path_set.state, FilePathSetState::Complete);
    assert_eq!(path_set.paths, expected_paths(&[DIR_A, DIR_B]));
}

#[test]
fn directory_enumeration_completes_active_bulk_read_before_enumerate() {
    let mut runtime = enumerate_runtime(false);
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 200),
        process.clone(),
        READ_PATH_A,
    ));
    let bulk_output = runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 201),
        process.clone(),
        READ_PATH_B,
    ));
    let bulk = bulk_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileBulkRead)
        .expect("second file read should activate file.bulk_read");
    let bulk_action_id = bulk.action_id.clone();

    let first_open = runtime.observe_event(&directory_open_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 202),
        process.clone(),
        DIR_A,
    ));
    let completed_bulk = completed_bulk_action(&first_open, &bulk_action_id)
        .expect("directory enumeration should complete an active file.bulk_read");
    assert_eq!(
        completed_bulk.completeness,
        SemanticActionCompleteness::Complete
    );
    assert!(
        first_open
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FsEnumerate)
    );
    assert_eq!(first_open.file_path_sets.len(), 1);
    assert_eq!(first_open.file_path_sets[0].action_id, bulk_action_id);
    assert_eq!(
        first_open.file_path_sets[0].paths,
        expected_paths(&[READ_PATH_A, READ_PATH_B])
    );

    let second_open = runtime.observe_event(&directory_open_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 203),
        process,
        DIR_B,
    ));
    assert!(
        second_open
            .actions
            .iter()
            .any(|action| action.kind == SemanticActionKind::FsEnumerate)
    );
}

#[test]
fn regular_file_open_does_not_project_fs_enumerate() {
    let mut runtime = enumerate_runtime(false);
    let process = ProcessIdentity::new(AGENT_PID, AGENT_START_TICKS, AGENT_GENERATION);

    for offset in 0..ENUMERATE_MIN_UNIQUE_PATHS {
        let output = runtime.observe_event(&regular_open_event(
            EventId::new(FILE_READ_EVENT_ID.get() + 300 + u64::from(offset)),
            process.clone(),
            REGULAR_PATH,
        ));
        assert!(!output.raw_event_consumed);
        assert!(
            output
                .actions
                .iter()
                .all(|action| action.kind != SemanticActionKind::FsEnumerate)
        );
    }
}

fn enumerate_runtime(agent_enabled: bool) -> LiveSemanticActionRuntime {
    let mut file_observation = FileObservationConfig::default();
    file_observation.bulk_read.mode = FileBulkReadMode::PathSet;
    file_observation.bulk_read.raw_event_retention = FileRawEventRetention::Summary;
    file_observation.bulk_read.min_unique_paths = BULK_MIN_UNIQUE_PATHS;
    file_observation.enumerate.raw_event_retention = FileRawEventRetention::Summary;
    file_observation.enumerate.min_unique_paths = ENUMERATE_MIN_UNIQUE_PATHS;
    file_observation.enumerate.max_paths_per_set = ENUMERATE_MAX_PATHS_PER_SET;
    LiveSemanticActionRuntime::new(
        AgentInvocationConfig {
            enabled: agent_enabled,
            commands: if agent_enabled {
                vec!["xiaoo".to_string()]
            } else {
                Vec::new()
            },
        },
        SemanticRetentionConfig::default(),
        file_observation,
    )
}

fn directory_open_event(event_id: EventId, process: ProcessIdentity, path: &str) -> DomainEvent {
    let mut event = regular_open_event(event_id, process, path);
    let EventPayload::File(payload) = &mut event.payload else {
        unreachable!("file_event returns a file payload");
    };
    payload
        .metadata
        .insert("flags".to_string(), (libc::O_DIRECTORY as u64).to_string());
    event
}

fn regular_open_event(event_id: EventId, process: ProcessIdentity, path: &str) -> DomainEvent {
    let mut event = file_event(event_id, process, "open", DIR_FD as i32, None);
    set_file_path(&mut event, path);
    let EventPayload::File(payload) = &mut event.payload else {
        unreachable!("file_event returns a file payload");
    };
    payload
        .metadata
        .insert("fd".to_string(), DIR_FD.to_string());
    payload
        .metadata
        .insert("flags".to_string(), "0".to_string());
    payload
        .metadata
        .insert("syscall".to_string(), "openat".to_string());
    event
}

fn directory_close_event(event_id: EventId, process: ProcessIdentity) -> DomainEvent {
    let mut event = file_event(event_id, process, "close", 0, None);
    let EventPayload::File(payload) = &mut event.payload else {
        unreachable!("file_event returns a file payload");
    };
    payload.path = None;
    payload
        .metadata
        .insert("fd".to_string(), DIR_FD.to_string());
    payload.metadata.remove("fd_target");
    event
}

fn read_event(event_id: EventId, process: ProcessIdentity, path: &str) -> DomainEvent {
    file_access_event(event_id, process, "read", path)
}

fn write_event(event_id: EventId, process: ProcessIdentity, path: &str) -> DomainEvent {
    file_access_event(event_id, process, "write", path)
}

fn file_access_event(
    event_id: EventId,
    process: ProcessIdentity,
    operation: &str,
    path: &str,
) -> DomainEvent {
    let mut event = file_event(
        event_id,
        process,
        operation,
        TEST_FILE_READ_BYTES as i32,
        Some(TEST_FILE_READ_BYTES),
    );
    set_file_path(&mut event, path);
    event
}

fn set_file_path(event: &mut DomainEvent, path: &str) {
    let EventPayload::File(payload) = &mut event.payload else {
        unreachable!("file_event returns a file payload");
    };
    payload.path = Some(path.to_string());
    payload
        .metadata
        .insert("fd_target".to_string(), path.to_string());
}

fn completed_enumerate_action<'a>(
    output: &'a LiveSemanticActionOutput,
    action_id: &str,
) -> Option<&'a SemanticAction> {
    output.actions.iter().find(|action| {
        action.kind == SemanticActionKind::FsEnumerate && action.action_id == action_id
    })
}

fn completed_bulk_action<'a>(
    output: &'a LiveSemanticActionOutput,
    action_id: &str,
) -> Option<&'a SemanticAction> {
    output.actions.iter().find(|action| {
        action.kind == SemanticActionKind::FileBulkRead && action.action_id == action_id
    })
}

fn expected_paths(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_string()).collect()
}
