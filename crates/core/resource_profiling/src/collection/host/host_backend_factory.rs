use super::{
    HostBackendMode, HostProfiler, HostProfilingConfig, HostResourceProfilingBackend,
    LinuxHostResourceProfilingBackend, noop::NoOpHostResourceProfilingBackend,
};
pub struct HostBackendFactory;
impl HostBackendFactory {
    pub fn build(config: HostProfilingConfig) -> Result<Option<HostProfiler>, String> {
        if !config.enabled {
            return Ok(None);
        }
        let backend: Box<dyn HostResourceProfilingBackend> = match config.mode {
            HostBackendMode::Linux => {
                let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                    .map_err(|error| format!("host boot identity: {error}"))?
                    .parse()?;
                Box::new(LinuxHostResourceProfilingBackend::new(boot_id))
            }
            HostBackendMode::Noop => Box::new(NoOpHostResourceProfilingBackend),
        };
        HostProfiler::new(backend, config.interval_ms).map(Some)
    }
}
