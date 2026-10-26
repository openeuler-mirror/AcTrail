//! File mutation, directory enumeration and typed I/O summary projection.
use super::super::shared::{
    FileFdOwner, FileFdRegistry, event_fd, event_file_path, event_result,
    file_open_has_directory_flag,
};
use super::enumerate::{FsEnumerateOutput, FsEnumerateProjector};
use super::summary::FileSummaryProjector;
use crate::live::actions::{is_file_modify_event, is_file_modify_operation};
use crate::live::runtime::LiveSemanticActionOutput;
use config_core::daemon::FileObservationConfig;
use model_core::event::{DomainEvent, EventPayload};
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use std::time::SystemTime;

pub(in crate::live) struct FileAccessProjector {
    enumerate: FsEnumerateProjector,
    summary: FileSummaryProjector,
    fd_registry: FileFdRegistry,
}
impl FileAccessProjector {
    pub(in crate::live) fn projects_modify_event(&self, event: &DomainEvent) -> bool {
        let EventPayload::File(payload) = &event.payload else {
            return false;
        };
        let collection = self.summary.collection();
        match payload.operation.as_str() {
            "open" => collection.writable_open && is_file_modify_event(event),
            "unlink" | "rename" | "mkdir" | "rmdir" => collection.path_mutations,
            "truncate" => {
                if payload
                    .metadata
                    .get("syscall")
                    .is_some_and(|name| name == "ftruncate")
                {
                    collection.fd_mutations
                } else {
                    collection.path_mutations
                }
            }
            "mmap_shared" => collection.fd_mutations,
            _ => false,
        }
    }

    pub(in crate::live) fn new(config: FileObservationConfig) -> Self {
        Self {
            enumerate: FsEnumerateProjector::new(config.enumerate.clone()),
            summary: FileSummaryProjector::new(config),
            fd_registry: FileFdRegistry::default(),
        }
    }
    pub(in crate::live) fn observe_file_event(
        &mut self,
        event: &DomainEvent,
    ) -> LiveSemanticActionOutput {
        let EventPayload::File(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        if payload.io_summary.is_some() {
            return self.summary.observe(event);
        }
        if fd_duplicate_lifecycle_operation(&payload.operation) {
            self.fd_registry.duplicate(event);
            let success = !event_result(event).is_some_and(|result| result < 0);
            return LiveSemanticActionOutput {
                retain_event: !success,
                raw_event_consumed: success,
                ..LiveSemanticActionOutput::default()
            };
        }
        if payload.operation == "open" && file_open_has_directory_flag(payload) {
            if let Some(output) = self.observe_directory_open(event) {
                return output;
            }
        }
        if payload.operation == "close" {
            if let Some(output) = self.observe_owned_close(event) {
                return output;
            }
        }
        let mut output = LiveSemanticActionOutput::default();
        if is_file_modify_operation(&payload.operation) || is_file_modify_event(event) {
            append_output(
                &mut output,
                live_output_from_enumerate(
                    self.enumerate.observe_boundary(
                        event.envelope.trace_id,
                        &event.envelope.process,
                        event.envelope.observed_at,
                    ),
                    false,
                ),
            );
        }
        append_output(&mut output, self.summary.observe(event));
        consume_successful_close(&mut output, event);
        consume_successful_unprojectable_file_event(&mut output, event);
        output
    }
    pub(in crate::live) fn finish_file_io_batch(&mut self) -> LiveSemanticActionOutput {
        self.summary.finish_batch()
    }
    pub(in crate::live) fn observe_boundary(
        &mut self,
        trace_id: TraceId,
        process: &ProcessIdentity,
        observed_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        live_output_from_enumerate(
            self.enumerate
                .observe_boundary(trace_id, process, observed_at),
            false,
        )
    }
    pub(in crate::live) fn observe_boundary_for_event(
        &mut self,
        event: &DomainEvent,
    ) -> LiveSemanticActionOutput {
        let output = self.observe_boundary(
            event.envelope.trace_id,
            &event.envelope.process,
            event.envelope.observed_at,
        );
        if matches!(&event.payload, EventPayload::Process(payload) if payload.operation == "exit") {
            self.fd_registry
                .forget_process(event.envelope.trace_id, &event.envelope.process);
        }
        output
    }
    pub(in crate::live) fn forget_trace(&mut self, trace_id: TraceId) {
        self.enumerate.forget_trace(trace_id);
        self.summary.forget_trace(trace_id);
        self.fd_registry.forget_trace(trace_id);
    }
    pub(in crate::live) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let mut output = self.summary.finalize_trace(trace_id, finished_at);
        append_output(
            &mut output,
            live_output_from_enumerate(self.enumerate.finalize_trace(trace_id, finished_at), false),
        );
        output
    }
    fn observe_directory_open(&mut self, event: &DomainEvent) -> Option<LiveSemanticActionOutput> {
        if !self.enumerate.enabled() {
            return None;
        }
        let path = event_file_path(event)?;
        if !event_result(event).is_some_and(|result| result < 0) {
            if let Some(fd) = event_fd(event) {
                self.fd_registry
                    .insert(event, fd, FileFdOwner::FsEnumerate, path.clone());
            }
        }
        let enumerate = self.enumerate.observe_open(event, path);
        Some(live_output_from_enumerate(
            enumerate.clone(),
            enumerate.consumed_by_summary,
        ))
    }

