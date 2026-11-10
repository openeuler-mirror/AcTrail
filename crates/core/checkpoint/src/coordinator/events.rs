//! Events: what the daemon observed about a running context. An event cannot be refused; the
//! coordinator decides how to respond, queues the work and returns at once.

use std::time::{Duration, Instant, SystemTime};

use super::scheduler::{Job, TraceState};
use super::{CheckpointCoordinator, CoordinatorError, Hold};
use crate::model::{Boundary, SnapshotContext};

/// What happens to the Agent waiting at an LLM request boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryAction {
    /// The context is disconnecting, so nothing is saved and it goes on now.
    Release,
    /// Its hold comes back in a completion, once the work it waits for is done.
    Wait,
}

impl CheckpointCoordinator {
    /// Decides what an LLM request of a running context triggers: a waiting restore, otherwise a
    /// save. While a restore that did not succeed holds the context, the Agent keeps waiting.
    pub fn at_boundary(
        &self,
        context: &SnapshotContext,
        boundary: Boundary,
        hold: Hold,
    ) -> Result<BoundaryAction, CoordinatorError> {
        let mut traces = self.shared.traces();
        let state = traces.entry(context.clone()).or_default();
        if let Some(operation) = state.waiting_restore.take() {
            drop(traces);
            self.shared.enqueue(
                context,
                Job::Restore {
                    operation,
                    boundary: Some((boundary, hold)),
                },
            );
            return Ok(BoundaryAction::Wait);
        }
        if let Some(held) = state.held.as_mut() {
            held.boundaries.push(hold);
            return Ok(BoundaryAction::Wait);
        }
        if state.disconnecting {
            return Ok(BoundaryAction::Release);
        }
        drop(traces);
        self.shared
            .enqueue(context, Job::Boundary { boundary, hold });
        Ok(BoundaryAction::Wait)
    }

    /// The daemon is stopping and takes the backend instances with it: disconnects the context
    /// once the work queued before is done, so its snapshots do not outlive the daemon. A restore
    /// that still waits for a boundary never runs; the next daemon records it as failed.
    pub fn daemon_stopping(&self, context: &SnapshotContext) {
        {
            let mut traces = self.shared.traces();
            let state = traces.entry(context.clone()).or_default();
            state.waiting_restore = None;
            state.disconnecting = true;
        }
        // Queue even when another disconnect is running: it may still fail after this event.
        self.shared.enqueue(
            context,
            Job::Disconnect {
                ended_at: Some(SystemTime::now()),
            },
        );
    }

    /// Waits until every worker has run out of work, and tells whether that took less than
    /// `timeout`. Worker threads die with the process, so work still running then is cut short.
    pub fn wait_idle(&self, timeout: Duration, poll_interval: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !self.shared.traces().values().any(TraceState::busy) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(
                poll_interval.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    /// Records the end of the Trace, runs a restore that was waiting for a boundary and then
    /// disconnects, which frees its snapshots: a Trace's checkpoints do not outlive it. A disconnect
    /// that failed while the Trace ran is retried, since nothing uses the backend any more.
    pub fn trace_ended(
        &self,
        context: &SnapshotContext,
        ended_at: SystemTime,
    ) -> Result<(), CoordinatorError> {
        self.shared.persist_pending(context)?;
        self.shared
            .catalog()
            .mark_trace_ended(context.trace_id, ended_at)?;
        let waiting = {
            let mut traces = self.shared.traces();
            let state = traces.entry(context.clone()).or_default();
            state.disconnecting = true;
            state.waiting_restore.take()
        };
        if let Some(operation) = waiting {
            self.shared.enqueue(
                context,
                Job::Restore {
                    operation,
                    boundary: None,
                },
            );
        }
        self.shared
            .enqueue(context, Job::Disconnect { ended_at: None });
        Ok(())
    }
}
