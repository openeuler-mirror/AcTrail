use config_core::daemon::{
    AgentInvocationConfig, FileBulkReadMode, FileObservationConfig, FileRawEventRetention,
    SemanticRetentionConfig,
};
use model_core::event::{
    DomainEvent, EventEnvelope, EventFlags, EventKind, EventPayload, ResourcePayload, StdioPayload,
};
use model_core::ids::{CollectorName, EventId};
use model_core::payload::{PayloadSegment, PayloadSourceBoundary, PayloadStreamKey};
use model_core::process::ProcessIdentity;
use semantic_action::{
    FilePathSetState, SemanticAction, SemanticActionCompleteness, SemanticActionKind,
    attr_keys as attrs,
};
use std::time::SystemTime;

use super::super::test_support::*;
use super::super::{LiveSemanticActionOutput, LiveSemanticActionRuntime};

const BULK_MIN_UNIQUE_PATHS: u32 = 2;
const BULK_MAX_PATHS_PER_SET: u32 = 4;
const PATH_A: &str = "/tmp/a";
const PATH_B: &str = "/tmp/b";
const PATH_C: &str = "/tmp/c";
const PATH_D: &str = "/tmp/d";
const WRITE_PATH: &str = "/tmp/out";
const TTY_PATH: &str = "/dev/tty";
const STDIO_BYTES: &[u8] = b"stdio noise";
const STALE_FD_BULK_MIN_UNIQUE_PATHS: u32 = 1;

#[test]
fn bulk_read_threshold_emits_summary_and_writes_path_set_on_boundary() {
    let mut runtime = bulk_runtime(false);
    let process = ProcessIdentity::new(AGENT_GENERATION);

    let first_output =
        runtime.observe_event(&read_event(FILE_READ_EVENT_ID, process.clone(), PATH_A));
    assert!(first_output.raw_event_consumed);
    assert!(!first_output.retain_event);
    assert!(
        first_output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileRead)
    );

    let second_output = runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 1),
        process.clone(),
        PATH_B,
    ));

    assert!(second_output.raw_event_consumed);
    assert!(!second_output.retain_event);
    assert!(second_output.file_observation_paths.is_empty());
    assert!(second_output.file_path_sets.is_empty());
    assert_eq!(
        second_output
            .actions
            .iter()
            .filter(|action| action.kind == SemanticActionKind::FileRead)
            .count(),
        0
    );
    let bulk = second_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileBulkRead)
        .expect("threshold event should project a file.bulk_read summary");
    let bulk_action_id = bulk.action_id.clone();
    assert!(bulk.evidence.is_empty());
    assert_eq!(
        bulk.attributes
            .get(attrs::file_bulk_read::UNIQUE_PATH_COUNT)
            .and_then(|value| value.parse::<u32>().ok()),
        Some(BULK_MIN_UNIQUE_PATHS)
    );
    assert_eq!(
        bulk.attributes
            .get(attrs::file_bulk_read::PATH_SET_STATE)
            .map(String::as_str),
        Some(FilePathSetState::Pending.as_str())
    );

    let write_output = runtime.observe_event(&write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 2),
        process,
        WRITE_PATH,
    ));

    assert_eq!(write_output.file_path_sets.len(), 1);
    let path_set = &write_output.file_path_sets[0];
    assert_eq!(path_set.action_id, bulk_action_id);
    assert_eq!(path_set.state, FilePathSetState::Complete);
    assert_eq!(path_set.paths, expected_paths(&[PATH_A, PATH_B]));
}

#[test]
fn short_read_burst_releases_detailed_reads_on_boundary() {
    let mut runtime = bulk_runtime_with_min_unique_paths(false, 3);
    let process = ProcessIdentity::new(AGENT_GENERATION);

    let first_output =
        runtime.observe_event(&read_event(FILE_READ_EVENT_ID, process.clone(), PATH_A));
    assert!(first_output.raw_event_consumed);
    assert!(!first_output.retain_event);
    assert!(first_output.actions.is_empty());

    let second_output = runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 1),
        process.clone(),
        PATH_B,
    ));
    assert!(second_output.raw_event_consumed);
    assert!(!second_output.retain_event);
    assert!(second_output.actions.is_empty());

    let boundary_output = runtime.observe_event(&write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 2),
        process,
        WRITE_PATH,
    ));
    assert!(boundary_output.file_path_sets.is_empty());
    assert!(
        boundary_output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileBulkRead)
    );
    assert_eq!(
        boundary_output
            .actions
            .iter()
            .filter(|action| action.kind == SemanticActionKind::FileRead)
            .count(),
        2
    );
    assert_eq!(boundary_output.deferred_events.len(), 0);
}

