use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::{Duration, Instant, SystemTime};

use config_core::daemon::PostTraceRuntimeConfig;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use model_core::payload::PayloadSegmentId;
use plugin_system::{
    PayloadReadResult, PluginHostGrants, PluginManifest, PluginRuntimeError, TraceActivityContext,
    TraceAnalysisActionPage, TraceCommandExecutionPage, TraceFileState, TraceFileStateStatus,
    TraceLlmExchangePage,
};
use semantic_action::{SemanticActionKind, SemanticActionStatus};
use storage_core::{PayloadRowLimit, PayloadSegmentQuery, StorageBackend};

use super::facts::{
    activity_context as project_activity_context, analysis_context, observed_host_path,
    project_analysis_action, project_command_executions, project_llm_exchanges, read_file_state,
    storage_runtime_error, trace_missing,
};
use super::protocol::{
    BrokerOperation, BrokerRequest, BrokerResponse, EventSignal, PluginScope, PostTraceHostClient,
};

pub(crate) struct PostTraceBroker {
    request_sender: SyncSender<BrokerRequest>,
    request_receiver: Receiver<BrokerRequest>,
    signal: Arc<EventSignal>,
    registrations: BTreeMap<String, RegisteredPlugin>,
    reply_timeout: Duration,
    requests_per_cycle: usize,
}

impl PostTraceBroker {
    pub(crate) fn new(config: PostTraceRuntimeConfig) -> Result<Self, ControlError> {
        let queue_capacity = usize::try_from(config.broker_queue_capacity).map_err(|error| {
            ControlError::new(
                "post_trace_config",
                format!("broker queue capacity overflow: {error}"),
            )
        })?;
        let requests_per_cycle = usize::try_from(config.requests_per_cycle).map_err(|error| {
            ControlError::new(
                "post_trace_config",
                format!("requests per cycle overflow: {error}"),
            )
        })?;
        let (request_sender, request_receiver) = sync_channel(queue_capacity);
        Ok(Self {
            request_sender,
            request_receiver,
            signal: Arc::new(EventSignal::new()?),
            registrations: BTreeMap::new(),
            reply_timeout: Duration::from_millis(config.broker_reply_timeout_ms),
            requests_per_cycle,
        })
    }

    pub(crate) fn event_poll_fd(&self) -> RawFd {
        self.signal.as_raw_fd()
    }

