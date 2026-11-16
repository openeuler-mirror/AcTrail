use super::cgroup_backend::VersionBackend;
use super::common::managed::ManagedScopeRuntime;
use super::common::sampler::CgroupSamplerFactory;
use super::{
    CgroupBackendMode, CgroupPersistence, CgroupProfilingConfig, CgroupSamplingBackend,
    v1::{CgroupV1BindingManager, CgroupV1SamplingBackend, ManagedV1Hierarchy},
    v2::{CgroupV2SamplingBackend, ManagedV2Hierarchy},
};
use crate::model::ResourceAccountingMethod;
use crate::{ProfilingError, WorkloadBackendMode};

pub(crate) struct CgroupBackendFactory;
impl CgroupBackendFactory {
    /// The persisted live scope chooses its recovery hierarchy. Otherwise new
    /// managed scopes prefer v2, then v1. Each version owns its own runtime.
    pub(crate) fn build(
        config: CgroupProfilingConfig,
        mode: WorkloadBackendMode,
        storage: &mut dyn CgroupPersistence,
    ) -> Result<CgroupSamplingBackend, ProfilingError> {
        let use_v1 = config.mode != CgroupBackendMode::V2;
        let use_v2 = config.mode != CgroupBackendMode::V1;
        if use_v1 && config.v1.is_none() || use_v2 && config.v2.is_none() {
            return Err(ProfilingError::new(
                "cgroup_config",
                "selected version configuration missing",
            ));
        }
        storage.validate_recovery_versions(use_v1, use_v2)?;
        let v2_sampler = CgroupSamplerFactory::new(
            config
                .v2
                .as_ref()
                .filter(|_| use_v2)
                .map(|v2| v2.process_rss.clone())
                .unwrap_or_default(),
        );
        let records = storage.prepare_recovery()?;
        if records.iter().any(|record| {
            record.scope.lifecycle_state.is_live()
                && !matches!(
                    record.scope.accounting_method,
                    ResourceAccountingMethod::CgroupV1 | ResourceAccountingMethod::CgroupV2
                )
        }) {
            return Err(ProfilingError::new(
                "cgroup_recovery",
                "live managed scope has an unsupported cgroup accounting method",
            ));
        }
        let live_v1 = records.iter().any(|record| {
            record.scope.lifecycle_state.is_live()
                && record.scope.accounting_method == ResourceAccountingMethod::CgroupV1
        });
        let live_v2 = records.iter().any(|record| {
            record.scope.lifecycle_state.is_live()
                && record.scope.accounting_method == ResourceAccountingMethod::CgroupV2
        });
        if live_v1 && live_v2 {
            return Err(ProfilingError::new(
                "cgroup_recovery",
                "live managed scopes exist in both cgroup versions; a single managed runtime cannot recover both",
            ));
        }

        let mut reasons = Vec::new();
        let mut backends = Vec::with_capacity(2);
        let mut managed_v2 = None;
        if let Some(v2) = config.v2.filter(|_| use_v2) {
            // A live v1 scope must be recovered by v1 even if v2 is available.
            if !live_v1 {
                let (managed, reason) = ManagedScopeRuntime::<ManagedV2Hierarchy>::try_initialize(
                    &v2.managed,
                    v2_sampler.clone(),
                    storage,
                    &records,
                )?;
                managed_v2 = managed;
                reasons.extend(reason);
            }
            let backend = CgroupV2SamplingBackend::new(
                v2,
                v2_sampler.clone(),
                WorkloadBackendMode::Auto,
                storage,
                managed_v2,
            )?;
            backends.push(VersionBackend::V2(backend));
        }

        if let Some(v1) = config.v1.filter(|_| use_v1) {
            let managed_v1 = if !backends.iter().any(VersionBackend::has_managed)
                && v1.managed.enabled
            {
                let (managed, reason) = ManagedScopeRuntime::<ManagedV1Hierarchy>::try_initialize(
                    &v1.managed,
                    CgroupSamplerFactory::new(Default::default()),
                    storage,
                    &records,
                )?;
                reasons.extend(reason);
                managed
            } else {
                None
            };
            let bindings = CgroupV1BindingManager::new(v1, storage)?;
            backends.push(VersionBackend::V1(CgroupV1SamplingBackend::new(
                managed_v1, bindings,
            )));
        }

        let has_managed = backends.iter().any(VersionBackend::has_managed);
        if !has_managed && (live_v1 || live_v2) {
            return Err(ProfilingError::new(
                "cgroup_recovery",
                "live managed resource scopes exist but no managed cgroup backend is selected",
            ));
        }
        if mode == WorkloadBackendMode::Cgroup
            && !has_managed
            && !backends.iter().any(VersionBackend::available)
        {
            return Err(ProfilingError::new(
                "cgroup_unavailable",
                "no selected cgroup version has the required capabilities",
            ));
        }
        let managed_unavailable = if has_managed {
            None
        } else if reasons.is_empty() {
            Some("managed cgroup admission disabled".to_string())
        } else {
            Some(reasons.join("; "))
        };
        Ok(CgroupSamplingBackend::new(
            backends,
            managed_unavailable,
            mode,
        ))
    }
}
