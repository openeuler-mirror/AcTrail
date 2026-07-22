use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use config_core::daemon::PostTraceRuntimeConfig;
use control_contract::reply::ControlError;
use export_core::ExportRuntime;
use model_core::ids::TraceId;
use plugin_system::{PluginRuntimeError, PostTraceTask};
use storage_core::{StorageBackend, TraceLease, TraceLeasePurpose};

pub(crate) struct PostTraceCoordinator {
    running: BTreeMap<TaskKey, TraceLease>,
    admitted: BTreeSet<TaskKey>,
    pending_since: BTreeMap<TaskKey, Instant>,
    diagnosed_admission_timeout: BTreeSet<TaskKey>,
    diagnosed_drain_timeout: BTreeSet<TaskKey>,
    barrier_ready: BTreeSet<TraceId>,
    admission_open: bool,
    max_in_flight_tasks: usize,
    admission_timeout: Duration,
    execution_timeout_ms: u64,
    shutdown_drain_timeout: Duration,
    cancellation_grace: Duration,
}

pub(crate) struct PostTraceAdmission {
    pub(crate) all_admitted: bool,
    pub(crate) timeout_diagnostics: Vec<PostTraceIssue>,
}

pub(crate) struct PostTraceIssue {
    pub(crate) trace_id: TraceId,
    pub(crate) instance_id: String,
    pub(crate) code: String,
    pub(crate) message: String,
}

pub(crate) struct PostTraceOutcome {
    pub(crate) trace_id: TraceId,
    pub(crate) instance_id: String,
    pub(crate) result: Result<(), PluginRuntimeError>,
}

impl PostTraceCoordinator {
    pub(crate) fn new(config: PostTraceRuntimeConfig) -> Result<Self, ControlError> {
        if config.shutdown_drain_timeout_ms <= config.broker_reply_timeout_ms {
            return Err(ControlError::new(
                "post_trace_config",
                "shutdown drain timeout must exceed broker reply timeout",
            ));
        }
        let max_in_flight_tasks = usize::try_from(config.max_in_flight_tasks).map_err(|error| {
            ControlError::new(
                "post_trace_config",
                format!("max in-flight task count overflow: {error}"),
            )
        })?;
        Ok(Self {
            running: BTreeMap::new(),
            admitted: BTreeSet::new(),
            pending_since: BTreeMap::new(),
            diagnosed_admission_timeout: BTreeSet::new(),
            diagnosed_drain_timeout: BTreeSet::new(),
            barrier_ready: BTreeSet::new(),
            admission_open: true,
            max_in_flight_tasks,
            admission_timeout: Duration::from_millis(config.admission_timeout_ms),
            execution_timeout_ms: config.execution_timeout_ms,
            shutdown_drain_timeout: Duration::from_millis(config.shutdown_drain_timeout_ms),
            cancellation_grace: Duration::from_millis(config.broker_reply_timeout_ms),
        })
    }

    pub(crate) fn barrier_ready(&self, trace_id: TraceId) -> bool {
        self.barrier_ready.contains(&trace_id)
    }

    pub(crate) fn has_running_tasks(&self) -> bool {
        !self.running.is_empty()
    }

    pub(crate) fn has_running_tasks_for(&self, instance_id: &str) -> bool {
        self.running
            .keys()
            .any(|key| key.instance_id == instance_id)
    }

