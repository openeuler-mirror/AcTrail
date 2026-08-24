use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sandbox_agent_runtime::SandboxAgentConfig;
use sandbox_control_uds::SandboxControlCodec;
use sandbox_linux_collector::SandboxLinuxConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SbDaemonConfig {
    collector: CollectorSection,
    sampler: SamplerSection,
    observation_queue: ObservationQueueSection,
    sender: SenderSection,
    control: ControlSection,
    diagnostics: DiagnosticsSection,
    instance_lock_path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CollectorSection {
    root_process_names: Vec<String>,
    procfs_root: PathBuf,
    require_initial_root: bool,
    root_refresh_interval_ms: u64,
    tracked_process_capacity: u32,
    pending_io_capacity: u32,
    aggregate_capacity: u32,
    poll_interval_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SamplerSection {
    poll_interval_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ObservationQueueSection {
    capacity: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SenderSection {
    batch_max_observations: usize,
    io_timeout_ms: u64,
    max_silence_interval_ms: u64,
    reconnect_interval_ms: u64,
    worker_thread_stack_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ControlSection {
    socket_path: PathBuf,
    socket_mode_octal: String,
    request_timeout_ms: u64,
    accepted_connection_max: usize,
    max_frame_bytes: usize,
    worker_thread_stack_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticsSection {
    interval_ms: u64,
}

pub(crate) struct ValidatedSbDaemonConfig {
    pub(crate) linux: SandboxLinuxConfig,
    pub(crate) resource_procfs_root: PathBuf,
    pub(crate) runtime: SandboxAgentConfig,
    pub(crate) sender_io_timeout: Duration,
    pub(crate) control: ValidatedControlConfig,
    pub(crate) diagnostics_interval: Option<Duration>,
    pub(crate) instance_lock_path: PathBuf,
}

pub(crate) struct ValidatedControlConfig {
    pub(crate) socket_path: PathBuf,
    pub(crate) socket_mode: u32,
    pub(crate) request_timeout: Duration,
    pub(crate) accepted_connection_max: usize,
    pub(crate) max_frame_bytes: usize,
    pub(crate) worker_thread_stack_bytes: usize,
}

#[derive(Debug, Default)]
pub(crate) struct SbDaemonConfigOverrides {
    root_process_names: Option<Vec<String>>,
    control_socket_path: Option<PathBuf>,
    instance_lock_path: Option<PathBuf>,
}

impl SbDaemonConfigOverrides {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_root_process_names(mut self, names: Vec<String>) -> Self {
        self.root_process_names = Some(names);
        self
    }

    pub(crate) fn with_control_socket_path(mut self, path: PathBuf) -> Self {
        self.control_socket_path = Some(path);
        self
    }

    pub(crate) fn with_instance_lock_path(mut self, path: PathBuf) -> Self {
        self.instance_lock_path = Some(path);
        self
    }
}

impl SbDaemonConfig {
    pub(crate) const DEFAULT_CONTROL_REQUEST_TIMEOUT_MS: u64 = 5_000;
    pub(crate) const DEFAULT_CONTROL_MAX_FRAME_BYTES: usize = 1_024;

    pub fn load(path: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        toml::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub(crate) fn validate(self) -> io::Result<ValidatedSbDaemonConfig> {
        Self::require_absolute("instance lock", &self.instance_lock_path)?;
        Self::require_absolute("control socket", &self.control.socket_path)?;

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

        let diagnostics_interval = (self.diagnostics.interval_ms > 0)
            .then(|| Duration::from_millis(self.diagnostics.interval_ms));
        let runtime = SandboxAgentConfig {
            io_poll_interval: Duration::from_millis(self.collector.poll_interval_ms),
            resource_poll_interval: Duration::from_millis(self.sampler.poll_interval_ms),
            max_silence_interval: Duration::from_millis(self.sender.max_silence_interval_ms),
            reconnect_interval: Duration::from_millis(self.sender.reconnect_interval_ms),
            control_request_timeout: Duration::from_millis(self.control.request_timeout_ms),
            observation_queue_capacity: self.observation_queue.capacity,
            batch_max_observations: self.sender.batch_max_observations,
            worker_thread_stack_bytes: self.sender.worker_thread_stack_bytes,
            metrics_enabled: diagnostics_interval.is_some(),
        };
        runtime.validate()?;

        let sender_io_timeout = Duration::from_millis(self.sender.io_timeout_ms);
        if sender_io_timeout.is_zero() {
            return Err(Self::invalid("sender I/O timeout must be positive"));
        }
        let socket_mode = u32::from_str_radix(&self.control.socket_mode_octal, 8)
            .map_err(|error| Self::invalid(format!("invalid control socket mode: {error}")))?;
        if socket_mode > 0o777 {
            return Err(Self::invalid(
                "control socket mode must contain permission bits only",
            ));
        }
        let request_timeout = Duration::from_millis(self.control.request_timeout_ms);
        if request_timeout.is_zero()
            || self.control.accepted_connection_max == 0
            || self.control.max_frame_bytes == 0
            || self.control.worker_thread_stack_bytes == 0
        {
            return Err(Self::invalid(
                "control timeout, connection limit, frame limit, and worker stack must be positive",
            ));
        }
        if std::time::Instant::now()
            .checked_add(request_timeout)
            .is_none()
        {
            return Err(Self::invalid(
                "control request timeout exceeds the platform clock range",
            ));
        }
        SandboxControlCodec::new(self.control.max_frame_bytes).map_err(io::Error::other)?;

        Ok(ValidatedSbDaemonConfig {
            linux,
            resource_procfs_root: self.collector.procfs_root,
            runtime,
            sender_io_timeout,
            control: ValidatedControlConfig {
                socket_path: self.control.socket_path,
                socket_mode,
                request_timeout,
                accepted_connection_max: self.control.accepted_connection_max,
                max_frame_bytes: self.control.max_frame_bytes,
                worker_thread_stack_bytes: self.control.worker_thread_stack_bytes,
            },
            diagnostics_interval,
            instance_lock_path: self.instance_lock_path,
        })
    }

    pub(crate) fn write_default(
        path: &Path,
        overrides: SbDaemonConfigOverrides,
        force: bool,
    ) -> io::Result<()> {
        let mut config = Self::default_config();
        if let Some(names) = overrides.root_process_names {
            config.collector.root_process_names = names;
        }
        if let Some(socket_path) = overrides.control_socket_path {
            config.control.socket_path = socket_path;
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
            observation_queue: ObservationQueueSection { capacity: 1_024 },
            sender: SenderSection {
                batch_max_observations: 256,
                io_timeout_ms: 1_000,
                max_silence_interval_ms: 5_000,
                reconnect_interval_ms: 1_000,
                worker_thread_stack_bytes: 524_288,
            },
            control: ControlSection {
                socket_path: PathBuf::from("/run/actrail/actrail-sb-control.sock"),
                socket_mode_octal: "600".to_string(),
                request_timeout_ms: Self::DEFAULT_CONTROL_REQUEST_TIMEOUT_MS,
                accepted_connection_max: 8,
                max_frame_bytes: Self::DEFAULT_CONTROL_MAX_FRAME_BYTES,
                worker_thread_stack_bytes: 262_144,
            },
            diagnostics: DiagnosticsSection { interval_ms: 0 },
            instance_lock_path: PathBuf::from("/run/actrail/actrail-sb.lock"),
        }
    }

    fn require_absolute(label: &str, path: &Path) -> io::Result<()> {
        if !path.is_absolute() {
            return Err(Self::invalid(format!(
                "actrail-sb {label} path must be absolute"
            )));
        }
        Ok(())
    }

    fn invalid(message: impl Into<String>) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, message.into())
    }
}
