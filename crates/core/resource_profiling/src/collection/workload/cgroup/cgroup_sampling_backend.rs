use super::CgroupPersistence;
use super::cgroup_backend::VersionBackend;
use super::common::managed::{ManagedScopeAdmission, ResourceFinalizationPoll};
use crate::model::cgroup_v1_binding::CgroupV1Binding;
use crate::model::cgroup_v2_binding::CgroupV2Binding;
use crate::model::resource_scope::TraceResourceScope;
use crate::{
    ProfilingError, ResourceMetricsDrain, ResourceSampleDraft, SamplingContext, SamplingSelection,
    WorkloadBackendMode, WorkloadSamplingBackend, WorkloadTarget, WorkloadTargets,
};
use model_core::ids::TraceId;
use model_core::process::HostProcessCoordinates;
use std::collections::BTreeMap;

/// Cgroup policy across versions. Implementations own their version-specific
/// state; this facade owns admission, sampling precedence and final barriers.
pub struct CgroupSamplingBackend {
    pub(super) backends: Vec<VersionBackend>, // v2, then v1
    pub(super) managed_unavailable: Option<String>,
    pub(super) launch_fallback_reasons: BTreeMap<TraceId, String>,
    pub(super) mode: WorkloadBackendMode,
}

impl CgroupSamplingBackend {
    pub(crate) fn new(
        backends: Vec<VersionBackend>,
        managed_unavailable: Option<String>,
        mode: WorkloadBackendMode,
    ) -> Self {
        Self {
            backends,
            managed_unavailable,
            launch_fallback_reasons: BTreeMap::new(),
            mode,
        }
    }

    fn host_scoped(&self) -> bool {
        self.backends
            .iter()
            .any(|backend| backend.has_managed() || backend.is_v2())
    }

    pub fn admit_stopped_launch(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
        launch: bool,
    ) -> Result<Option<TraceResourceScope>, ProfilingError> {
        if !launch || !target.requests_resources {
            return Ok(None);
        }
        let trace_id = target.trace_id;
        if self.host_scoped() {
            if target.is_container {
                self.launch_fallback_reasons.insert(
                    trace_id,
                    "controlled container/VM launch is outside the host-managed cgroup scope"
                        .to_string(),
                );
            } else {
                for backend in &mut self.backends {
                    match backend.admit_managed_launch(trace_id, coordinates.pid)? {
                        Some(ManagedScopeAdmission::Managed(scope)) => return Ok(Some(scope)),
                        Some(ManagedScopeAdmission::Fallback(reason)) => {
                            tracing::warn!(%trace_id, %reason, "cgroup launch admission fell back to procfs");
                            self.launch_fallback_reasons.insert(trace_id, reason);
                            break;
                        }
                        None => {}
                    }
                }
            }
        }
        self.admit_v1_binding(target, coordinates)?;
        Ok(None)
    }

