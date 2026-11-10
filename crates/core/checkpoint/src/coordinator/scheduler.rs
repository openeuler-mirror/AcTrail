//! Runs each context's jobs in order on a worker thread of its own, started when work arrives and
//! ended when the queue is empty, and keeps what the jobs of a context share: its backend and the
//! hold of a restore that did not succeed.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, MutexGuard};
use std::time::SystemTime;

use checkpoint::CheckpointBackend;

use super::jobs::records::PendingRecord;
use super::jobs::{disconnect, launch, release, restore, save};
use super::{CompletedJob, Completion, CoordinatorError, Hold, LaunchRequest, Shared, lock};
use crate::model::{Boundary, CheckpointId, OperationId, SnapshotContext};
use crate::ports::PausedProcess;

#[derive(Default)]
pub(crate) struct TraceState {
    jobs: VecDeque<Job>,
    running: bool,
    /// The backend connected and its initial checkpoint was recorded.
    pub(crate) initialized: bool,
    /// A completed backend result that must still be written to the catalog.
    pub(crate) pending_record: Option<PendingRecord>,
    /// `disconnect` was accepted or the Trace ended: the context takes no more saves or restores, and
    /// its LLM requests go on at once.
    pub(crate) disconnecting: bool,
    /// Set while a restore runs and kept when it does not succeed; a later successful restore or
    /// resume lets the context go on.
    pub(crate) held: Option<Held>,
    /// A restore of a running Trace waiting for its next LLM request boundary.
    pub(crate) waiting_restore: Option<OperationId>,
    /// Connected at launch and disconnected by the disconnect job. A running job holds it.
    backend: Option<CheckpointBackend>,
    /// The last disconnect failed and the backend is kept: another `disconnect` or the end of the
    /// Trace retries it.
    pub(crate) disconnect_failed: bool,
}

impl TraceState {
    pub(crate) fn busy(&self) -> bool {
        self.running
    }
}

/// What keeps a context from going on after a restore that did not succeed.
#[derive(Default)]
pub(crate) struct Held {
    /// Processes the restore stopped.
    pub(crate) processes: Vec<PausedProcess>,
    /// Agents kept waiting at LLM request boundaries.
    pub(crate) boundaries: Vec<Hold>,
}

pub(crate) enum Job {
    Launch(LaunchRequest),
    Save {
        operation: OperationId,
    },
    Boundary {
        boundary: Boundary,
        hold: Hold,
    },
    Restore {
        operation: OperationId,
        boundary: Option<(Boundary, Hold)>,
    },
    Release {
        checkpoint_id: CheckpointId,
        snapshot_id: Vec<u8>,
    },
    Disconnect {
        ended_at: Option<SystemTime>,
    },
    Resume,
}

