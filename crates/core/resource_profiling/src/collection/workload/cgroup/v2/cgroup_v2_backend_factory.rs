use super::{CgroupCounterSource, CgroupProcessRssConfig, CgroupV2Backend};
use model_core::process::ProcessIdentity;

#[derive(Clone)]
pub(crate) struct CgroupSamplerFactory {
    process_rss: CgroupProcessRssConfig,
}

impl CgroupSamplerFactory {
    pub(crate) fn new(process_rss: CgroupProcessRssConfig) -> Self {
        Self { process_rss }
    }

    pub(crate) fn bind<S: CgroupCounterSource>(
        &self,
        root: ProcessIdentity,
        source: S,
    ) -> Result<CgroupV2Backend<S>, String> {
        CgroupV2Backend::new(root, source, self.process_rss.clone())
    }
}

pub(crate) struct CgroupV2BackendFactory;
impl CgroupV2BackendFactory {
    pub(crate) fn initialize(
        config: super::CgroupV2Config,
        mode: crate::WorkloadBackendMode,
        storage: &mut dyn crate::CgroupV2Persistence,
    ) -> Result<
        (
            Option<super::ManagedScopeManager>,
            Option<super::ExternalScopeManager>,
            Option<String>,
        ),
        crate::ProfilingError,
    > {
        let sampler = CgroupSamplerFactory::new(config.process_rss);
        let external =
            super::ExternalScopeManager::initialize(&config.external, sampler.clone(), storage)?;
        let (managed, unavailable) =
            super::ManagedScopeManager::initialize(&config.managed, sampler, storage)?;
        if mode == crate::WorkloadBackendMode::Cgroup
            && managed.is_none()
            && external
                .as_ref()
                .is_none_or(|manager| !manager.is_available())
        {
            return Err(crate::ProfilingError::new(
                "cgroup_unavailable",
                unavailable.unwrap_or_else(|| "no cgroup admission path is available".into()),
            ));
        }
        Ok((managed, external, unavailable))
    }
}

pub(super) struct CgroupAvailability;
impl CgroupAvailability {
    pub(super) fn is_unavailable(error: &linux_cgroup::CgroupError) -> bool {
        match error {
            linux_cgroup::CgroupError::Unsupported(_) => true,
            linux_cgroup::CgroupError::Io { source, .. } => matches!(
                source.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::PermissionDenied
                    | std::io::ErrorKind::ReadOnlyFilesystem
            ),
            _ => false,
        }
    }
}