#[test]
fn short_read_burst_retains_deferred_events_when_configured_full() {
    let mut runtime = bulk_runtime_with_retention(false, 3, FileRawEventRetention::Full);
    let process = ProcessIdentity::new(AGENT_GENERATION);

    runtime.observe_event(&read_event(FILE_READ_EVENT_ID, process.clone(), PATH_A));
    runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 1),
        process.clone(),
        PATH_B,
    ));

    let boundary_output = runtime.observe_event(&write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 2),
        process,
        WRITE_PATH,
    ));

    assert_eq!(
        boundary_output
            .actions
            .iter()
            .filter(|action| action.kind == SemanticActionKind::FileRead)
            .count(),
        2
    );
    assert_eq!(boundary_output.deferred_events.len(), 2);
}

#[test]
fn bulk_read_boundary_completes_current_burst_before_next_action() {
    let mut runtime = bulk_runtime(true);
    let process = ProcessIdentity::new(AGENT_GENERATION);

    runtime.observe_event(&exec_event(
        AGENT_EXEC_EVENT_ID,
        process.clone(),
        None,
        "/root/.cargo/bin/xiaoo",
    ));

    runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 10),
        process.clone(),
        PATH_A,
    ));
    let first_bulk_output = runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 11),
        process.clone(),
        PATH_B,
    ));
    let first_bulk = first_bulk_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileBulkRead)
        .expect("threshold event should start a bulk read action");
    let first_bulk_action_id = first_bulk.action_id.clone();

    let write_output = runtime.observe_event(&write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 12),
        process.clone(),
        WRITE_PATH,
    ));
    let completed_first = completed_bulk_action(&write_output, &first_bulk_action_id)
        .expect("non-read file operation should complete the current bulk read");
    assert_eq!(
        completed_first.completeness,
        SemanticActionCompleteness::Complete
    );
    assert_eq!(write_output.file_path_sets.len(), 1);
    assert_eq!(
        write_output.file_path_sets[0].action_id,
        first_bulk_action_id
    );
    assert_eq!(
        write_output.file_path_sets[0].paths,
        expected_paths(&[PATH_A, PATH_B])
    );
    let completed_first_index = write_output
        .actions
        .iter()
        .position(|action| action.action_id == completed_first.action_id)
        .expect("completed bulk read should be in the write output");
    let file_modify_index = write_output
        .actions
        .iter()
        .position(|action| action.kind == SemanticActionKind::FileModify)
        .expect("write should still project its own file.modify action");
    assert!(completed_first_index < file_modify_index);

    runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 13),
        process.clone(),
        PATH_C,
    ));
    let second_bulk_output = runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 14),
        process.clone(),
        PATH_D,
    ));
    let second_bulk = second_bulk_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileBulkRead)
        .expect("next read burst should start a separate bulk read action");
    assert_ne!(second_bulk.action_id, first_bulk_action_id);
    let second_bulk_action_id = second_bulk.action_id.clone();

    let payload_output = runtime.observe_payload_segment(&llm_payload_segment(process));
    let completed_second = completed_bulk_action(&payload_output, &second_bulk_action_id)
        .expect("payload semantic action should complete the current bulk read");
    assert_eq!(
        completed_second.completeness,
        SemanticActionCompleteness::Complete
    );
    assert_eq!(payload_output.file_path_sets.len(), 1);
    assert_eq!(
        payload_output.file_path_sets[0].action_id,
        second_bulk_action_id
    );
    assert_eq!(
        payload_output.file_path_sets[0].paths,
        expected_paths(&[PATH_C, PATH_D])
    );
    let completed_second_index = payload_output
        .actions
        .iter()
        .position(|action| action.action_id == completed_second.action_id)
        .expect("completed bulk read should be in the payload output");
    let llm_call_index = payload_output
        .actions
        .iter()
        .position(|action| action.kind == SemanticActionKind::LlmCall)
        .expect("payload should still project the LLM action");
    assert!(completed_second_index < llm_call_index);
}

