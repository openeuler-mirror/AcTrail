use super::bulk_read::{BulkReadKey, BulkReadState};
use super::io_action::FileIoAction;
use crate::live::runtime::LiveSemanticActionOutput;
use config_core::daemon::{FileCollectionConfig, FileObservationConfig, FileRawEventRetention};
use model_core::event::{
    DomainEvent, EventPayload, FileIoDirection, FileIoTargetKind, FileSummaryPathState,
};
use model_core::ids::TraceId;
use semantic_action::FileObservationPath;
use std::collections::BTreeMap;
use std::time::SystemTime;

pub(super) struct FileSummaryProjector {
    config: FileObservationConfig,
    bulk_read: BTreeMap<BulkReadKey, BulkReadState>,
}
impl FileSummaryProjector {
    pub(super) fn new(config: FileObservationConfig) -> Self {
        Self {
            config,
            bulk_read: BTreeMap::new(),
        }
    }
    pub(super) fn collection(&self) -> &FileCollectionConfig {
        &self.config.collection
    }

    pub(super) fn observe(&mut self, event: &DomainEvent) -> LiveSemanticActionOutput {
        let EventPayload::File(payload) = &event.payload else {
            return LiveSemanticActionOutput::default();
        };
        let Some(summary) = &payload.io_summary else {
            return LiveSemanticActionOutput::default();
        };
        let tty = summary.target_kind == FileIoTargetKind::CharacterDevice;
        let retention = if tty {
            let direction_enabled = match summary.direction {
                FileIoDirection::Read => {
                    self.config.tty.matches_operation("read")
                        || self.config.tty.matches_operation("readv")
                }
                FileIoDirection::Write => {
                    self.config.tty.matches_operation("write")
                        || self.config.tty.matches_operation("writev")
                }
            };
            if !self.config.enabled
                || !direction_enabled
                || !payload
                    .path
                    .as_deref()
                    .is_some_and(|path| self.config.tty.matches_path(path))
            {
                return LiveSemanticActionOutput {
                    retain_event: false,
                    raw_event_consumed: true,
                    ..LiveSemanticActionOutput::default()
                };
            }
            self.config.tty.raw_event_retention
        } else {
            let demand = match summary.direction {
                FileIoDirection::Read => self.config.collection.read,
                FileIoDirection::Write => self.config.collection.write,
            };
            if !demand.enabled() || (summary.errno != 0 && !demand.errors) {
                return LiveSemanticActionOutput::default();
            }
            match summary.direction {
                FileIoDirection::Read => self.config.bulk_read.raw_event_retention,
                FileIoDirection::Write => FileRawEventRetention::Full,
            }
        };
        let mut output = LiveSemanticActionOutput {
            retain_event: Self::retains(retention, summary.errno),
            raw_event_consumed: true,
            ..LiveSemanticActionOutput::default()
        };
        if !tty
            && summary.direction == FileIoDirection::Read
            && self.config.enabled
            && self.config.bulk_read.enabled
            && summary.path_state == FileSummaryPathState::Resolved
        {
            if let Some(path) = payload.path.as_deref() {
                let key = BulkReadKey {
                    trace_id: event.envelope.trace_id,
                    process: event.envelope.process.clone(),
                };
                self.bulk_read
                    .entry(key)
                    .or_insert_with(|| {
                        BulkReadState::new(
                            event,
                            summary,
                            &self.config.bulk_read,
                            self.config.collection.read.counts,
                            self.config.collection.read.bytes,
                        )
                    })
                    .observe(event, summary, path);
                return output;
            }
        }
        let action = FileIoAction::from_summary(event, payload, summary, tty);
        if summary.path_state == FileSummaryPathState::Resolved {
            if let Some(path) = &payload.path {
                output.file_observation_paths.push(FileObservationPath {
                    trace_id: action.trace_id,
                    action_id: action.action_id.clone(),
                    path_order: 0,
                    path: path.clone(),
                });
            }
        }
        output.actions.push(action);
        output
    }
    fn retains(retention: FileRawEventRetention, errno: u32) -> bool {
        if errno == 0 {
            retention.retains_success()
        } else {
            retention.retains_error()
        }
    }
    pub(super) fn finish_batch(&mut self) -> LiveSemanticActionOutput {
        let mut output = LiveSemanticActionOutput::default();
        for (_, state) in std::mem::take(&mut self.bulk_read) {
            output.file_path_sets.extend(state.path_set_write());
            output.actions.push(state.action());
        }
        output
    }
    pub(super) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        _finished_at: SystemTime,
    ) -> LiveSemanticActionOutput {
        let mut output = LiveSemanticActionOutput::default();
        self.bulk_read.retain(|key, state| {
            if key.trace_id != trace_id {
                return true;
            }
            output.file_path_sets.extend(state.path_set_write());
            output.actions.push(state.action());
            false
        });
        output
    }
    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.bulk_read.retain(|key, _| key.trace_id != trace_id);
    }
}
