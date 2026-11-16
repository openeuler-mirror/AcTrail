//! Owns the coordinator and the launches waiting for their initial checkpoint.

use std::collections::{BTreeMap, BTreeSet};
use std::os::fd::RawFd;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use checkpoint_core::model::{Boundary, SnapshotContext};
use checkpoint_core::ports::ProcessControlError;
use checkpoint_core::{
    BackendEntry, BoundaryAction, CheckpointCoordinator, CheckpointFactory, CompletedJob,
    CoordinatorError, LaunchRequest,
};
use checkpoint_sqlite::{CheckpointSqliteCatalog, CheckpointSqliteReader};
use config_core::daemon::{CheckpointBackendConfig, CheckpointRuntimeConfig};
use control_contract::reply::{ControlError, ControlReply, TrackAddReply};
use model_core::ids::TraceId;
use trace_runtime::TraceRuntime;
use uds_control_server::ReplyTicket;

use super::completions::{CompletionQueue, QueueSink};
use super::process_control::ProcfsProcessControl;
use crate::services::attach::HeldLlmRequest;

pub(crate) struct CheckpointService {
    pub(super) coordinator: CheckpointCoordinator,
    processes: ProcfsProcessControl,
    completions: Arc<CompletionQueue>,
    pending_launches: BTreeMap<TraceId, PendingLaunch>,
    finished: Vec<(ReplyTicket, Result<ControlReply, ControlError>)>,
    next_ticket: u64,
    /// Checkpoint Traces this daemon runs, watched until they end.
    pub(super) running: BTreeSet<TraceId>,
    shutdown_drain_timeout: Duration,
    idle_poll_interval: Duration,
}

/// What the attach service has to do after a drain.
#[derive(Default)]
pub(crate) struct Drained {
    /// Traces whose launch failed; the caller removes them.
    pub(crate) failed_launches: Vec<TraceId>,
    /// Traces watched from now on (`true`) or no longer.
    pub(crate) watch: Vec<(TraceId, bool)>,
    /// Holds of gated LLM responses that may reach the Agent.
    pub(crate) release: Vec<u64>,
}

struct PendingLaunch {
    ticket: ReplyTicket,
    reply: TrackAddReply,
}

impl CheckpointService {
    /// Opens the catalog, closes what an earlier daemon left behind and loads the integrations.
    /// Also returns the next Trace ID after the catalog's retained history.
    pub(crate) fn open(
        catalog_path: &Path,
        backends: &[CheckpointBackendConfig],
        config: CheckpointRuntimeConfig,
    ) -> Result<(Self, u64), ControlError> {
        let catalog = CheckpointSqliteCatalog::open(catalog_path).map_err(catalog_error)?;
        let reader = CheckpointSqliteReader::open(
            catalog_path,
            Duration::from_millis(config.busy_timeout_ms),
        )
        .map_err(catalog_error)?;
        let trace_id_seed = reader.next_trace_id_seed().map_err(catalog_error)?;
        let completions = CompletionQueue::new()?;
        let processes = ProcfsProcessControl::new(
            Duration::from_millis(config.stop_timeout_ms),
            Duration::from_millis(config.stop_poll_interval_ms),
        );
        let entries: Vec<BackendEntry> = backends
            .iter()
            .map(|backend| BackendEntry {
                name: backend.name.clone(),
                library: backend.library.clone(),
                config: backend.config.clone(),
            })
            .collect();
        let factory = CheckpointFactory::load(&entries).map_err(coordinator_error)?;
        let coordinator = CheckpointCoordinator::new(
            Box::new(catalog),
            Box::new(reader),
            factory,
            Box::new(processes.clone()),
            Box::new(QueueSink(Arc::clone(&completions))),
        );
        let recovered = coordinator.recover().map_err(coordinator_error)?;
        if recovered > 0 {
            tracing::warn!(
                recovered,
                "closed checkpoint operations an earlier daemon left unfinished"
            );
        }
        Ok((
            Self {
                coordinator,
                processes,
                completions,
                pending_launches: BTreeMap::new(),
                finished: Vec::new(),
                next_ticket: 0,
                running: BTreeSet::new(),
                shutdown_drain_timeout: Duration::from_millis(config.shutdown_drain_timeout_ms),
                idle_poll_interval: Duration::from_millis(config.idle_poll_interval_ms),
            },
            trace_id_seed,
        ))
    }

    /// Backend instances end with this process, so running Traces disconnect first and work in
    /// progress gets a chance to finish.
    pub(crate) fn shutdown(&self) {
        // A launch still in progress is disconnected once it finishes, so its initial snapshot
        // does not outlive the daemon either.
        for trace_id in self.running.iter().chain(self.pending_launches.keys()) {
            self.coordinator.daemon_stopping(&SnapshotContext {
                trace_id: *trace_id,
            });
        }
        if !self
            .coordinator
            .wait_idle(self.shutdown_drain_timeout, self.idle_poll_interval)
        {
            tracing::warn!("checkpoint work was still running when the daemon stopped");
        }
    }

    pub(crate) fn poll_fd(&self) -> RawFd {
        self.completions.wake_fd()
    }