#[test]
fn bulk_read_ignores_stdio_tty_and_raw_stdio_payload_until_real_boundary() {
    let mut runtime = bulk_runtime(false);
    let process = ProcessIdentity::new(AGENT_GENERATION);

    runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 20),
        process.clone(),
        PATH_A,
    ));
    let bulk_output = runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 21),
        process.clone(),
        PATH_B,
    ));
    let bulk = bulk_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileBulkRead)
        .expect("threshold event should start a bulk read action");
    let bulk_action_id = bulk.action_id.clone();

    for (offset, stream) in ["stdin", "stdout", "stderr"].into_iter().enumerate() {
        let output = runtime.observe_event(&stdio_event(
            EventId::new(FILE_READ_EVENT_ID.get() + 22 + offset as u64),
            process.clone(),
            stream,
        ));
        assert_bulk_not_completed(&output, &bulk_action_id);
    }

    let tty_output = runtime.observe_event(&write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 25),
        process.clone(),
        TTY_PATH,
    ));
    assert!(
        tty_output
            .actions
            .iter()
            .any(|action| action.kind == SemanticActionKind::FileTtyIo)
    );
    assert_bulk_not_completed(&tty_output, &bulk_action_id);

    let tty_truncate_output = runtime.observe_event(&truncate_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 26),
        process.clone(),
        TTY_PATH,
    ));
    assert!(tty_truncate_output.raw_event_consumed);
    assert!(!tty_truncate_output.retain_event);
    assert!(
        tty_truncate_output
            .actions
            .iter()
            .all(|action| action.kind != SemanticActionKind::FileModify)
    );
    assert_bulk_not_completed(&tty_truncate_output, &bulk_action_id);

    let pathless_write_output = runtime.observe_event(&pathless_write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 27),
        process.clone(),
    ));
    assert!(
        pathless_write_output
            .actions
            .iter()
            .any(|action| action.kind == SemanticActionKind::FileModify)
    );
    assert_bulk_not_completed(&pathless_write_output, &bulk_action_id);

    let resource_output = runtime.observe_event(&resource_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 28),
        process.clone(),
    ));
    assert_bulk_not_completed(&resource_output, &bulk_action_id);

    let raw_stdio_payload_output =
        runtime.observe_payload_segment(&stdio_payload_segment(process.clone()));
    assert_bulk_not_completed(&raw_stdio_payload_output, &bulk_action_id);

    let write_output = runtime.observe_event(&write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 29),
        process,
        WRITE_PATH,
    ));
    let completed = completed_bulk_action(&write_output, &bulk_action_id)
        .expect("real file write should complete the current bulk read");
    assert_eq!(completed.completeness, SemanticActionCompleteness::Complete);
    assert_eq!(write_output.file_path_sets.len(), 1);
    assert_eq!(write_output.file_path_sets[0].action_id, bulk_action_id);
}

#[test]
fn bulk_read_consumed_io_does_not_reuse_stale_fd_path_for_later_write() {
    let mut runtime = bulk_runtime_with_min_unique_paths(false, STALE_FD_BULK_MIN_UNIQUE_PATHS);
    let process = ProcessIdentity::new(AGENT_GENERATION);

    runtime.observe_event(&open_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 30),
        process.clone(),
        PATH_A,
    ));
    let bulk_output = runtime.observe_event(&read_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 31),
        process.clone(),
        PATH_A,
    ));
    let bulk = bulk_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileBulkRead)
        .expect("read should activate bulk summary");
    let bulk_action_id = bulk.action_id.clone();

    let write_output = runtime.observe_event(&write_event(
        EventId::new(FILE_READ_EVENT_ID.get() + 32),
        process,
        WRITE_PATH,
    ));

    let completed = completed_bulk_action(&write_output, &bulk_action_id)
        .expect("real file write should complete the current bulk read");
    assert_eq!(completed.completeness, SemanticActionCompleteness::Complete);
    let write_action = write_output
        .actions
        .iter()
        .find(|action| action.kind == SemanticActionKind::FileWrite)
        .expect("write should project a file.write action");
    assert_eq!(
        write_action
            .attributes
            .get(attrs::file::PATH)
            .map(String::as_str),
        Some(WRITE_PATH)
    );
}

fn bulk_runtime(agent_enabled: bool) -> LiveSemanticActionRuntime {
    bulk_runtime_with_min_unique_paths(agent_enabled, BULK_MIN_UNIQUE_PATHS)
}

