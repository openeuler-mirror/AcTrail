use std::sync::Arc;
use std::time::SystemTime;

use model_core::payload::PayloadSegment;
use model_core::trace::TraceRecord;
use semantic_action::{
    FileChangeKind, FileObservationPath, SemanticAction, SemanticActionCompleteness,
    SemanticActionLink, SemanticActionStatus,
};
use serde::{Deserialize, Serialize};

use crate::{
    PluginDroppedRecord, PluginHostcallMetricsSource, PluginRuntimeError, PluginRuntimeKind,
};

pub const DEFAULT_OBSERVATION_QUEUE_CAPACITY: u32 = 4096;
pub const DEFAULT_OBSERVATION_EVENT_FAMILIES: [ObservationEventFamily; 2] = [
    ObservationEventFamily::SemanticAction,
    ObservationEventFamily::SemanticActionLink,
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObservationEventFamily {
    SemanticAction,
    SemanticActionLink,
    Diagnostic,
    TraceLifecycle,
    ResourceMetric,
    PayloadMetadata,
}

pub struct ObservationBatch<'a> {
    pub trace: &'a TraceRecord,
    pub semantic_actions: &'a [SemanticAction],
    pub semantic_links: &'a [SemanticActionLink],
    pub file_observation_paths: &'a [FileObservationPath],
    pub payload_segments: &'a [PayloadSegment],
}

pub trait ObservationConsumer: Send + Sync {
    fn instance_id(&self) -> &str;

    fn plugin_id(&self) -> &str;

    fn runtime_kind(&self) -> PluginRuntimeKind;

    fn host_grants(&self) -> Vec<String> {
        Vec::new()
    }

    fn hostcall_metrics_source(&self) -> Option<Arc<dyn PluginHostcallMetricsSource>> {
        None
    }

    fn payload_snapshot_limit(&self) -> Option<usize> {
        None
    }

    fn observation_queue_capacity(&self) -> u32 {
        DEFAULT_OBSERVATION_QUEUE_CAPACITY
    }

    fn subscribed_event_families(&self) -> Vec<ObservationEventFamily> {
        DEFAULT_OBSERVATION_EVENT_FAMILIES.to_vec()
    }

    fn post_trace_analyzer(&self) -> Option<&dyn PostTraceAnalyzer> {
        None
    }

    fn consume(
        &self,
        batch: ObservationBatch<'_>,
    ) -> Result<ObservationConsumeReport, PluginRuntimeError>;

    fn finish(&self) -> Result<ObservationConsumeReport, PluginRuntimeError> {
        Ok(ObservationConsumeReport::empty())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostTraceTask {
    pub trace_id: model_core::ids::TraceId,
    pub timeout_ms: u64,
}

pub trait PostTraceAnalyzer: Send + Sync {
    fn analyze_post_trace(&self, task: PostTraceTask) -> Result<(), PluginRuntimeError>;

    fn cancel_post_trace(&self);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceAnalysisContext {
    pub terminal_at: SystemTime,
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceAnalysisFileChange {
    pub operation: String,
    pub change_kind: FileChangeKind,
    pub successful: bool,
    pub path: Option<String>,
    pub path_complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceAnalysisAction {
    pub action_id: String,
    pub kind: String,
    pub status: SemanticActionStatus,
    pub completeness: SemanticActionCompleteness,
    pub file_change: Option<TraceAnalysisFileChange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceAnalysisActionPage {
    pub actions: Vec<TraceAnalysisAction>,
    pub next_offset: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceActivityContext {
    pub root_container_id: Option<String>,
    pub root_process_id: String,
    pub display_name: String,
    pub profile_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceLlmExchange {
    pub call_action_id: String,
    pub request_action_id: String,
    pub response_action_id: Option<String>,
    pub process_id: String,
    pub model: Option<String>,
    pub server_address: Option<String>,
    pub url_path: Option<String>,
    pub started_at: SystemTime,
    pub completed_at: Option<SystemTime>,
    pub request_body_bytes: u64,
    pub request_raw_bytes: Option<u64>,
    pub request_complete: bool,
    pub response_body_bytes: Option<u64>,
    pub response_raw_bytes: Option<u64>,
    pub response_complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceLlmExchangePage {
    pub exchanges: Vec<TraceLlmExchange>,
    pub next_offset: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceCommandExecution {
    pub action_id: String,
    pub process_id: String,
    pub executable: Option<String>,
    pub command_line: Option<String>,
    pub started_at: SystemTime,
    pub ended_at: Option<SystemTime>,
    pub status: SemanticActionStatus,
    pub exit_code: Option<i32>,
    pub agent_action_id: Option<String>,
    pub parent_command_action_id: Option<String>,
    pub top_level_agent_child: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceCommandExecutionPage {
    pub commands: Vec<TraceCommandExecution>,
    pub next_offset: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceFileStateStatus {
    Exists,
    NotFound,
    Inaccessible,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceFileState {
    pub status: TraceFileStateStatus,
    pub checked_at: SystemTime,
    pub file_kind: Option<String>,
}

pub trait PostTraceHost: Send + Sync {
    fn analysis_context(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<TraceAnalysisContext, PluginRuntimeError>;

    fn semantic_actions_page(
        &self,
        trace_id: model_core::ids::TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<TraceAnalysisActionPage, PluginRuntimeError>;

    fn activity_context(
        &self,
        trace_id: model_core::ids::TraceId,
    ) -> Result<TraceActivityContext, PluginRuntimeError>;

    fn llm_exchanges_page(
        &self,
        trace_id: model_core::ids::TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<TraceLlmExchangePage, PluginRuntimeError>;

    fn command_executions_page(
        &self,
        trace_id: model_core::ids::TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<TraceCommandExecutionPage, PluginRuntimeError>;

    fn file_state(
        &self,
        trace_id: model_core::ids::TraceId,
        action_id: &str,
    ) -> Result<TraceFileState, PluginRuntimeError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationConsumeReport {
    pub dropped_records: Vec<PluginDroppedRecord>,
}

impl ObservationConsumeReport {
    pub fn empty() -> Self {
        Self {
            dropped_records: Vec::new(),
        }
    }
}