    pub fn admit_binding(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ProfilingError> {
        if !target.requests_resources {
            return Ok(());
        }
        for backend in self.backends.iter_mut().filter(|backend| backend.is_v2()) {
            if backend.admit_binding(target, coordinates)? {
                return Ok(());
            }
        }
        self.admit_v1_binding(target, coordinates)
    }

    fn admit_v1_binding(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ProfilingError> {
        let reason =
            if let Some(backend) = self.backends.iter_mut().find(|backend| !backend.is_v2()) {
                match backend.admit_binding(target, coordinates) {
                    Ok(true) => return Ok(()),
                    Ok(false) => "no eligible cgroup backend for target".to_string(),
                    Err(error) if error.is_unavailable() => error.message,
                    Err(error) => return Err(error),
                }
            } else {
                "no eligible cgroup backend for target".to_string()
            };
        if self.mode == WorkloadBackendMode::Cgroup {
            return Err(ProfilingError::new("cgroup_admission", reason));
        }
        tracing::debug!(trace_id=%target.trace_id,%reason,"cgroup admission unavailable; outer auto may select procfs");
        Ok(())
    }

    pub fn activate_scope(&mut self, scope: &TraceResourceScope) -> Result<(), ProfilingError> {
        for backend in &mut self.backends {
            if backend.activate_scope(scope)? {
                self.launch_fallback_reasons.remove(&scope.trace_id);
                return Ok(());
            }
        }
        Err(ProfilingError::new(
            "cgroup_scope_activation",
            "cannot activate a scope without a cgroup runtime",
        ))
    }

    pub fn poll_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
    ) -> Result<ResourceFinalizationPoll, ProfilingError> {
        let mut poll = ResourceFinalizationPoll::default();
        for backend in &mut self.backends {
            let next = backend.poll_managed_finalizations(targets)?;
            poll.waiting.extend(next.waiting);
            poll.ready.extend(next.ready);
        }
        Ok(poll)
    }
    pub fn finish_finalization(&mut self, id: TraceId, remove_scope: bool) {
        for backend in &mut self.backends {
            backend.finish_managed_finalization(id, remove_scope);
        }
    }
    pub fn force_pending_finalizations_due(&mut self) {
        for backend in &mut self.backends {
            backend.force_pending_finalizations_due();
        }
    }
    pub fn prune_forgotten(&mut self, targets: &dyn WorkloadTargets) {
        for backend in &mut self.backends {
            backend.prune_forgotten(targets);
        }
        self.launch_fallback_reasons
            .retain(|id, _| targets.contains_trace(*id));
    }
    pub fn resource_barrier_ready(&self, id: TraceId) -> bool {
        self.backends
            .iter()
            .all(|backend| backend.managed_barrier_ready(id))
    }
    pub fn completed_managed_barrier(&self, id: TraceId) -> bool {
        self.backends
            .iter()
            .any(|backend| backend.completed_managed_barrier(id))
    }
    pub fn has_pending_barrier(&self, id: TraceId) -> bool {
        self.backends
            .iter()
            .any(|backend| backend.has_pending_managed_barrier(id))
    }
    /// True when no version still holds an unfinished v1 or v2 binding for this
    /// trace. A trace has at most one binding, so the daemon need not know which.
    pub fn binding_barrier_ready(&self, id: TraceId) -> bool {
        self.backends
            .iter()
            .all(|backend| !backend.v1_binding_pending(id) && backend.v2_binding_barrier_ready(id))
    }
    fn v2_binding_only_barrier_ready(&self, id: TraceId) -> bool {
        self.backends
            .iter()
            .all(|backend| backend.v2_binding_barrier_ready(id))
    }

    pub fn poll_v2_binding_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
    ) -> Result<Vec<super::CgroupV2BindingFinalization>, ProfilingError> {
        let mut drafts = Vec::new();
        for backend in &mut self.backends {
            drafts.extend(backend.poll_v2_binding_finalizations(targets, storage)?);
        }
        Ok(drafts)
    }
    pub fn finish_v2_binding_finalization(&mut self, id: TraceId) {
        // Existing service behavior clears any v1 fallback prepared for this
        // trace when a binding finalization is committed.
        self.finish_v1_binding_finalization(id);
        for backend in &mut self.backends {
            backend.finish_v2_binding_finalization(id);
        }
    }
    pub fn v2_binding(&self, id: TraceId) -> Option<CgroupV2Binding> {
        self.backends
            .iter()
            .find_map(|backend| backend.v2_binding(id))
    }
    pub fn has_pending_v1_binding_finalizations(&self) -> bool {
        self.backends
            .iter()
            .any(VersionBackend::has_pending_v1_binding_finalizations)
    }
    pub fn v1_binding(&self, id: TraceId) -> Option<CgroupV1Binding> {
        self.backends
            .iter()
            .find_map(|backend| backend.v1_binding(id))
    }
    pub fn activate_v1_binding(&mut self, id: TraceId) {
        for backend in &mut self.backends {
            backend.activate_v1_binding(id);
        }
    }
    pub fn finish_v1_binding_finalization(&mut self, id: TraceId) {
        for backend in &mut self.backends {
            backend.finish_v1_binding_finalization(id);
        }
    }
    pub fn poll_v1_binding_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupPersistence,
    ) -> Vec<ResourceSampleDraft> {
        self.backends
            .iter_mut()
            .flat_map(|backend| backend.poll_v1_binding_finalizations(targets, storage))
            .collect()
    }

    fn fallback_reason(&self, id: TraceId) -> String {
        self.backends
            .iter()
            .find_map(|backend| backend.fallback_reason(id))
            .or_else(|| self.launch_fallback_reasons.get(&id).map(String::as_str))
            .or(self.managed_unavailable.as_deref())
            .unwrap_or("trace_has_no_managed_cgroup_scope")
            .to_string()
    }
}

impl<'a> WorkloadSamplingBackend<dyn CgroupPersistence + 'a> for CgroupSamplingBackend {
    fn maintain(&mut self) {
        for backend in &mut self.backends {
            backend.maintain();
        }
    }
    fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut (dyn CgroupPersistence + 'a),
        context: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        let mut drain = ResourceMetricsDrain::default();
        if self.host_scoped() {
            for trace in targets.traces() {
                let id = trace.trace_id;
                if trace.terminal
                    && self.resource_barrier_ready(id)
                    && self.v2_binding_only_barrier_ready(id)
                {
                    selection.claim(id);
                } else if !selection.is_claimed(id) && self.completed_managed_barrier(id) {
                    selection.claim(id);
                }
            }
            // Complete the managed phase across all versions before any
            // binding. A v1 managed scope must outrank a v2 binding.
            for backend in &mut self.backends {
                backend.collect_managed(targets, context, selection, &mut drain);
            }
            for backend in self.backends.iter_mut().filter(|backend| backend.is_v2()) {
                let next = backend.collect_bindings(targets, storage, context, selection)?;
                drain.samples.extend(next.samples);
                drain.failures.extend(next.failures);
            }
            for trace in targets.traces() {
                if trace.requests_resources && !selection.is_claimed(trace.trace_id) {
                    let reason = self.fallback_reason(trace.trace_id);
                    selection
                        .fallback_metadata(trace.trace_id)
                        .insert("cgroup_fallback_reason".into(), reason);
                }
            }
        }
        for backend in self.backends.iter_mut().filter(|backend| !backend.is_v2()) {
            let next = backend.collect_bindings(targets, storage, context, selection)?;
            drain.samples.extend(next.samples);
            drain.failures.extend(next.failures);
        }
        Ok(drain)
    }
}
