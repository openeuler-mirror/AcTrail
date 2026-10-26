//! Converts kernel cumulative snapshots to independently consumable deltas.

use std::collections::{HashMap, HashSet};

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::event::{FileIoDirection, FileIoSummary, FileIoTargetKind, FileSummaryPathState};
use model_core::ids::{CollectorName, TraceId};

use crate::EbpfCollector;
use crate::decode::{resolve_bound_event_observation, wall_from_ktime};
use crate::loader::{FileIoKey, FileIoSnapshot};

pub(crate) type FileIoProcess = (TraceId, u32, u64);

#[derive(Default)]
pub(super) struct FileIoSummaryCollector {
    // The kernel is authoritative. These checkpoints only prevent re-delivering
    // the same cumulative counters, and expire after final delivery.
    delivered: HashMap<FileIoKey, Checkpoint>,
    pub(super) backlog: Vec<RawCollectorEvent>,
}

struct Checkpoint {
    sequence: u64,
    count: u64,
    bytes: u64,
    end_ns: u64,
}

impl FileIoSummaryCollector {
    fn delta(&self, snapshot: &FileIoSnapshot, complete: bool) -> Option<FileIoSummary> {
        let previous = self.delivered.get(&snapshot.key);
        if snapshot.sequence == 0 || previous.is_some_and(|p| p.sequence == snapshot.sequence) {
            return None;
        }
        let direction = match snapshot.key.direction {
            1 => FileIoDirection::Read,
            2 => FileIoDirection::Write,
            _ => return None,
        };
        let target_kind = match snapshot.target_kind {
            1 => FileIoTargetKind::RegularFile,
            2 => FileIoTargetKind::CharacterDevice,
            _ => return None,
        };
        Some(FileIoSummary {
            file_token: snapshot.key.token,
            direction,
            target_kind,
            errno: snapshot.key.error_result.unsigned_abs(),
            interval_start: wall_from_ktime(previous.map_or(snapshot.first_ns, |p| p.end_ns)),
            interval_end: wall_from_ktime(snapshot.last_ns),
            interval_complete: complete && snapshot.coverage_complete,
            operations: (snapshot.flags & 2 != 0).then(|| {
                snapshot
                    .count
                    .saturating_sub(previous.map_or(0, |p| p.count))
            }),
            bytes: (snapshot.flags & 4 != 0 && snapshot.key.error_result == 0).then(|| {
                snapshot
                    .bytes
                    .saturating_sub(previous.map_or(0, |p| p.bytes))
            }),
            path_state: match snapshot.path_status {
                1 => FileSummaryPathState::Resolved,
                2 => FileSummaryPathState::Truncated,
                _ => FileSummaryPathState::Unavailable,
            },
        })
    }

    fn commit(&mut self, snapshot: &FileIoSnapshot) {
        self.delivered.insert(
            snapshot.key,
            Checkpoint {
                sequence: snapshot.sequence,
                count: snapshot.count,
                bytes: snapshot.bytes,
                end_ns: snapshot.last_ns,
            },
        );
    }
}

impl EbpfCollector {
    pub(super) fn collect_file_io_summaries(
        &mut self,
        exited: &HashSet<FileIoProcess>,
        cutoff: Option<FileIoProcess>,
        cutoff_trace: Option<TraceId>,
        output: &mut Vec<RawCollectorEvent>,
    ) {
        let Some(runtime) = self.runtime.as_mut() else {
            return;
        };
        let snapshots = runtime
            .file_io_snapshots(!exited.is_empty() || cutoff.is_some() || cutoff_trace.is_some());
        for mut snapshot in snapshots {
            let process = (
                snapshot.key.trace_id,
                snapshot.key.pid,
                snapshot.key.generation,
            );
            if cutoff.is_some_and(|selected| selected != process) {
                continue;
            }
            if cutoff_trace.is_some_and(|trace| trace != process.0) {
                continue;
            }
            let retiring = exited.contains(&process)
                || cutoff == Some(process)
                || cutoff_trace == Some(process.0);
            if let Some(summary) = self
                .file_io_summaries
                .delta(&snapshot, cutoff.is_none() && cutoff_trace.is_none())
            {
                let identity = match resolve_bound_event_observation(
                    snapshot.key.trace_id,
                    snapshot.key.pid,
                    snapshot.key.generation,
                    &self.bindings,
                ) {
                    Ok(identity) => identity,
                    Err(_) => {
                        self.binding_gap_drops = self.binding_gap_drops.saturating_add(1);
                        continue;
                    }
                };
                output.push(RawCollectorEvent {
                    envelope: RawEventEnvelope {
                        trace_id: Some(snapshot.key.trace_id),
                        observed_at: summary.interval_end,
                        process: identity,
                        collector: CollectorName::new("ebpf"),
                    },
                    payload: RawObservationPayload::FileIoSummary {
                        path: snapshot.path.take(),
                        summary,
                    },
                });
                self.file_io_summaries.commit(&snapshot);
            }
            if retiring
                && self
                    .runtime
                    .as_mut()
                    .is_some_and(|r| r.remove_file_io_summary(snapshot.key))
            {
                self.file_io_summaries.delivered.remove(&snapshot.key);
            }
        }
    }

    pub(super) fn finish_file_io_trace(&mut self, trace: TraceId) {
        let mut events = Vec::new();
        self.collect_file_io_summaries(&HashSet::new(), None, Some(trace), &mut events);
        self.file_io_summaries.backlog.extend(events);
    }

    pub fn take_pending_file_io_events(&mut self) -> Vec<RawCollectorEvent> {
        std::mem::take(&mut self.file_io_summaries.backlog)
    }

    pub fn has_pending_file_io_events(&self) -> bool {
        !self.file_io_summaries.backlog.is_empty()
    }

    pub(super) fn finish_file_io_process(&mut self, process: FileIoProcess) {
        let mut events = Vec::new();
        self.collect_file_io_summaries(&HashSet::new(), Some(process), None, &mut events);
        self.file_io_summaries.backlog.extend(events);
    }
}
