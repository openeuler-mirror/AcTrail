use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use model_core::container::ContainerRuntime;
use model_core::ids::TraceId;
use resource_profiling::model::cgroup_v2_binding::CgroupV2Binding;
use resource_profiling::model::cgroup_v2_binding::CgroupV2BindingStaleReason;
use resource_profiling::model::cgroup_v2_binding::CgroupV2BindingState;
use resource_profiling::model::resource_scope::ResourceScopeLifecycleState;
use resource_profiling::model::resource_scope::TraceResourceScope;
use storage_core::StorageError;

#[derive(Clone, Default)]
struct ControlRecords {
    traces: BTreeSet<TraceId>,
    scopes: BTreeMap<TraceId, TraceResourceScope>,
    bindings: BTreeMap<TraceId, CgroupV2Binding>,
    v1: BTreeMap<TraceId, resource_profiling::model::cgroup_v1_binding::CgroupV1Binding>,
}

/// Sole in-memory resource registry; observations never enter this state.
#[derive(Default)]
pub(super) struct ResourceState {
    records: ControlRecords,
    transaction_open: bool,
    before_write: Option<ControlRecords>,
}

impl ResourceState {
    pub(super) fn v1(
        &self,
        id: TraceId,
    ) -> Option<resource_profiling::model::cgroup_v1_binding::CgroupV1Binding> {
        self.records.v1.get(&id).cloned()
    }
    pub(super) fn live_v1(
        &self,
    ) -> Vec<resource_profiling::model::cgroup_v1_binding::CgroupV1Binding> {
        self.records
            .v1
            .values()
            .filter(|b| b.closed_at.is_none())
            .cloned()
            .collect()
    }
    pub(super) fn create_v1(
        &mut self,
        binding: resource_profiling::model::cgroup_v1_binding::CgroupV1Binding,
    ) -> Result<(), StorageError> {
        if let Some(current) = self.records.v1.get(&binding.trace_id) {
            return if *current == binding {
                Ok(())
            } else {
                Err(Self::error())
            };
        }
        if !self.records.traces.contains(&binding.trace_id) || binding.closed_at.is_some() {
            return Err(Self::error());
        }
        self.write().v1.insert(binding.trace_id, binding);
        Ok(())
    }
    pub(super) fn mark_v1_binding_stale(
        &mut self,
        id: TraceId,
        reason: &str,
    ) -> Result<(), StorageError> {
        let binding = self
            .write()
            .v1
            .get_mut(&id)
            .filter(|b| b.closed_at.is_none())
            .ok_or_else(Self::error)?;
        binding.stale_reason.get_or_insert_with(|| reason.into());
        Ok(())
    }

    pub(super) fn close_v1(&mut self, id: TraceId, at: SystemTime) -> Result<(), StorageError> {
        self.write()
            .v1
            .get_mut(&id)
            .ok_or_else(Self::error)?
            .closed_at
            .get_or_insert(at);
        Ok(())
    }

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
    pub fn binding(&self, id: TraceId) -> Option<CgroupV2Binding> {
        self.records.bindings.get(&id).cloned()
    }
    pub fn live_bindings(&self) -> Vec<CgroupV2Binding> {
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
        time: SystemTime,
    ) -> Result<(), StorageError> {
        let scope = self.write().scopes.get_mut(&id).ok_or_else(Self::error)?;
        scope.lifecycle_state = state;
        scope.updated_at = time;
        Ok(())
    }

    pub fn create_binding(&mut self, binding: CgroupV2Binding) -> Result<(), StorageError> {
        let valid = !matches!(binding.runtime, ContainerRuntime::Unknown)
            && match binding.lifecycle_state {
                CgroupV2BindingState::Active => {
                    binding.stale_reason.is_none() && binding.closed_at.is_none()
                }
                CgroupV2BindingState::Stale => {
                    binding.stale_reason.is_some() && binding.closed_at.is_none()
                }
                CgroupV2BindingState::Closed => binding.closed_at.is_some(),
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

    fn active(&mut self, id: TraceId) -> Result<&mut CgroupV2Binding, StorageError> {
        self.write()
            .bindings
            .get_mut(&id)
            .filter(|b| b.lifecycle_state == CgroupV2BindingState::Active)
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
        reason: CgroupV2BindingStaleReason,
        time: SystemTime,
    ) -> Result<(), StorageError> {
        if self.records.bindings.get(&id).is_some_and(|b| {
            b.lifecycle_state == CgroupV2BindingState::Stale && b.stale_reason == Some(reason)
        }) {
            return Ok(());
        }
        let binding = self.active(id)?;
        binding.lifecycle_state = CgroupV2BindingState::Stale;
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

    pub fn close(&mut self, id: TraceId, time: SystemTime) -> Result<(), StorageError> {
        let binding = self.write().bindings.get_mut(&id).ok_or_else(Self::error)?;
        if binding.lifecycle_state != CgroupV2BindingState::Closed {
            binding.lifecycle_state = CgroupV2BindingState::Closed;
            binding.closed_at = Some(time);
            binding.updated_at = time;
        }
        Ok(())
    }
}
