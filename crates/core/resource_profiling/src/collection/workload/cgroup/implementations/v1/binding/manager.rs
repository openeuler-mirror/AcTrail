use super::super::CgroupV1Config;
use super::{
    handle::{CgroupV1BindingHandle, V1Discovery},
    persistence::CgroupV1BindingPersistence,
    sampler::CgroupV1BindingSampler,
};
use crate::model::ResourceAccountingCoverage;
use crate::model::ResourceAccountingMethod;
use crate::model::ResourceMetrics;
use crate::model::ResourceSampleKind;
use crate::model::cgroup_v1_binding::CgroupV1Binding;
use crate::{
    ProfilingError, ResourceMetricsDrain, ResourceSampleDraft, ResourceSamplingFailure,
    SamplingContext, SamplingSelection, WorkloadTarget, WorkloadTargets,
};
use model_core::host_identity::HostBootId;
use model_core::ids::TraceId;
use model_core::process::HostProcessCoordinates;
use model_core::process::ProcessIdentity;
use std::{collections::BTreeMap, time::SystemTime};
pub struct CgroupV1BindingManager {
    config: CgroupV1Config,
    discovery: V1Discovery,
    boot: HostBootId,
    sessions: BTreeMap<TraceId, CgroupV1BindingSession>,
}
struct CgroupV1BindingSession {
    binding: CgroupV1Binding,
    root: ProcessIdentity,
    backend: Option<CgroupV1BindingSampler>,
    pending_stale: Option<String>,
    active: bool,
    recovered: bool,
    terminal: bool,
    diagnosed: bool,
}
impl CgroupV1BindingManager {
    pub(crate) fn new(
        config: CgroupV1Config,
        storage: &mut dyn CgroupV1BindingPersistence,
    ) -> Result<Self, ProfilingError> {
        if config.binding_attempts == 0 {
            return Err(ProfilingError::new(
                "cgroup_v1_config",
                "binding_attempts must be positive",
            ));
        }
        let discovery = V1Discovery::discover()?;
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|e| ProfilingError::new("cgroup_v1_boot", e.to_string()))?
            .parse()
            .map_err(|e| ProfilingError::new("cgroup_v1_boot", e))?;
        let mut sessions = BTreeMap::new();
        for record in storage.prepare_v1_binding_recovery()? {
            let mut binding = record.binding;
            let backend = if binding.stale_reason.is_none() {
                match CgroupV1BindingHandle::reopen(&discovery, &binding, boot) {
                    Ok(handle) => Some(CgroupV1BindingSampler::new(record.root, handle)),
                    Err(error) => {
                        storage.mark_v1_binding_stale(binding.trace_id, &error.message)?;
                        binding.stale_reason = Some(error.message);
                        None
                    }
                }
            } else {
                None
            };
            sessions.insert(
                binding.trace_id,
                CgroupV1BindingSession {
                    binding,
                    root: record.root,
                    backend,
                    pending_stale: None,
                    active: true,
                    recovered: true,
                    terminal: record.terminal,
                    diagnosed: false,
                },
            );
        }
        Ok(Self {
            config,
            discovery,
            boot,
            sessions,
        })
    }
    pub(crate) fn available(&self) -> bool {
        self.discovery.available()
    }
    pub(crate) fn admit(
        &mut self,
        target: &WorkloadTarget,
        coordinates: &HostProcessCoordinates,
    ) -> Result<(), ProfilingError> {
        if self.sessions.contains_key(&target.trace_id) {
            return Err(ProfilingError::new(
                "cgroup_v1_admission",
                "trace already bound",
            ));
        }
        let (binding, handle) = CgroupV1BindingHandle::bind(
            &self.discovery,
            &self.config,
            target.trace_id,
            coordinates,
            self.boot,
        )?;
        self.sessions.insert(
            target.trace_id,
            CgroupV1BindingSession {
                binding,
                root: target.root,
                backend: Some(CgroupV1BindingSampler::new(target.root, handle)),
                pending_stale: None,
                active: false,
                recovered: false,
                terminal: false,
                diagnosed: false,
            },
        );
        Ok(())
    }
    pub fn binding(&self, id: TraceId) -> Option<CgroupV1Binding> {
        self.sessions.get(&id).map(|s| s.binding.clone())
    }
    pub fn activate(&mut self, id: TraceId) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.active = true;
        }
    }
    pub fn finish(&mut self, id: TraceId) {
        self.sessions.remove(&id);
    }
    pub fn pending(&self, id: TraceId) -> bool {
        self.sessions.get(&id).is_some_and(|s| s.active)
    }
    pub fn has_pending_finalizations(&self) -> bool {
        self.sessions
            .values()
            .any(|session| session.active && session.terminal)
    }
    pub(crate) fn collect(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupV1BindingPersistence,
        context: SamplingContext,
        selection: &mut SamplingSelection,
    ) -> Result<ResourceMetricsDrain, ProfilingError> {
        let traces = targets
            .traces()
            .into_iter()
            .map(|t| (t.trace_id, t))
            .collect::<BTreeMap<_, _>>();
        self.sessions
            .retain(|id, s| s.active || traces.get(id).is_some_and(|t| !t.terminal));
        let mut drain = ResourceMetricsDrain::default();
        for (id, session) in &mut self.sessions {
            if selection.is_claimed(*id) {
                continue;
            }
            selection.claim(*id);
            if !session.active || traces.get(id).is_some_and(|t| t.terminal) || session.terminal {
                continue;
            }
            let result = session.sample(targets, storage, context);
            match result {
                Ok(Some(payload)) => {
                    session.diagnosed = false;
                    drain.samples.push(ResourceSampleDraft {
                        trace_id: *id,
                        observed_at: context.observed_at,
                        process: session.root,
                        payload,
                        recovered: session.recovered && !targets.contains_trace(*id),
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    if !session.diagnosed {
                        session.diagnosed = true;
                        drain.failures.push(ResourceSamplingFailure {
                            trace_id: *id,
                            message: error.to_string(),
                            recovered: session.recovered && !targets.contains_trace(*id),
                        });
                    }
                }
            }
        }
        Ok(drain)
    }
    pub fn poll_finalizations(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupV1BindingPersistence,
    ) -> Vec<ResourceSampleDraft> {
        let traces = targets
            .traces()
            .into_iter()
            .map(|t| (t.trace_id, t))
            .collect::<BTreeMap<_, _>>();
        let mut drafts = Vec::new();
        for (id, session) in &mut self.sessions {
            if !session.active {
                continue;
            }
            let terminal = traces.get(id).is_some_and(|t| t.terminal) || session.terminal;
            let recovered = session.recovered && !targets.contains_trace(*id);
            let root_gone = if recovered && !terminal {
                if session.binding.host_boot_id != self.boot {
                    true
                } else {
                    match targets.root_is_gone(session.root) {
                        Ok(gone) => gone,
                        Err(error) => {
                            tracing::warn!(%id,%error,"v1 recovered root liveness unavailable");
                            false
                        }
                    }
                }
            } else {
                false
            };
            if !terminal && !root_gone {
                continue;
            }
            session.terminal = true;
            let at = SystemTime::now();
            if recovered {
                if let Err(error) = storage.complete_v1_binding_recovered_trace(*id, at) {
                    tracing::warn!(%id,%error,"v1 recovered completion failed");
                    continue;
                }
                session.terminal = true;
            }
            if let Err(error) = session.persist_stale(storage) {
                tracing::warn!(%id,%error,"v1 stale persistence failed");
                continue;
            }
            let context = SamplingContext {
                sampled_at: std::time::Instant::now(),
                observed_at: at,
            };
            let mut payload = match session.final_payload(targets, storage, context) {
                Ok(payload) if session.binding.stale_reason.is_none() => payload,
                result => {
                    let reason = session
                        .binding
                        .stale_reason
                        .clone()
                        .or_else(|| result.err())
                        .unwrap_or_else(|| "bound counters unavailable".into());
                    ResourceMetrics {
                        scope: "cgroup".into(),
                        subject: id.to_string(),
                        accounting_method: ResourceAccountingMethod::CgroupV1,
                        accounting_coverage: ResourceAccountingCoverage::Partial,
                        metadata: BTreeMap::from([("finalization_read_error".into(), reason)]),
                        ..ResourceMetrics::default()
                    }
                }
            };
            if let Err(error) = session.persist_stale(storage) {
                tracing::warn!(%id,%error,"v1 final stale persistence failed");
                continue;
            }
            payload.sample_kind = ResourceSampleKind::Final;
            drafts.push(ResourceSampleDraft {
                trace_id: *id,
                observed_at: at,
                process: session.root,
                payload,
                recovered,
            });
        }
        drafts
    }
}
impl CgroupV1BindingSession {
    fn final_payload(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupV1BindingPersistence,
        context: SamplingContext,
    ) -> Result<ResourceMetrics, String> {
        if !targets.root_is_gone(self.root).map_err(|e| e.to_string())? {
            return self
                .sample(targets, storage, context)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "root exited during final sampling".into());
        }
        // A completed root cannot prove membership; retain its pinned boundary.
        self.backend
            .as_mut()
            .ok_or_else(|| "bound counters unavailable".to_string())?
            .payload(&self.binding, context)
    }
    fn persist_stale(
        &mut self,
        storage: &mut dyn CgroupV1BindingPersistence,
    ) -> Result<(), ProfilingError> {
        if let Some(reason) = self.pending_stale.as_ref() {
            storage.mark_v1_binding_stale(self.binding.trace_id, reason)?;
            self.binding.stale_reason = self.pending_stale.take();
            self.backend = None;
        }
        Ok(())
    }
    fn sample(
        &mut self,
        targets: &dyn WorkloadTargets,
        storage: &mut dyn CgroupV1BindingPersistence,
        context: SamplingContext,
    ) -> Result<Option<ResourceMetrics>, ProfilingError> {
        self.persist_stale(storage)?;
        if self.binding.stale_reason.is_some() {
            return Ok(None);
        }
        let coordinates = targets.coordinates(self.root)?.ok_or_else(|| {
            ProfilingError::new(
                "cgroup_v1_identity",
                "root observer coordinates unavailable",
            )
        })?;
        let backend = self
            .backend
            .as_mut()
            .ok_or_else(|| ProfilingError::new("cgroup_v1_binding", "bound reader missing"))?;
        if let Err(error) = backend.binding.verify(&self.binding, &coordinates) {
            if targets.root_is_gone(self.root)? {
                return Ok(None);
            }
            if error.code != "cgroup_v1_binding" {
                return Err(error);
            }
            self.pending_stale = Some(error.message.clone());
            self.persist_stale(storage)?;
            return Err(error);
        }
        let sample = backend
            .payload(&self.binding, context)
            .map_err(|e| ProfilingError::new("cgroup_v1_sample", e))?;
        // Reject a sample collected while the target moved controller boundaries.
        if let Err(error) = backend.binding.verify(&self.binding, &coordinates) {
            if targets.root_is_gone(self.root)? {
                return Ok(None);
            }
            if error.code != "cgroup_v1_binding" {
                return Err(error);
            }
            self.pending_stale = Some(error.message.clone());
            self.persist_stale(storage)?;
            return Err(error);
        }
        Ok(Some(sample))
    }
}
