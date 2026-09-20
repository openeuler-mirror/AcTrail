use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::container::ContainerRuntime;
use model_core::external_cgroup::{
    ExternalBindingStaleReason, ExternalBindingState, ExternalCgroupBinding,
};
use model_core::ids::{EventId, TraceId};
use model_core::resource_scope::{ResourceScopeLifecycleState, TraceResourceScope};
use storage_core::StorageError;

#[derive(Clone, Default)]
struct ControlRecords {
    traces: BTreeSet<TraceId>,
    scopes: BTreeMap<TraceId, TraceResourceScope>,
    bindings: BTreeMap<TraceId, ExternalCgroupBinding>,
}

/// Sole in-memory resource registry; observations never enter this state.
#[derive(Default)]
pub(super) struct ResourceState {
    records: ControlRecords,
    transaction_open: bool,
    before_write: Option<ControlRecords>,
}

impl ResourceState {
    fn error() -> StorageError {
        StorageError::new(
            "noop_resource_state",
            "missing or invalid resource control state",
        )
    }

    pub fn begin(&mut self) -> Result<(), StorageError> {
        if self.transaction_open {
            return Err(Self::error());
        }
        self.transaction_open = true;
        Ok(())
    }

    pub fn finish(&mut self, commit: bool) {
        if let Some(previous) = self.before_write.take() {
            if !commit {
                self.records = previous;
            }
        }
        self.transaction_open = false;
    }

    fn write(&mut self) -> &mut ControlRecords {
        // Ordinary observation-only transactions allocate no snapshot.
        if self.transaction_open && self.before_write.is_none() {
            self.before_write = Some(self.records.clone());
        }
        &mut self.records
    }

    pub fn trace_created(&mut self, id: TraceId) {
        if !self.records.traces.contains(&id) {
            self.write().traces.insert(id);
        }
    }
    pub fn scope(&self, id: TraceId) -> Option<TraceResourceScope> {
        self.records.scopes.get(&id).cloned()
    }
    pub fn scopes(&self) -> Vec<TraceResourceScope> {
        self.records.scopes.values().cloned().collect()
    }
    pub fn binding(&self, id: TraceId) -> Option<ExternalCgroupBinding> {
        self.records.bindings.get(&id).cloned()
    }
    pub fn live_bindings(&self) -> Vec<ExternalCgroupBinding> {
        self.records
            .bindings
            .values()
            .filter(|b| b.lifecycle_state.is_live())
            .cloned()
            .collect()
    }

    pub fn create_scope(&mut self, scope: TraceResourceScope) -> Result<(), StorageError> {
        if let Some(current) = self.records.scopes.get(&scope.trace_id) {
            return if current == &scope {
                Ok(())
            } else {
                Err(Self::error())
            };
        }
        if scope.lifecycle_state.is_live() && scope.final_event_id.is_some() {
            return Err(Self::error());
        }
        if self
            .records
            .scopes
            .values()
            .any(|current| current.relative_path == scope.relative_path)
        {
            return Err(Self::error());
        }
        self.write().scopes.insert(scope.trace_id, scope);
        Ok(())
    }

    pub fn update_scope(
        &mut self,
        id: TraceId,
        state: ResourceScopeLifecycleState,
        event: Option<EventId>,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        if state.is_live() && event.is_some() {
            return Err(Self::error());
        }
        let scope = self.write().scopes.get_mut(&id).ok_or_else(Self::error)?;
        scope.lifecycle_state = state;
        scope.final_event_id = event;
        scope.updated_at = time;
        Ok(())
    }

    pub fn create_binding(&mut self, binding: ExternalCgroupBinding) -> Result<(), StorageError> {
        let valid = !matches!(binding.runtime, ContainerRuntime::Unknown)
            && match binding.lifecycle_state {
                ExternalBindingState::Active => {
                    binding.stale_reason.is_none()
                        && binding.closed_at.is_none()
                        && binding.final_event_id.is_none()
                }
                ExternalBindingState::Stale => {
                    binding.stale_reason.is_some()
                        && binding.closed_at.is_none()
                        && binding.final_event_id.is_none()
                }
                ExternalBindingState::Closed => {
                    binding.closed_at.is_some() && binding.final_event_id.is_some()
                }
            };
        if !valid {
            return Err(Self::error());
        }
        if let Some(current) = self.records.bindings.get(&binding.trace_id) {
            return if current == &binding {
                Ok(())
            } else {
                Err(Self::error())
            };
        }
        self.write().bindings.insert(binding.trace_id, binding);
        Ok(())
    }

    fn active(&mut self, id: TraceId) -> Result<&mut ExternalCgroupBinding, StorageError> {
        self.write()
            .bindings
            .get_mut(&id)
            .filter(|b| b.lifecycle_state == ExternalBindingState::Active)
            .ok_or_else(Self::error)
    }

    pub fn success(&mut self, id: TraceId, time: SystemTime) -> Result<(), StorageError> {
        let binding = self.active(id)?;
        binding.consecutive_failures = 0;
        binding.last_good_at = Some(time);
        binding.updated_at = time;
        Ok(())
    }

    pub fn failure(&mut self, id: TraceId, time: SystemTime) -> Result<u32, StorageError> {
        let binding = self.active(id)?;
        binding.consecutive_failures = binding
            .consecutive_failures
            .checked_add(1)
            .ok_or_else(Self::error)?;
        binding.updated_at = time;
        Ok(binding.consecutive_failures)
    }

    pub fn stale(
        &mut self,
        id: TraceId,
        reason: ExternalBindingStaleReason,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        if self.records.bindings.get(&id).is_some_and(|b| {
            b.lifecycle_state == ExternalBindingState::Stale && b.stale_reason == Some(reason)
        }) {
            return Ok(());
        }
        let binding = self.active(id)?;
        binding.lifecycle_state = ExternalBindingState::Stale;
        binding.stale_reason = Some(reason);
        binding.updated_at = time;
        Ok(())
    }

    pub fn discard_orphan(&mut self, id: TraceId) {
        if !self.records.traces.contains(&id)
            && self
                .records
                .bindings
                .get(&id)
                .is_some_and(|b| b.lifecycle_state.is_live())
        {
            self.write().bindings.remove(&id);
        }
    }

    pub fn close(
        &mut self,
        id: TraceId,
        event: EventId,
        time: SystemTime,
    ) -> Result<EventId, StorageError> {
        let current = self.records.bindings.get(&id).ok_or_else(Self::error)?;
        if current.lifecycle_state == ExternalBindingState::Closed {
            return current.final_event_id.ok_or_else(Self::error);
        }
        let binding = self.write().bindings.get_mut(&id).ok_or_else(Self::error)?;
        binding.lifecycle_state = ExternalBindingState::Closed;
        binding.closed_at = Some(time);
        binding.final_event_id = Some(event);
        binding.updated_at = time;
        Ok(event)
    }
}
