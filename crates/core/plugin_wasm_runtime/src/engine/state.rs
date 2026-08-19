use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::payload::{PayloadSegment, PayloadSourceBoundary};
use model_core::trace::TraceAlertToken;
use plugin_system::{
    AlertHost, CommandExecutionContext, CommandPolicyHost, FilePolicyHost, FilePolicyReadContext,
    NetworkActionContext, NetworkPolicyHost, PluginHostGrants, PostTraceHost,
};
use wasmtime::{StoreLimits, StoreLimitsBuilder};

use super::WasmHostcallMetrics;

pub(crate) struct WasmStoreState {
    limits: StoreLimits,
    host_grants: PluginHostGrants,
    host_limits: WasmHostLimits,
    hostcall_metrics: Arc<WasmHostcallMetrics>,
    plugin_config: Option<Vec<u8>>,
    payload_snapshot: BTreeMap<String, PayloadSnapshotEntry>,
    observation_trace_context: Option<ObservationTraceContext>,
    requested_reevaluation_at: Option<SystemTime>,
    alert_host: Option<Arc<dyn AlertHost>>,
    post_trace_host: Option<Arc<dyn PostTraceHost>>,
    post_trace_task: Option<PostTraceTaskContext>,
    control_context: Option<ControlContextSnapshot>,
    file_policy_context: Option<FilePolicyReadContext>,
    file_policy_host: Option<Arc<dyn FilePolicyHost>>,
    file_policy_owner_instance_id: Option<String>,
    command_execution_context: Option<CommandExecutionContext>,
    command_policy_host: Option<Arc<dyn CommandPolicyHost>>,
    command_policy_owner_instance_id: Option<String>,
    network_action_context: Option<NetworkActionContext>,
    network_policy_host: Option<Arc<dyn NetworkPolicyHost>>,
    network_policy_owner_instance_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ObservationTraceContext {
    pub(crate) trace_id: TraceId,
    pub(crate) working_directory: Option<String>,
    pub(crate) alert_token: Option<TraceAlertToken>,
    pub(crate) activity_page_max_count: usize,
    pub(crate) activity_total_max_count: usize,
    pub(crate) activity_rows_read: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PostTraceCallLimits {
    pub(crate) action_page_max_count: usize,
    pub(crate) action_total_max_count: usize,
    pub(crate) activity_page_max_count: usize,
    pub(crate) activity_total_max_count: usize,
    pub(crate) file_state_query_max_count: usize,
}

pub(crate) struct PostTraceTaskContext {
    pub(crate) trace_id: TraceId,
    pub(crate) limits: PostTraceCallLimits,
    pub(crate) action_rows_read: usize,
    pub(crate) activity_rows_read: usize,
    pub(crate) file_state_queries: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct WasmHostLimits {
    pub(crate) env_name_max_bytes: usize,
    pub(crate) env_value_max_bytes: usize,
    pub(crate) payload_segment_max_count: usize,
    pub(crate) payload_ref_max_bytes: usize,
    pub(crate) payload_read_max_bytes: usize,
    pub(crate) context_ref_max_bytes: usize,
    pub(crate) context_query_max_bytes: usize,
    pub(crate) context_read_max_bytes: usize,
    pub(crate) file_policy_context_ref_max_bytes: usize,
    pub(crate) file_policy_query_max_bytes: usize,
    pub(crate) file_policy_io_max_bytes: usize,
    pub(crate) command_context_ref_max_bytes: usize,
    pub(crate) command_context_query_max_bytes: usize,
    pub(crate) command_context_read_max_bytes: usize,
    pub(crate) command_policy_io_max_bytes: usize,
    pub(crate) network_context_ref_max_bytes: usize,
    pub(crate) network_query_max_bytes: usize,
    pub(crate) network_policy_io_max_bytes: usize,
    pub(crate) plugin_config_read_max_bytes: usize,
    pub(crate) plugin_command_argv_max_count: usize,
    pub(crate) plugin_command_arg_max_bytes: usize,
    pub(crate) plugin_command_output_max_bytes: usize,
    pub(crate) plugin_command_timeout_ms: u64,
    pub(crate) alert_payload_max_bytes: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct ControlContextSnapshot {
    pub(crate) context_ref: String,
    pub(crate) decision_id: String,
    pub(crate) trace_id: String,
    pub(crate) subject: String,
    pub(crate) operation: String,
    pub(crate) target_summary: String,
    pub(crate) actor_process_identity: String,
}

pub(crate) struct PayloadSnapshotEntry {
    pub(crate) source_boundary: PayloadSourceBoundary,
    pub(crate) bytes: Option<Vec<u8>>,
}

impl WasmStoreState {
    pub(super) fn new(
        memory_max_bytes: usize,
        host_grants: PluginHostGrants,
        host_limits: WasmHostLimits,
        hostcall_metrics: Arc<WasmHostcallMetrics>,
    ) -> Self {
        Self {
            limits: StoreLimitsBuilder::new()
                .memory_size(memory_max_bytes)
                .build(),
            host_grants,
            host_limits,
            hostcall_metrics,
            plugin_config: None,
            payload_snapshot: BTreeMap::new(),
            observation_trace_context: None,
            requested_reevaluation_at: None,
            alert_host: None,
            post_trace_host: None,
            post_trace_task: None,
            control_context: None,
            file_policy_context: None,
            file_policy_host: None,
            file_policy_owner_instance_id: None,
            command_execution_context: None,
            command_policy_host: None,
            command_policy_owner_instance_id: None,
            network_action_context: None,
            network_policy_host: None,
            network_policy_owner_instance_id: None,
        }
    }

    pub(super) fn store_limits_mut(&mut self) -> &mut StoreLimits {
        &mut self.limits
    }

    pub(crate) fn host_grants(&self) -> &PluginHostGrants {
        &self.host_grants
    }

    pub(crate) fn host_limits(&self) -> &WasmHostLimits {
        &self.host_limits
    }

    pub(crate) fn hostcall_metrics(&self) -> &WasmHostcallMetrics {
        &self.hostcall_metrics
    }

    pub(crate) fn payload_entry(&self, ref_id: &str) -> Option<&PayloadSnapshotEntry> {
        self.payload_snapshot.get(ref_id)
    }

    pub(crate) fn plugin_config(&self) -> Option<&[u8]> {
        self.plugin_config.as_deref()
    }

    pub(crate) fn set_plugin_config(&mut self, plugin_config: Option<&str>) {
        self.plugin_config = plugin_config.map(|config| config.as_bytes().to_vec());
    }

    pub(crate) fn set_payload_snapshot(&mut self, segments: &[PayloadSegment]) {
        self.payload_snapshot.clear();
        for segment in segments
            .iter()
            .take(self.host_limits.payload_segment_max_count)
        {
            self.payload_snapshot.insert(
                segment.segment_id.to_string(),
                PayloadSnapshotEntry {
                    source_boundary: segment.source_boundary,
                    bytes: self
                        .host_grants
                        .can_read_payload_source(segment.source_boundary)
                        .then(|| segment.bytes.clone()),
                },
            );
        }
    }

    pub(crate) fn clear_payload_snapshot(&mut self) {
        self.payload_snapshot.clear();
    }

    pub(crate) fn observation_trace_context(&self) -> Option<&ObservationTraceContext> {
        self.observation_trace_context.as_ref()
    }

    pub(crate) fn observation_trace_context_mut(&mut self) -> Option<&mut ObservationTraceContext> {
        self.observation_trace_context.as_mut()
    }

    pub(crate) fn set_observation_trace_context(
        &mut self,
        trace_id: TraceId,
        working_directory: Option<String>,
        alert_token: &TraceAlertToken,
        activity_page_max_count: usize,
        activity_total_max_count: usize,
    ) {
        self.requested_reevaluation_at = None;
        self.observation_trace_context = Some(ObservationTraceContext {
            trace_id,
            working_directory,
            alert_token: self
                .host_grants
                .can_write_alerts()
                .then(|| alert_token.clone()),
            activity_page_max_count,
            activity_total_max_count,
            activity_rows_read: 0,
        });
    }

    pub(crate) fn clear_observation_trace_context(&mut self) {
        self.observation_trace_context = None;
    }

    pub(crate) fn request_reevaluation_at(&mut self, requested_at: SystemTime) {
        if self
            .requested_reevaluation_at
            .is_none_or(|current| requested_at < current)
        {
            self.requested_reevaluation_at = Some(requested_at);
        }
    }

    pub(crate) fn take_requested_reevaluation_at(&mut self) -> Option<SystemTime> {
        self.requested_reevaluation_at.take()
    }

    pub(crate) fn set_alert_host(&mut self, host: Option<Arc<dyn AlertHost>>) {
        self.alert_host = host;
    }

    pub(crate) fn alert_host(&self) -> Option<&Arc<dyn AlertHost>> {
        self.alert_host.as_ref()
    }

    pub(crate) fn set_post_trace_host(&mut self, host: Option<Arc<dyn PostTraceHost>>) {
        self.post_trace_host = host;
    }

    pub(crate) fn post_trace_host(&self) -> Option<&Arc<dyn PostTraceHost>> {
        self.post_trace_host.as_ref()
    }

    pub(crate) fn begin_post_trace_task(&mut self, trace_id: TraceId, limits: PostTraceCallLimits) {
        self.post_trace_task = Some(PostTraceTaskContext {
            trace_id,
            limits,
            action_rows_read: 0,
            activity_rows_read: 0,
            file_state_queries: 0,
        });
    }

    pub(crate) fn post_trace_task(&self) -> Option<&PostTraceTaskContext> {
        self.post_trace_task.as_ref()
    }

    pub(crate) fn post_trace_task_mut(&mut self) -> Option<&mut PostTraceTaskContext> {
        self.post_trace_task.as_mut()
    }

    pub(crate) fn end_post_trace_task(&mut self) {
        self.post_trace_task = None;
    }

    pub(crate) fn control_context(&self) -> Option<&ControlContextSnapshot> {
        self.control_context.as_ref()
    }

    pub(crate) fn set_control_context(&mut self, context: Option<ControlContextSnapshot>) {
        self.control_context = context;
    }

    pub(crate) fn clear_control_context(&mut self) {
        self.control_context = None;
    }

    pub(crate) fn file_policy_context(&self) -> Option<&FilePolicyReadContext> {
        self.file_policy_context.as_ref()
    }

    pub(crate) fn set_file_policy_context(&mut self, context: Option<FilePolicyReadContext>) {
        self.file_policy_context = context;
    }

    pub(crate) fn clear_file_policy_context(&mut self) {
        self.file_policy_context = None;
    }

    pub(crate) fn file_policy_host(&self) -> Option<&Arc<dyn FilePolicyHost>> {
        self.file_policy_host.as_ref()
    }

    pub(crate) fn file_policy_owner_instance_id(&self) -> Option<&str> {
        self.file_policy_owner_instance_id.as_deref()
    }

    pub(crate) fn set_file_policy_host(
        &mut self,
        owner_instance_id: impl Into<String>,
        host: Option<Arc<dyn FilePolicyHost>>,
    ) {
        self.file_policy_owner_instance_id = host.as_ref().map(|_| owner_instance_id.into());
        self.file_policy_host = host;
    }

    pub(crate) fn command_execution_context(&self) -> Option<&CommandExecutionContext> {
        self.command_execution_context.as_ref()
    }

    pub(crate) fn set_command_execution_context(
        &mut self,
        context: Option<CommandExecutionContext>,
    ) {
        self.command_execution_context = context;
    }

    pub(crate) fn clear_command_execution_context(&mut self) {
        self.command_execution_context = None;
    }

    pub(crate) fn command_policy_host(&self) -> Option<&Arc<dyn CommandPolicyHost>> {
        self.command_policy_host.as_ref()
    }

    pub(crate) fn command_policy_owner_instance_id(&self) -> Option<&str> {
        self.command_policy_owner_instance_id.as_deref()
    }

    pub(crate) fn set_command_policy_host(
        &mut self,
        owner_instance_id: impl Into<String>,
        host: Option<Arc<dyn CommandPolicyHost>>,
    ) {
        self.command_policy_owner_instance_id = host.as_ref().map(|_| owner_instance_id.into());
        self.command_policy_host = host;
    }

    pub(crate) fn network_action_context(&self) -> Option<&NetworkActionContext> {
        self.network_action_context.as_ref()
    }

    pub(crate) fn set_network_action_context(&mut self, context: Option<NetworkActionContext>) {
        self.network_action_context = context;
    }

    pub(crate) fn clear_network_action_context(&mut self) {
        self.network_action_context = None;
    }

    pub(crate) fn network_policy_host(&self) -> Option<&Arc<dyn NetworkPolicyHost>> {
        self.network_policy_host.as_ref()
    }

    pub(crate) fn network_policy_owner_instance_id(&self) -> Option<&str> {
        self.network_policy_owner_instance_id.as_deref()
    }

    pub(crate) fn set_network_policy_host(
        &mut self,
        owner_instance_id: impl Into<String>,
        host: Option<Arc<dyn NetworkPolicyHost>>,
    ) {
        self.network_policy_owner_instance_id = host.as_ref().map(|_| owner_instance_id.into());
        self.network_policy_host = host;
    }
}
