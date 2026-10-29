use super::{CgroupCounterSource, CgroupProcessRssConfig, CgroupScopeSampler};
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
    ) -> Result<CgroupScopeSampler<S>, String> {
        CgroupScopeSampler::new(root, source, self.process_rss.clone())
    }
}
