use super::{
    CgroupBackendMode, CgroupPersistence, CgroupProfilingConfig, CgroupSamplingBackend,
    v1::CgroupV1Runtime, v2::CgroupV2SamplingBackend,
};
use crate::{ProfilingError, WorkloadBackendMode};
pub(crate) struct CgroupBackendFactory;
impl CgroupBackendFactory {
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
        let v2 = if use_v2 {
            Some(CgroupV2SamplingBackend::new(
                config.v2.expect("validated"),
                WorkloadBackendMode::Auto,
                storage,
            )?)
        } else {
            None
        };
        let v1 = if use_v1 {
            Some(CgroupV1Runtime::new(
                config.v1.expect("validated"),
                storage,
            )?)
        } else {
            None
        };
        if mode == WorkloadBackendMode::Cgroup
            && !v1.as_ref().is_some_and(|b| b.available())
            && !v2.as_ref().is_some_and(|b| b.available())
        {
            return Err(ProfilingError::new(
                "cgroup_unavailable",
                "no selected cgroup version has the required capabilities",
            ));
        }
        Ok(CgroupSamplingBackend { v1, v2, mode })
    }
}