impl Shared {
    pub(crate) fn traces(&self) -> MutexGuard<'_, BTreeMap<SnapshotContext, TraceState>> {
        lock(&self.traces)
    }

    pub(crate) fn enqueue(self: &Arc<Self>, context: &SnapshotContext, job: Job) {
        let mut traces = self.traces();
        let state = traces.entry(context.clone()).or_default();
        state.jobs.push_back(job);
        if state.running {
            return;
        }
        state.running = true;
        drop(traces);
        let shared = Arc::clone(self);
        let context = context.clone();
        std::thread::Builder::new()
            .name(format!("checkpoint-{}", context.trace_id.get()))
            .spawn(move || shared.run_jobs(&context))
            .expect("spawn a checkpoint worker thread");
    }

    pub(crate) fn set_backend(&self, context: &SnapshotContext, backend: CheckpointBackend) {
        self.traces()
            .get_mut(context)
            .expect("queued contexts have state")
            .backend = Some(backend);
    }

    /// Keeps a backend whose disconnect failed, for a retry to free what is left.
    pub(crate) fn keep_for_retry(&self, context: &SnapshotContext, backend: CheckpointBackend) {
        let mut traces = self.traces();
        let state = traces.get_mut(context).expect("queued contexts have state");
        state.backend = Some(backend);
        state.disconnect_failed = true;
    }

    pub(crate) fn connected(&self, context: &SnapshotContext) -> bool {
        self.traces()
            .get(context)
            .is_some_and(|state| state.backend.is_some())
    }

    /// Jobs run only while the context is connected, and one at a time.
    pub(crate) fn take_backend(&self, context: &SnapshotContext) -> CheckpointBackend {
        self.traces()
            .get_mut(context)
            .and_then(|state| state.backend.take())
            .expect("a connected context has its backend")
    }

    pub(crate) fn take_held(&self, context: &SnapshotContext) -> Held {
        self.traces()
            .get_mut(context)
            .and_then(|state| state.held.take())
            .unwrap_or_default()
    }

    /// Holds the context with these processes stopped and these Agents waiting.
    pub(crate) fn hold(&self, context: &SnapshotContext, held: Held) {
        let mut traces = self.traces();
        let current = traces
            .entry(context.clone())
            .or_default()
            .held
            .get_or_insert_default();
        current.processes.extend(held.processes);
        current.processes.sort();
        current.processes.dedup();
        current.boundaries.extend(held.boundaries);
    }

    /// The Agent may go on unless a restore that did not succeed holds the context.
    fn release_unless_held(&self, context: &SnapshotContext, hold: Hold) -> Vec<Hold> {
        let mut traces = self.traces();
        match traces
            .get_mut(context)
            .and_then(|state| state.held.as_mut())
        {
            Some(held) => {
                held.boundaries.push(hold);
                Vec::new()
            }
            None => vec![hold],
        }
    }

    fn run_jobs(&self, context: &SnapshotContext) {
        loop {
            let job = {
                let mut traces = self.traces();
                let state = traces.get_mut(context).expect("queued contexts have state");
                match state.jobs.pop_front() {
                    Some(job) => job,
                    None => {
                        state.running = false;
                        return;
                    }
                }
            };
            let (job, result, released) = self.run(context, job);
            self.sink.completed(Completion {
                context: context.clone(),
                job,
                result,
                released,
            });
        }
    }

    fn run(
        &self,
        context: &SnapshotContext,
        job: Job,
    ) -> (CompletedJob, Result<(), CoordinatorError>, Vec<Hold>) {
        match job {
            Job::Launch(request) => {
                let result = launch::run(self, request);
                self.traces()
                    .get_mut(context)
                    .expect("queued contexts have state")
                    .initialized = result.is_ok();
                (CompletedJob::Launch, result, Vec::new())
            }
            Job::Save { operation } => (
                CompletedJob::Operation(operation),
                save::run(self, context, operation, None).map(drop),
                Vec::new(),
            ),
            Job::Boundary { boundary, hold } => (
                CompletedJob::Boundary,
                save::run_boundary(self, context, &boundary),
                self.release_unless_held(context, hold),
            ),
            Job::Restore {
                operation,
                boundary,
            } => {
                let (boundary, hold) = boundary.unzip();
                let (result, released) =
                    match restore::run(self, context, operation, boundary.as_ref()) {
                        Ok(Some(mut released)) => {
                            released.extend(hold);
                            (Ok(()), released)
                        }
                        // The context stays held, with this Agent waiting as well.
                        other => {
                            self.hold(
                                context,
                                Held {
                                    processes: Vec::new(),
                                    boundaries: hold.into_iter().collect(),
                                },
                            );
                            (other.map(drop), Vec::new())
                        }
                    };
                (CompletedJob::Operation(operation), result, released)
            }
            Job::Release {
                checkpoint_id,
                snapshot_id,
            } => (
                CompletedJob::Release(checkpoint_id),
                release::run(self, context, checkpoint_id, snapshot_id),
                Vec::new(),
            ),
            Job::Disconnect { ended_at } => {
                self.traces()
                    .get_mut(context)
                    .expect("queued contexts have state")
                    .disconnect_failed = false;
                match disconnect::run(self, context, ended_at) {
                    Ok(released) => (CompletedJob::Disconnect, Ok(()), released),
                    Err(error) => {
                        self.traces()
                            .get_mut(context)
                            .expect("queued contexts have state")
                            .disconnect_failed = true;
                        (CompletedJob::Disconnect, Err(error), Vec::new())
                    }
                }
            }
            Job::Resume => {
                let recorded = self.persist_pending(context);
                match restore::resume(self, context) {
                    Ok(released) => (CompletedJob::Resume, recorded, released),
                    Err(error) => (CompletedJob::Resume, Err(error), Vec::new()),
                }
            }
        }
    }
}
