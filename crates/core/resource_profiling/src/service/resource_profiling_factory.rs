use super::{ResourceProfilingService, resource_sample_sink::ResourceSampleSink};
use crate::{
    CgroupPersistence, HostBackendFactory, ProfilingError, ResourceProfilingConfig,
    WorkloadBackendFactory, storage::ResourceStorageFactory,
};
pub struct ResourceProfilingFactory;
impl ResourceProfilingFactory {
    pub fn build(
        mut config: ResourceProfilingConfig,
        persistence: &mut dyn CgroupPersistence,
    ) -> Result<ResourceProfilingService, ProfilingError> {
        config.collection.host.enabled &= config.enabled;
        config.collection.workload.enabled &= config.enabled;
        let backend = if config.enabled {
            Some(
                ResourceStorageFactory::build(config.storage)
                    .map_err(|e| ProfilingError::new("resource_storage", e.to_string()))?,
            )
        } else {
            None
        };
        let host = HostBackendFactory::build(config.collection.host)
            .map_err(|e| ProfilingError::new("host_profiling", e))?;
        let profiler = WorkloadBackendFactory::build(config.collection.workload, persistence)?;
        Ok(ResourceProfilingService {
            host,
            profiler,
            sink: ResourceSampleSink::new(backend, config.alerts),
        })
    }
}