fn bulk_runtime_with_min_unique_paths(
    agent_enabled: bool,
    min_unique_paths: u32,
) -> LiveSemanticActionRuntime {
    bulk_runtime_with_retention(
        agent_enabled,
        min_unique_paths,
        FileRawEventRetention::Summary,
    )
}

fn bulk_runtime_with_retention(
    agent_enabled: bool,
    min_unique_paths: u32,
    raw_event_retention: FileRawEventRetention,
) -> LiveSemanticActionRuntime {
    let mut file_observation = FileObservationConfig::default();
    file_observation.bulk_read.mode = FileBulkReadMode::PathSet;
    file_observation.bulk_read.raw_event_retention = raw_event_retention;
    file_observation.bulk_read.min_unique_paths = min_unique_paths;
    file_observation.bulk_read.max_paths_per_set = BULK_MAX_PATHS_PER_SET;
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

fn open_event(event_id: EventId, process: ProcessIdentity, path: &str) -> DomainEvent {
    file_access_event(event_id, process, "open", path)
}

fn read_event(event_id: EventId, process: ProcessIdentity, path: &str) -> DomainEvent {
    file_access_event(event_id, process, "read", path)
}

fn write_event(event_id: EventId, process: ProcessIdentity, path: &str) -> DomainEvent {
    file_access_event(event_id, process, "write", path)
}

fn truncate_event(event_id: EventId, process: ProcessIdentity, path: &str) -> DomainEvent {
    let mut event = file_event(event_id, process, "truncate", 0, None);
    set_file_path(&mut event, path);
    event
}

fn pathless_write_event(event_id: EventId, process: ProcessIdentity) -> DomainEvent {
    let mut event = write_event(event_id, process, WRITE_PATH);
    let EventPayload::File(payload) = &mut event.payload else {
        unreachable!("write_event returns a file payload");
    };
    payload.path = None;
    payload.metadata.remove("fd_target");
    event
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

fn stdio_event(event_id: EventId, process: ProcessIdentity, stream: &str) -> DomainEvent {
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: SystemTime::UNIX_EPOCH,
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::Stdio,
            flags: EventFlags::clean(),
        },
        EventPayload::Stdio(StdioPayload {
            stream: stream.to_string(),
            data: STDIO_BYTES.to_vec(),
            original_size: Some(STDIO_BYTES.len()),
            truncated: false,
        }),
    )
}

fn resource_event(event_id: EventId, process: ProcessIdentity) -> DomainEvent {
    DomainEvent::new(
        EventEnvelope {
            event_id,
            trace_id: TRACE_ID,
            observed_at: SystemTime::UNIX_EPOCH,
            process,
            collector: CollectorName::new("test"),
            kind: EventKind::Resource,
            flags: EventFlags::clean(),
        },
        EventPayload::Resource(ResourcePayload {
            scope: "process_tree".to_string(),
            subject: "pid:test".to_string(),
            cpu_percent_millis: None,
            rss_kb: None,
            virtual_memory_kb: None,
            metadata: Default::default(),
        }),
    )
}

fn stdio_payload_segment(process: ProcessIdentity) -> PayloadSegment {
    let mut segment = llm_payload_segment(process);
    let size = STDIO_BYTES.len() as u64;
    segment.source_boundary = PayloadSourceBoundary::Stdio;
    segment.stream_key = PayloadStreamKey::new("stdio:test:stdout");
    segment.original_size = size;
    segment.captured_size = size;
    segment.operation_original_size = size;
    segment.operation_captured_size = size;
    segment.library = "stdio".to_string();
    segment.symbol = "write".to_string();
    segment.protocol_hint = Some("stdio".to_string());
    segment.bytes = STDIO_BYTES.to_vec();
    segment
}

fn completed_bulk_action<'a>(
    output: &'a LiveSemanticActionOutput,
    action_id: &str,
) -> Option<&'a SemanticAction> {
    output.actions.iter().find(|action| {
        action.kind == SemanticActionKind::FileBulkRead && action.action_id == action_id
    })
}

fn assert_bulk_not_completed(output: &LiveSemanticActionOutput, action_id: &str) {
    assert!(completed_bulk_action(output, action_id).is_none());
    assert!(output.file_path_sets.is_empty());
}

fn expected_paths(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_string()).collect()
}
