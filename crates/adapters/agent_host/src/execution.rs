//! Execution state supplied by agent integrations to optional consumers.
use std::collections::HashMap;
use std::time::{Instant, SystemTime};

use agent_lifecycle_contract::{
    TurnLifecycleKind, UserInteractionState, UserLifecycleEvent, WorkKind, WorkState,
};
use model_core::ids::TraceId;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ExecutionKey {
    pub trace_id: TraceId,
    pub session_id: String,
    pub task_id: String,
}

pub struct ExecutionState {
    models: u32,
    tools: u32,
    users: u32,
    unknown: bool,
    candidate_since: Option<(Instant, SystemTime)>,
}

impl ExecutionState {
    fn new() -> Self {
        Self {
            models: 0,
            tools: 0,
            users: 0,
            unknown: false,
            candidate_since: Some((Instant::now(), SystemTime::now())),
        }
    }

    pub fn candidate_since(&self) -> Option<(Instant, SystemTime)> {
        self.candidate_since
    }

    pub fn is_unknown(&self) -> bool {
        self.unknown
    }

    fn update(&mut self, kind: Option<WorkKind>, started: bool) -> Result<(), String> {
        if self.unknown {
            return Ok(());
        }
        let counter = match kind {
            Some(WorkKind::Model) => &mut self.models,
            Some(WorkKind::Tool) => &mut self.tools,
            None => &mut self.users,
        };
        let next = if started {
            counter.checked_add(1)
        } else {
            counter.checked_sub(1)
        };
        let Some(next) = next else {
            self.unknown = true;
            self.candidate_since = None;
            return Err(
                "agent execution counter is inconsistent; monitoring suspended for this task"
                    .to_owned(),
            );
        };
        *counter = next;
        self.candidate_since = if self.models == 0 && self.tools == 0 && self.users == 0 {
            Some((Instant::now(), SystemTime::now()))
        } else {
            None
        };
        Ok(())
    }
}

/// One authoritative map, updated on the serialized lifecycle control path.
pub struct ExecutionStates {
    enabled: bool,
    executions: HashMap<ExecutionKey, ExecutionState>,
}

impl ExecutionStates {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            executions: HashMap::new(),
        }
    }

    pub fn observe(&mut self, event: &UserLifecycleEvent) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        event.validate()?;
        let task = match event {
            UserLifecycleEvent::Turn(event) => &event.task_id,
            UserLifecycleEvent::Interaction(event) => &event.task_id,
            UserLifecycleEvent::Work(event) => &event.task_id,
            UserLifecycleEvent::SessionClosed(event) => {
                self.executions.retain(|key, _| {
                    key.trace_id != event.trace_id || key.session_id != event.session_id
                });
                return Ok(());
            }
        };
        let key = ExecutionKey {
            trace_id: event.trace_id(),
            session_id: event.session_id().to_owned(),
            task_id: task.clone(),
        };
        match event {
            UserLifecycleEvent::Turn(event) => match event.kind {
                TurnLifecycleKind::Started => {
                    self.executions
                        .entry(key)
                        .or_insert_with(ExecutionState::new);
                }
                TurnLifecycleKind::Completed => {
                    self.executions.remove(&key);
                }
            },
            UserLifecycleEvent::Work(event) => {
                if let Some(state) = self.executions.get_mut(&key) {
                    state.update(Some(event.kind), event.state == WorkState::Started)?;
                }
            }
            UserLifecycleEvent::Interaction(event) => {
                if let Some(state) = self.executions.get_mut(&key) {
                    state.update(None, event.state == UserInteractionState::Requested)?;
                }
            }
            UserLifecycleEvent::SessionClosed(_) => unreachable!(),
        }
        Ok(())
    }

    pub fn finish_trace(&mut self, trace_id: TraceId) {
        if self.enabled {
            self.executions.retain(|key, _| key.trace_id != trace_id);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ExecutionKey, &ExecutionState)> {
        self.executions.iter()
    }

    pub fn get(&self, key: &ExecutionKey) -> Option<&ExecutionState> {
        self.executions.get(key)
    }
}