    pub(crate) fn running_instance_ids(&self) -> Vec<String> {
        self.running
            .keys()
            .map(|key| key.instance_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn shutdown_drain_timeout(&self) -> Duration {
        self.shutdown_drain_timeout
    }

    pub(crate) fn cancellation_grace(&self) -> Duration {
        self.cancellation_grace
    }

    pub(crate) fn close_admission(&mut self) {
        self.admission_open = false;
    }

    pub(crate) fn mark_barrier_ready(&mut self, trace_id: TraceId) {
        self.barrier_ready.insert(trace_id);
    }

    pub(crate) fn admit_trace(
        &mut self,
        trace_id: TraceId,
        instance_ids: &[String],
        export_runtime: &ExportRuntime,
        storage: &mut dyn StorageBackend,
    ) -> Result<PostTraceAdmission, ControlError> {
        if !self.admission_open && !instance_ids.is_empty() {
            return Err(ControlError::new(
                "post_trace_closed",
                "post-trace task admission is closed during daemon shutdown",
            ));
        }
        let now = Instant::now();
        let mut all_admitted = true;
        let mut timeout_diagnostics = Vec::new();
        for instance_id in instance_ids {
            let key = TaskKey {
                trace_id,
                instance_id: instance_id.clone(),
            };
            if self.admitted.contains(&key) {
                continue;
            }
            all_admitted = false;
            let pending_since = *self.pending_since.entry(key.clone()).or_insert(now);
            let admission_result = if self.running.len() >= self.max_in_flight_tasks {
                Err(ControlError::new(
                    "post_trace_capacity",
                    "post-trace in-flight task capacity is full",
                ))
            } else {
                self.enqueue_task(&key, export_runtime, storage)
            };
            match admission_result {
                Ok(()) => {
                    self.admitted.insert(key.clone());
                    self.pending_since.remove(&key);
                }
                Err(error) => {
                    if now.duration_since(pending_since) >= self.admission_timeout
                        && self.diagnosed_admission_timeout.insert(key.clone())
                    {
                        timeout_diagnostics.push(PostTraceIssue {
                            trace_id,
                            instance_id: instance_id.clone(),
                            code: "post_trace_admission_timeout".to_string(),
                            message: format!(
                                "post-trace task admission exceeded {}ms: {}: {}",
                                self.admission_timeout.as_millis(),
                                error.code,
                                error.message
                            ),
                        });
                    }
                }
            }
        }
        if !all_admitted {
            all_admitted = instance_ids.iter().all(|instance_id| {
                self.admitted.contains(&TaskKey {
                    trace_id,
                    instance_id: instance_id.clone(),
                })
            });
        }
        Ok(PostTraceAdmission {
            all_admitted,
            timeout_diagnostics,
        })
    }

    pub(crate) fn drain_completions(
        &mut self,
        export_runtime: &ExportRuntime,
        storage: &mut dyn StorageBackend,
    ) -> Result<Vec<PostTraceOutcome>, ControlError> {
        let completions = export_runtime.drain_post_trace_completions();
        let mut outcomes = Vec::with_capacity(completions.len());
        for completion in completions {
            let key = TaskKey {
                trace_id: completion.trace_id,
                instance_id: completion.instance_id.clone(),
            };
            let lease = self.running.get(&key).cloned().ok_or_else(|| {
                ControlError::new(
                    "post_trace_completion",
                    format!(
                        "completion for untracked task trace={} instance={}",
                        completion.trace_id, completion.instance_id
                    ),
                )
            })?;
            storage
                .release_trace_lease(lease)
                .map_err(|error| ControlError::new(error.stage, error.message))?;
            self.running.remove(&key);
            self.diagnosed_drain_timeout.remove(&key);
            outcomes.push(PostTraceOutcome {
                trace_id: completion.trace_id,
                instance_id: completion.instance_id,
                result: completion.result,
            });
        }
        Ok(outcomes)
    }

    pub(crate) fn mark_trace_finalized(&mut self, trace_id: TraceId) {
        self.barrier_ready.remove(&trace_id);
        self.admitted.retain(|key| key.trace_id != trace_id);
        self.pending_since.retain(|key, _| key.trace_id != trace_id);
        self.diagnosed_admission_timeout
            .retain(|key| key.trace_id != trace_id);
    }

    pub(crate) fn forget_instance(&mut self, instance_id: &str) -> Result<(), ControlError> {
        if self.has_running_tasks_for(instance_id) {
            return Err(ControlError::new(
                "post_trace_unload",
                format!("plugin instance {instance_id} still has running post-trace tasks"),
            ));
        }
        self.admitted.retain(|key| key.instance_id != instance_id);
        self.pending_since
            .retain(|key, _| key.instance_id != instance_id);
        self.diagnosed_admission_timeout
            .retain(|key| key.instance_id != instance_id);
        self.diagnosed_drain_timeout
            .retain(|key| key.instance_id != instance_id);
        Ok(())
    }

    pub(crate) fn diagnose_drain_timeout(
        &mut self,
        instance_id: Option<&str>,
    ) -> Vec<PostTraceIssue> {
        let timeout_ms = self.shutdown_drain_timeout.as_millis();
        let timed_out = self
            .running
            .keys()
            .filter(|key| instance_id.is_none_or(|target| key.instance_id == target))
            .cloned()
            .collect::<Vec<_>>();
        timed_out
            .into_iter()
            .filter(|key| self.diagnosed_drain_timeout.insert(key.clone()))
            .map(|key| PostTraceIssue {
                trace_id: key.trace_id,
                instance_id: key.instance_id,
                code: "post_trace_drain_timeout".to_string(),
                message: format!(
                    "post-trace task did not stop within configured drain timeout {timeout_ms}ms"
                ),
            })
            .collect()
    }

    fn enqueue_task(
        &mut self,
        key: &TaskKey,
        export_runtime: &ExportRuntime,
        storage: &mut dyn StorageBackend,
    ) -> Result<(), ControlError> {
        let lease = storage
            .acquire_trace_lease(key.trace_id, TraceLeasePurpose::PostTraceAnalysis)
            .map_err(|error| ControlError::new(error.stage, error.message))?;
        let enqueue = export_runtime.enqueue_post_trace(
            &key.instance_id,
            PostTraceTask {
                trace_id: key.trace_id,
                timeout_ms: self.execution_timeout_ms,
            },
        );
        if let Err(error) = enqueue {
            storage
                .release_trace_lease(lease)
                .map_err(|release| ControlError::new(release.stage, release.message))?;
            return Err(ControlError::new(error.code, error.message));
        }
        self.running.insert(key.clone(), lease);
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TaskKey {
    trace_id: TraceId,
    instance_id: String,
}
