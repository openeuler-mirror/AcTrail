use super::super::{
    cgroup::CgroupBackendFactory, noop::NoOpSamplingBackend, procfs::ProcfsSamplingSessions,
    sampling::FallbackChain,
};
use super::{WorkloadBackend, workload_backend::SelectedBackend};
use crate::{
    CgroupPersistence, ProfilingError, WorkloadBackendMode, WorkloadProfiler,
    WorkloadProfilingConfig,
};
pub struct WorkloadBackendFactory;
impl WorkloadBackendFactory {
    pub fn build(
        config: WorkloadProfilingConfig,
        storage: &mut dyn CgroupPersistence,
    ) -> Result<WorkloadProfiler<WorkloadBackend>, ProfilingError> {
        WorkloadProfiler::<WorkloadBackend>::validate_interval(config.interval_ms, config.enabled)?;
        let missing = |name| {
            ProfilingError::new(
                "resource_config",
                format!("selected workload mode requires {name} configuration"),
            )
        };
        let selected = if !config.enabled {
            SelectedBackend::NoOp(NoOpSamplingBackend)
        } else {
            match config.mode {
                WorkloadBackendMode::Noop => SelectedBackend::NoOp(NoOpSamplingBackend),
                WorkloadBackendMode::Procfs => SelectedBackend::Procfs(
                    ProcfsSamplingSessions::new(config.procfs.ok_or_else(|| missing("procfs"))?),
                ),
                WorkloadBackendMode::Cgroup => {
                    SelectedBackend::Cgroup(CgroupBackendFactory::build(
                        config.cgroup.ok_or_else(|| missing("cgroup"))?,
                        config.mode,
                        storage,
                    )?)
                }
                WorkloadBackendMode::Auto => {
                    let cgroup = config.cgroup.ok_or_else(|| missing("cgroup"))?;
                    let procfs = config.procfs.ok_or_else(|| missing("procfs"))?;
                    SelectedBackend::Auto(FallbackChain {
                        primary: CgroupBackendFactory::build(cgroup, config.mode, storage)?,
                        fallback: ProcfsSamplingSessions::new(procfs),
                    })
                }
            }
        };
        WorkloadProfiler::new(
            WorkloadBackend { selected },
            config.interval_ms,
            config.enabled,
        )
    }
}
