use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sandbox_agent_runtime::SandboxAgentConfig;
use sandbox_linux_collector::SandboxLinuxConfig;
use sandbox_vsock_transport::VsockClientConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SbConfig {
    pub collector: CollectorSection,
    pub sampler: SamplerSection,
    pub transport: TransportSection,
    pub runtime: RuntimeSection,
    pub diagnostics: DiagnosticsSection,
    pub instance_lock_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CollectorSection {
    pub root_process_names: Vec<String>,
    pub procfs_root: PathBuf,
    pub require_initial_root: bool,
    pub root_refresh_interval_ms: u64,
    pub tracked_process_capacity: u32,
    pub pending_io_capacity: u32,
    pub aggregate_capacity: u32,
    pub poll_interval_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SamplerSection {
    pub poll_interval_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransportSection {
    pub host_cid: u32,
    pub port: u32,
    pub io_timeout_ms: u64,
    pub max_silence_interval_ms: u64,
    pub reconnect_interval_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSection {
    pub observation_queue_capacity: usize,
    pub batch_max_observations: usize,
    pub worker_thread_stack_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsSection {
    pub interval_ms: u64,
}

pub struct ValidatedSbConfig {
    pub linux: SandboxLinuxConfig,
    pub resource_procfs_root: PathBuf,
    pub transport: VsockClientConfig,
    pub runtime: SandboxAgentConfig,
    pub diagnostics_interval: Option<Duration>,
    pub instance_lock_path: PathBuf,
}

#[derive(Debug, Default)]
pub struct SbConfigOverrides {
    root_process_names: Option<Vec<String>>,
    host_cid: Option<u32>,
    port: Option<u32>,
    instance_lock_path: Option<PathBuf>,
}

impl SbConfigOverrides {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_root_process_names(mut self, names: Vec<String>) -> Self {
        self.root_process_names = Some(names);
        self
    }

    pub fn with_host_cid(mut self, host_cid: u32) -> Self {
        self.host_cid = Some(host_cid);
        self
    }

    pub fn with_port(mut self, port: u32) -> Self {
        self.port = Some(port);
        self
    }

    pub fn with_instance_lock_path(mut self, path: PathBuf) -> Self {
        self.instance_lock_path = Some(path);
        self
    }
}

impl SbConfig {
    pub fn load(path: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn validate(self) -> io::Result<ValidatedSbConfig> {
        if !self.instance_lock_path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "actrail-sb instance lock path must be absolute",
            ));
        }
        let linux = SandboxLinuxConfig::new(&self.collector.root_process_names)
            .and_then(|config| config.with_procfs_root(&self.collector.procfs_root))
            .and_then(|config| {
                config.with_map_capacities(
                    self.collector.tracked_process_capacity,
                    self.collector.pending_io_capacity,
                    self.collector.aggregate_capacity,
                )
            })
            .and_then(|config| {
                config.with_root_refresh_interval(Duration::from_millis(
                    self.collector.root_refresh_interval_ms,
                ))
            })
            .map(|config| config.with_initial_root_required(self.collector.require_initial_root))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let transport = VsockClientConfig {
            host_cid: self.transport.host_cid,
            port: self.transport.port,
            io_timeout: Duration::from_millis(self.transport.io_timeout_ms),
        };
        if transport.host_cid == libc::VMADDR_CID_ANY
            || transport.port == libc::VMADDR_PORT_ANY
            || transport.io_timeout.is_zero()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "actrail-sb VSOCK host CID and port must be concrete and I/O timeout must be positive",
            ));
        }
        let diagnostics_interval = (self.diagnostics.interval_ms > 0)
            .then(|| Duration::from_millis(self.diagnostics.interval_ms));
        let runtime = SandboxAgentConfig {
            io_poll_interval: Duration::from_millis(self.collector.poll_interval_ms),
            resource_poll_interval: Duration::from_millis(self.sampler.poll_interval_ms),
            max_silence_interval: Duration::from_millis(self.transport.max_silence_interval_ms),
            reconnect_interval: Duration::from_millis(self.transport.reconnect_interval_ms),
            observation_queue_capacity: self.runtime.observation_queue_capacity,
            batch_max_observations: self.runtime.batch_max_observations,
            worker_thread_stack_bytes: self.runtime.worker_thread_stack_bytes,
            metrics_enabled: diagnostics_interval.is_some(),
        };
        runtime.validate()?;
        Ok(ValidatedSbConfig {
            linux,
            resource_procfs_root: self.collector.procfs_root,
            transport,
            runtime,
            diagnostics_interval,
            instance_lock_path: self.instance_lock_path,
        })
    }

    pub fn write_default(path: &Path, overrides: SbConfigOverrides, force: bool) -> io::Result<()> {
        let mut config = Self::default_config();
        if let Some(names) = overrides.root_process_names {
            config.collector.root_process_names = names;
        }
        if let Some(host_cid) = overrides.host_cid {
            config.transport.host_cid = host_cid;
        }
        if let Some(port) = overrides.port {
            config.transport.port = port;
        }
        if let Some(instance_lock_path) = overrides.instance_lock_path {
            config.instance_lock_path = instance_lock_path;
        }
        config.clone().validate()?;
        let text = toml::to_string_pretty(&config)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        toml::from_str::<Self>(&text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .validate()?;
        let mut options = OpenOptions::new();
        options.write(true);
        if force {
            options.create(true).truncate(true);
        } else {
            options.create_new(true);
        }
        let mut file = options.open(path)?;
        file.write_all(text.as_bytes())?;
        drop(file);
        Self::load(path)?.validate().map(|_| ())
    }

    fn default_config() -> Self {
        Self {
            collector: CollectorSection {
                root_process_names: vec!["xiaoo".to_string(), "claude".to_string()],
                procfs_root: PathBuf::from("/proc"),
                require_initial_root: false,
                root_refresh_interval_ms: 1_000,
                tracked_process_capacity: 16_384,
                pending_io_capacity: 32_768,
                aggregate_capacity: 4_096,
                poll_interval_ms: 1_000,
            },
            sampler: SamplerSection {
                poll_interval_ms: 1_000,
            },
            transport: TransportSection {
                host_cid: 2,
                port: 43_182,
                io_timeout_ms: 1_000,
                max_silence_interval_ms: 5_000,
                reconnect_interval_ms: 1_000,
            },
            runtime: RuntimeSection {
                observation_queue_capacity: 1_024,
                batch_max_observations: 256,
                worker_thread_stack_bytes: 524_288,
            },
            diagnostics: DiagnosticsSection { interval_ms: 0 },
            instance_lock_path: PathBuf::from("/run/actrail/actrail-sb.lock"),
        }
    }
}