    fn observe_owned_close(&mut self, event: &DomainEvent) -> Option<LiveSemanticActionOutput> {
        let fd = event_fd(event)?;
        let state = self.fd_registry.close_state(event, fd)?;
        match state.owner {
            FileFdOwner::FsEnumerate => {
                let enumerate = self.enumerate.observe_close(event, state.path);
                Some(live_output_from_enumerate(
                    enumerate.clone(),
                    enumerate.consumed_by_summary,
                ))
            }
        }
    }
}

fn fd_duplicate_lifecycle_operation(operation: &str) -> bool {
    matches!(operation, "dup" | "dup2" | "dup3" | "fcntl_dup")
}

fn consume_successful_close(output: &mut LiveSemanticActionOutput, event: &DomainEvent) {
    let EventPayload::File(payload) = &event.payload else {
        return;
    };
    if payload.operation == "close" && !event_result(event).is_some_and(|result| result < 0) {
        output.retain_event = false;
        output.raw_event_consumed = true;
    }
}

fn consume_successful_unprojectable_file_event(
    output: &mut LiveSemanticActionOutput,
    event: &DomainEvent,
) {
    let EventPayload::File(payload) = &event.payload else {
        return;
    };
    if is_file_modify_operation(&payload.operation) || is_file_modify_event(event) {
        return;
    }
    if event_result(event).is_some_and(|result| result < 0) {
        return;
    }
    if event_file_path(event).is_some() {
        return;
    }
    output.retain_event = false;
    output.raw_event_consumed = true;
}

fn live_output_from_enumerate(
    enumerate: FsEnumerateOutput,
    raw_event_consumed: bool,
) -> LiveSemanticActionOutput {
    let mut output = LiveSemanticActionOutput {
        actions: enumerate.actions,
        ..LiveSemanticActionOutput::default()
    };
    output.file_path_sets = enumerate.file_path_sets;
    output.retain_event = enumerate.retain_event;
    output.raw_event_consumed = raw_event_consumed;
    output
}

fn append_output(output: &mut LiveSemanticActionOutput, other: LiveSemanticActionOutput) {
    output.actions.extend(other.actions);
    output.links.extend(other.links);
    output
        .file_observation_paths
        .extend(other.file_observation_paths);
    output.file_path_sets.extend(other.file_path_sets);
    output
        .llm_request_contents
        .extend(other.llm_request_contents);
    output
        .llm_pipeline_diagnostics
        .extend(other.llm_pipeline_diagnostics);
    output.deferred_events.extend(other.deferred_events);
    output.retain_event = output.retain_event && other.retain_event;
    output.raw_event_consumed = output.raw_event_consumed || other.raw_event_consumed;
}
