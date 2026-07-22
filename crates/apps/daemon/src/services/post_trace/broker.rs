use std::collections::BTreeMap;
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::time::{Duration, Instant, SystemTime};

use config_core::daemon::PostTraceRuntimeConfig;
use control_contract::reply::ControlError;
use model_core::ids::TraceId;
use plugin_system::{
    PluginManifest, PluginRuntimeError, TraceAnalysisActionPage, TraceFileState,
    TraceFileStateStatus,
};
use semantic_action::{SemanticActionKind, SemanticActionStatus};
use storage_core::StorageBackend;

use super::facts::{
    analysis_context, observed_host_path, project_analysis_action, read_file_state,
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
        self.registration(scope)?;
        match operation {
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

    fn analysis_action_page(
        &self,
        storage: &mut dyn StorageBackend,
        trace_id: TraceId,
        offset: usize,
        limit: usize,
    ) -> Result<TraceAnalysisActionPage, PluginRuntimeError> {
        storage
            .get_trace(trace_id)
            .map_err(storage_runtime_error)?
            .ok_or_else(|| trace_missing(trace_id))?;
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
}
