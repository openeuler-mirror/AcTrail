use std::collections::BTreeMap;

use model_core::event::DomainEvent;
use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;

use super::{event_result, event_source_fd, event_target_fd};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::live::file) enum FileFdOwner {
    FsEnumerate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::live::file) struct FileFdState {
    pub(in crate::live::file) owner: FileFdOwner,
    pub(in crate::live::file) path: String,
}

#[derive(Default)]
pub(in crate::live::file) struct FileFdRegistry {
    states: BTreeMap<FileFdKey, FileFdState>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct FileFdKey {
    trace_id: TraceId,
    process: ProcessIdentity,
    fd: u32,
}

impl FileFdRegistry {
    pub(in crate::live::file) fn insert(
        &mut self,
        event: &DomainEvent,
        fd: u32,
        owner: FileFdOwner,
        path: String,
    ) {
        self.states
            .insert(FileFdKey::new(event, fd), FileFdState { owner, path });
    }

    pub(in crate::live::file) fn close_state(
        &mut self,
        event: &DomainEvent,
        fd: u32,
    ) -> Option<FileFdState> {
        let key = FileFdKey::new(event, fd);
        if event_result(event).is_some_and(|result| result < 0) {
            return self.states.get(&key).cloned();
        }
        self.states.remove(&key)
    }

    pub(in crate::live::file) fn duplicate(&mut self, event: &DomainEvent) {
        if event_result(event).is_some_and(|result| result < 0) {
            return;
        }
        let Some(source_fd) = event_source_fd(event) else {
            return;
        };
        let Some(target_fd) = event_target_fd(event) else {
            return;
        };
        let source_key = FileFdKey::new(event, source_fd);
        let target_key = FileFdKey::new(event, target_fd);
        let Some(state) = self.states.get(&source_key).cloned() else {
            if source_fd != target_fd {
                self.states.remove(&target_key);
            }
            return;
        };
        self.states.insert(target_key, state);
    }

    pub(in crate::live::file) fn forget_trace(&mut self, trace_id: TraceId) {
        self.states.retain(|key, _| key.trace_id != trace_id);
    }
}

impl FileFdKey {
    fn new(event: &DomainEvent, fd: u32) -> Self {
        Self {
            trace_id: event.envelope.trace_id,
            process: event.envelope.process.clone(),
            fd,
        }
    }
}