    /// Keeps reparented members available as discovery roots when the Trace root exits.
    pub(crate) fn update_processes(
        &self,
        runtime: &TraceRuntime,
        resolve: &dyn Fn(
            model_core::process::ProcessIdentity,
        ) -> Result<model_core::process::ProcessRecord, ControlError>,
    ) {
        for trace_id in &self.running {
            let Some(entry) = runtime.get_trace(*trace_id) else {
                // The runtime already finalized it; a restore still waiting needs its last members.
                continue;
            };
            let members = entry
                .memberships
                .memberships()
                .filter(|member| {
                    matches!(
                        member.state,
                        model_core::process::MembershipState::Starting
                            | model_core::process::MembershipState::Active
                    )
                })
                .map(|member| {
                    resolve(member.identity)
                        .map_err(|error| ProcessControlError::new(error.message))
                })
                .collect();
            self.processes.set_members(*trace_id, members);
        }
    }

    /// Starts connecting and the initial save of an attached Trace; `TrackAdd` is answered when
    /// they finish. On error the caller removes the Trace again.
    pub(crate) fn start_launch(
        &mut self,
        request: LaunchRequest,
        reply: TrackAddReply,
    ) -> Result<ReplyTicket, ControlError> {
        let trace_id = request.context.trace_id;
        self.coordinator
            .launch(request)
            .map_err(coordinator_error)?;
        self.next_ticket += 1;
        let ticket = ReplyTicket::new(self.next_ticket);
        self.pending_launches
            .insert(trace_id, PendingLaunch { ticket, reply });
        Ok(ticket)
    }

    /// Applies finished work, decides LLM requests and notices checkpoint Traces that ended.
    pub(crate) fn drain(
        &mut self,
        trace_runtime: &TraceRuntime,
        llm_requests: Vec<HeldLlmRequest>,
    ) -> Drained {
        let mut drained = Drained::default();
        for completion in self.completions.take() {
            let trace_id = completion.context.trace_id;
            drained.release.extend(completion.released);
            match (completion.job, completion.result) {
                (CompletedJob::Launch, result) => {
                    let Some(pending) = self.pending_launches.remove(&trace_id) else {
                        continue;
                    };
                    let reply = match result {
                        Ok(()) => {
                            self.running.insert(trace_id);
                            drained.watch.push((trace_id, true));
                            Ok(ControlReply::TrackAdded(pending.reply))
                        }
                        Err(error) => {
                            drained.failed_launches.push(trace_id);
                            Err(coordinator_error(error))
                        }
                    };
                    self.finished.push((pending.ticket, reply));
                }
                // A disconnected Trace that still runs no longer has its LLM requests held.
                (CompletedJob::Disconnect, Ok(())) if self.running.contains(&trace_id) => {
                    drained.watch.push((trace_id, false));
                }
                // Nothing restores this Trace any more.
                (CompletedJob::Disconnect, Ok(())) => self.processes.forget(trace_id),
                (_, Ok(())) => {}
                (job, Err(error)) => {
                    tracing::warn!(
                        %trace_id,
                        ?job,
                        code = %error.code,
                        message = %error.message,
                        "checkpoint job failed"
                    );
                }
            }
        }
        for request in llm_requests {
            if let Some(hold) = self.at_boundary(request) {
                drained.release.push(hold);
            }
        }
        // A Trace leaves the runtime only after it reached a terminal state.
        let ended: Vec<TraceId> = self
            .running
            .iter()
            .copied()
            .filter(|trace_id| {
                trace_runtime
                    .get_trace(*trace_id)
                    .is_none_or(|entry| entry.trace.lifecycle_state.is_terminal())
            })
            .collect();
        for trace_id in ended {
            match self
                .coordinator
                .trace_ended(&SnapshotContext { trace_id }, SystemTime::now())
            {
                Ok(()) => {
                    self.running.remove(&trace_id);
                    drained.watch.push((trace_id, false));
                }
                Err(error) => {
                    tracing::warn!(%trace_id, %error, "recording the end of a checkpoint Trace failed");
                }
            }
        }
        drained
    }

    pub(crate) fn take_finished_replies(
        &mut self,
    ) -> Vec<(ReplyTicket, Result<ControlReply, ControlError>)> {
        std::mem::take(&mut self.finished)
    }
}

impl CheckpointService {
    /// Saves, or restores when a restore waits. Returns the hold when the Agent may go on now;
    /// otherwise it comes back in a completion.
    fn at_boundary(&self, request: HeldLlmRequest) -> Option<u64> {
        let trace_id = request.trace_id;
        if !self.running.contains(&trace_id) {
            return Some(request.hold);
        }
        let at = Boundary {
            observed_at: SystemTime::now(),
            request_action_id: request.request_action_id,
        };
        match self
            .coordinator
            .at_boundary(&SnapshotContext { trace_id }, at, request.hold)
        {
            Ok(BoundaryAction::Wait) => None,
            Ok(BoundaryAction::Release) => Some(request.hold),
            Err(error) => {
                tracing::warn!(
                    %trace_id,
                    %error,
                    "checkpoint work at an LLM request boundary could not start"
                );
                Some(request.hold)
            }
        }
    }
}

pub(super) fn coordinator_error(error: CoordinatorError) -> ControlError {
    ControlError::new(error.code, error.message)
}

fn catalog_error(error: checkpoint_core::ports::CatalogError) -> ControlError {
    ControlError::new("checkpoint_catalog", error.message)
}
