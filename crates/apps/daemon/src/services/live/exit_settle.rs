//! Exit handshake: a launcher's `TrackRemove` reply is held until its trace settled.
//!
//! A launcher that is the main process of a sandbox takes the sandbox down when it
//! exits. The daemon still has to finalize the trace and hand the last records to
//! the export sinks, so the launcher asks to be answered only after that.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use control_contract::command::TrackRemoveSettle;
use control_contract::reply::TrackSettleReply;
use model_core::ids::TraceId;
use plugin_system::ObservationFlush;

use crate::services::attach::StorageAttachService;

/// Part of the deadline kept for the export flush when finalization is slow.
const EXPORT_FLUSH_RESERVE: Duration = Duration::from_millis(2_000);

struct ExitSettle {
    ticket: u64,
    /// The reply is sent by this instant whatever the progress.
    deadline: Instant,
    /// Finalization is awaited until this instant; then the export flush starts.
    finalize_until: Instant,
    finalized: bool,
    /// `Some` once the export flush has been started.
    flushes: Option<Vec<(String, ObservationFlush)>>,
    had_flush_consumer: bool,
    flushed_records: u64,
    flush_errors: Vec<String>,
}

#[derive(Default)]
pub(in crate::services) struct ExitSettles {
    pending: BTreeMap<TraceId, ExitSettle>,
    finished: Vec<(u64, TrackSettleReply)>,
}

impl ExitSettles {
    pub(in crate::services) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub(in crate::services) fn take_finished(&mut self) -> Vec<(u64, TrackSettleReply)> {
        std::mem::take(&mut self.finished)
    }
}

impl ExitSettle {
    fn reply(&self, flush_timed_out: bool) -> TrackSettleReply {
        let export_state = if !self.had_flush_consumer {
            "no_consumer"
        } else if flush_timed_out {
            "timeout"
        } else if !self.flush_errors.is_empty() {
            "failed"
        } else {
            "flushed"
        };
        let mut detail = Vec::new();
        if !self.finalized {
            detail.push("trace was not finalized before the deadline".to_string());
        }
        if flush_timed_out {
            detail.push("export flush did not complete before the deadline".to_string());
        }
        detail.extend(self.flush_errors.iter().cloned());
        TrackSettleReply {
            finalized: self.finalized,
            export_state: export_state.to_string(),
            flushed_records: self.flushed_records,
            timed_out: !self.finalized || flush_timed_out,
            detail: detail.join("; "),
        }
    }
}

impl StorageAttachService {
    pub(in crate::services) fn begin_exit_settle_impl(
        &mut self,
        trace_id: TraceId,
        ticket: u64,
        settle: TrackRemoveSettle,
    ) {
        let now = Instant::now();
        let total = Duration::from_millis(u64::from(settle.deadline_ms));
        let reserve = EXPORT_FLUSH_RESERVE.min(total / 2);
        let deadline = now + total;
        let replaced = self.exit_settles.pending.insert(
            trace_id,
            ExitSettle {
                ticket,
                deadline,
                finalize_until: deadline - reserve,
                finalized: false,
                flushes: None,
                had_flush_consumer: false,
                flushed_records: 0,
                flush_errors: Vec::new(),
            },
        );
        if let Some(replaced) = replaced {
            // One trace has one launcher; answer an earlier request instead of leaving it waiting.
            self.exit_settles
                .finished
                .push((replaced.ticket, replaced.reply(false)));
        }
    }

    /// Advances every pending handshake. Called once per drain cycle, after the
    /// terminal finalizations of that cycle have run.
    pub(in crate::services) fn progress_exit_settles_impl(&mut self) {
        if self.exit_settles.pending.is_empty() {
            return;
        }
        let now = Instant::now();
        let trace_ids = self
            .exit_settles
            .pending
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for trace_id in trace_ids {
            let finalized = self.finalized_terminal_traces.contains(&trace_id);
            let Some(settle) = self.exit_settles.pending.get_mut(&trace_id) else {
                continue;
            };
            if settle.flushes.is_none() {
                if finalized {
                    settle.finalized = true;
                } else if now < settle.finalize_until {
                    continue;
                }
                // The builtin consumers were given every record of this trace on this
                // thread, so a flush queued now follows all of them.
                settle.flushes = Some(
                    self.export_runtime
                        .flush_observation_consumers(settle.deadline),
                );
                settle.had_flush_consumer = settle
                    .flushes
                    .as_ref()
                    .is_some_and(|flushes| !flushes.is_empty());
            }
            let mut running = settle.flushes.take().unwrap_or_default();
            running.retain_mut(|(instance, flush)| match flush.try_outcome() {
                Some(outcome) => {
                    settle.flushed_records = settle
                        .flushed_records
                        .saturating_add(outcome.flushed_records);
                    if let Some(error) = outcome.error {
                        settle.flush_errors.push(format!("{instance}: {error}"));
                    }
                    false
                }
                None => true,
            });
            let flush_timed_out = !running.is_empty() && now >= settle.deadline;
            if !running.is_empty() && !flush_timed_out {
                settle.flushes = Some(running);
                continue;
            }
            let reply = settle.reply(flush_timed_out);
            let ticket = settle.ticket;
            self.exit_settles.pending.remove(&trace_id);
            tracing::debug!(
                trace_id = %trace_id,
                finalized = reply.finalized,
                export = %reply.export_state,
                flushed_records = reply.flushed_records,
                timed_out = reply.timed_out,
                "exit settle answered"
            );
            self.exit_settles.finished.push((ticket, reply));
        }
    }
}