    pub(crate) fn register_plugin(
        &mut self,
        instance_id: &str,
        manifest: &PluginManifest,
        host_grants: &PluginHostGrants,
    ) -> Result<Arc<PostTraceHostClient>, ControlError> {
        if self.registrations.contains_key(instance_id) {
            return Err(ControlError::new(
                "post_trace_registration",
                format!("post-trace plugin instance {instance_id} is already registered"),
            ));
        }
        self.registrations.insert(
            instance_id.to_string(),
            RegisteredPlugin {
                plugin_id: manifest.id().to_string(),
                host_grants: host_grants.clone(),
                payload_read_max_bytes: manifest.hostcall_limits.payload.read_max_bytes,
            },
        );
        let file_state_timeout = Duration::from_millis(
            manifest
                .hostcall_limits
                .trace_file_state
                .timeout_ms
                .unwrap_or(
                    self.reply_timeout
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX),
                ),
        )
        .min(self.reply_timeout);
        Ok(Arc::new(PostTraceHostClient::new(
            PluginScope {
                instance_id: instance_id.to_string(),
                plugin_id: manifest.id().to_string(),
            },
            self.request_sender.clone(),
            Arc::clone(&self.signal),
            self.reply_timeout,
            file_state_timeout,
        )))
    }

    pub(crate) fn unregister_plugin(&mut self, instance_id: &str) {
        self.registrations.remove(instance_id);
    }

    pub(crate) fn drain_requests(
        &mut self,
        storage: &mut dyn StorageBackend,
    ) -> Result<usize, ControlError> {
        self.signal.drain()?;
        let mut processed = 0_usize;
        while processed < self.requests_per_cycle {
            let request = match self.request_receiver.try_recv() {
                Ok(request) => request,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            };
            let response = if Instant::now() >= request.expires_at {
                Err(PluginRuntimeError::new(
                    "post_trace_host_timeout",
                    "host request expired before daemon execution",
                ))
            } else {
                self.handle_request(storage, &request.scope, request.operation)
            };
            let _ = request.reply.send(response);
            processed += 1;
        }
        if processed == self.requests_per_cycle {
            let _ = self.signal.notify();
        }
        Ok(processed)
    }

    fn handle_request(
        &self,
        storage: &mut dyn StorageBackend,
        scope: &PluginScope,
        operation: BrokerOperation,
    ) -> Result<BrokerResponse, PluginRuntimeError> {
        if let BrokerOperation::ReadPayload {
            trace_id,
            segment_id,
            offset,
            max_bytes,
        } = operation
        {
            return Ok(BrokerResponse::ReadPayload(self.read_payload(
                storage, scope, trace_id, segment_id, offset, max_bytes,
            )));
        }
        self.registration(scope)?;
        match operation {
            BrokerOperation::ReadPayload { .. } => unreachable!("payload request handled above"),
            BrokerOperation::AnalysisContext { trace_id } => storage
                .get_trace(trace_id)
                .map_err(storage_runtime_error)?
                .ok_or_else(|| trace_missing(trace_id))
                .and_then(|trace| analysis_context(&trace))
                .map(BrokerResponse::AnalysisContext),
            BrokerOperation::SemanticActionsPage {
                trace_id,
                offset,
                limit,
            } => self
                .analysis_action_page(storage, trace_id, offset, limit)
                .map(BrokerResponse::SemanticActionsPage),
            BrokerOperation::ActivityContext { trace_id } => self
                .activity_context(storage, trace_id)
                .map(BrokerResponse::ActivityContext),
            BrokerOperation::LlmExchangesPage {
                trace_id,
                offset,
                limit,
            } => self
                .llm_exchanges_page(storage, trace_id, offset, limit)
                .map(BrokerResponse::LlmExchangesPage),
            BrokerOperation::CommandExecutionsPage {
                trace_id,
                offset,
                limit,
            } => self
                .command_executions_page(storage, trace_id, offset, limit)
                .map(BrokerResponse::CommandExecutionsPage),
            BrokerOperation::FileState {
                trace_id,
                action_id,
            } => self
                .trace_file_state(storage, trace_id, &action_id)
                .map(BrokerResponse::FileState),
        }
    }

    fn registration(&self, scope: &PluginScope) -> Result<&RegisteredPlugin, PluginRuntimeError> {
        let registration = self.registrations.get(&scope.instance_id).ok_or_else(|| {
            PluginRuntimeError::new(
                "post_trace_registration",
                format!("plugin instance {} is not registered", scope.instance_id),
            )
        })?;
        if registration.plugin_id != scope.plugin_id {
            return Err(PluginRuntimeError::new(
                "post_trace_registration",
                "plugin identity does not match the registered instance",
            ));
        }
        Ok(registration)
    }

    fn read_payload(
        &self,
        storage: &dyn StorageBackend,
        scope: &PluginScope,
        trace_id: TraceId,
        segment_id: u64,
        offset: u64,
        max_bytes: usize,
    ) -> PayloadReadResult {
        let Ok(registration) = self.registration(scope) else {
            return PayloadReadResult::Denied;
        };
        if !registration.host_grants.can_read_payload() {
            return PayloadReadResult::Denied;
        }
        let query = PayloadSegmentQuery {
            segment_id: Some(PayloadSegmentId::new(segment_id)),
            direction: None,
            limit: Some(PayloadRowLimit::Head(1)),
            include_bytes: false,
        };
        let metadata = match storage.list_payload_segments(trace_id, query) {
            Ok(mut rows) => match rows.pop() {
                Some(row) => row,
                None => return PayloadReadResult::NotFound,
            },
            Err(_) => return PayloadReadResult::Failed,
        };
        if metadata.trace_id != trace_id
            || metadata.segment_id.get() != segment_id
            || !registration
                .host_grants
                .can_read_payload_source(metadata.source_boundary)
        {
            return PayloadReadResult::Denied;
        }
        // This explicit host call reads one stored segment. Its response is bounded;
        // the existing storage interface may load that segment's complete body.
        let segment = match storage.list_payload_segments(
            trace_id,
            PayloadSegmentQuery {
                include_bytes: true,
                ..query
            },
        ) {
            Ok(mut rows) => match rows.pop() {
                Some(row) => row,
                None => return PayloadReadResult::NotFound,
            },
            Err(_) => return PayloadReadResult::Failed,
        };
        if segment.trace_id != trace_id
            || segment.segment_id.get() != segment_id
            || !registration
                .host_grants
                .can_read_payload_source(segment.source_boundary)
        {
            return PayloadReadResult::Denied;
        }
        let limit = registration
            .payload_read_max_bytes
            .map(|limit| limit as usize)
            .unwrap_or(max_bytes)
            .min(max_bytes);
        let total_bytes = segment.bytes.len();
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(total_bytes);
        let end = start.saturating_add(limit).min(total_bytes);
        PayloadReadResult::Chunk {
            bytes: segment.bytes[start..end].to_vec(),
            total_bytes,
            truncated: end < total_bytes,
        }
    }

    fn analysis_action_page(
        &self,
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<TraceAnalysisActionPage, PluginRuntimeError> {
        let page = storage
            .semantic_actions_page(trace_id, offset, limit)
            .map_err(storage_runtime_error)?;
        let mut actions = Vec::with_capacity(page.actions.len());
        for action in page.actions {
            let paths = storage
                .list_file_observation_paths(trace_id, &action.action_id)
                .map_err(storage_runtime_error)?;
            actions.push(project_analysis_action(action, paths));
        }
        Ok(TraceAnalysisActionPage {
            actions,
            next_offset: page.next_offset,
        })
    }

    fn activity_context(
        &self,
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
    ) -> Result<TraceActivityContext, PluginRuntimeError> {
        let trace = storage
            .get_trace(trace_id)
            .map_err(storage_runtime_error)?
            .ok_or_else(|| trace_missing(trace_id))?;
        Ok(project_activity_context(&trace))
    }

    fn llm_exchanges_page(
        &self,
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<TraceLlmExchangePage, PluginRuntimeError> {
        let actions = storage
            .semantic_actions_matching_kinds(
                trace_id,
                &[
                    SemanticActionKind::LlmCall.as_str(),
                    SemanticActionKind::LlmRequest.as_str(),
                    SemanticActionKind::LlmResponse.as_str(),
                ],
            )
            .map_err(storage_runtime_error)?;
        let exchanges = project_llm_exchanges(actions)?;
        let total = exchanges.len();
        let page = exchanges
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let next = offset.checked_add(page.len()).filter(|next| *next < total);
        Ok(TraceLlmExchangePage {
            exchanges: page,
            next_offset: next,
        })
    }

    fn command_executions_page(
        &self,
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<TraceCommandExecutionPage, PluginRuntimeError> {
        let actions = storage
            .semantic_actions_matching_kinds(
                trace_id,
                &[SemanticActionKind::CommandInvocation.as_str()],
            )
            .map_err(storage_runtime_error)?;
        let links = storage
            .list_semantic_action_links(trace_id)
            .map_err(storage_runtime_error)?;
        let memberships = storage
            .trace_memberships(trace_id)
            .map_err(storage_runtime_error)?;
        let commands = project_command_executions(actions, &links, &memberships)?;
        let total = commands.len();
        let page = commands
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();
        let next = offset.checked_add(page.len()).filter(|next| *next < total);
        Ok(TraceCommandExecutionPage {
            commands: page,
            next_offset: next,
        })
    }

    fn trace_file_state(
        &self,
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
        action_id: &str,
    ) -> Result<TraceFileState, PluginRuntimeError> {
        let trace = storage
            .get_trace(trace_id)
            .map_err(storage_runtime_error)?
            .ok_or_else(|| trace_missing(trace_id))?;
        let action = storage
            .semantic_action_by_id(trace_id, action_id)
            .map_err(storage_runtime_error)?
            .ok_or_else(|| {
                PluginRuntimeError::new(
                    "trace_file_state",
                    format!("semantic action {action_id} was not found in trace {trace_id}"),
                )
            })?;
        if !matches!(
            action.kind,
            SemanticActionKind::FileModify | SemanticActionKind::FileWrite
        ) || action.status != SemanticActionStatus::Success
        {
            return Err(PluginRuntimeError::new(
                "trace_file_state",
                "file-state reads require a successful file.modify or file.write action",
            ));
        }
        let paths = storage
            .list_file_observation_paths(trace_id, action_id)
            .map_err(storage_runtime_error)?;
        let observed_path = match action.kind {
            SemanticActionKind::FileModify => {
                let [path] = paths.as_slice() else {
                    return Err(PluginRuntimeError::new(
                        "trace_file_state",
                        "file.modify state reads require exactly one complete observed path",
                    ));
                };
                path.path.as_str()
            }
            SemanticActionKind::FileWrite => action
                .attributes
                .get(semantic_action::attr_keys::file::PATH)
                .map(String::as_str)
                .ok_or_else(|| {
                    PluginRuntimeError::new(
                        "trace_file_state",
                        "file.write state reads require a complete observed path",
                    )
                })?,
            _ => unreachable!("file-state action kind was validated above"),
        };
        let Some(host_path) = observed_host_path(storage, &trace, &action, observed_path)? else {
            return Ok(TraceFileState {
                status: TraceFileStateStatus::Unavailable,
                checked_at: SystemTime::now(),
                file_kind: None,
            });
        };
        Ok(read_file_state(&host_path))
    }
}

struct RegisteredPlugin {
    plugin_id: String,
    host_grants: PluginHostGrants,
    payload_read_max_bytes: Option<u32>,
}
