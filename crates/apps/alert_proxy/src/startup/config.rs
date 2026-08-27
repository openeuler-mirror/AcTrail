use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

const MIN_THREAD_STACK_BYTES: usize = 64 * 1024;
const MIN_FRAME_BYTES: usize = 256;
const JSON_FRAME_HEADER_BYTES: usize = 4;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertProxyConfig {
    pub(super) daemon_ingress: DaemonIngressConfig,
    pub(super) subscriber: SubscriberConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DaemonIngressConfig {
    pub socket_path: PathBuf,
    pub socket_mode_octal: String,
    pub socket_uid: Option<u32>,
    pub socket_gid: Option<u32>,
    pub allowed_uids: Vec<u32>,
    pub allowed_gids: Vec<u32>,
    pub connection_limit: u32,
    pub accept_poll_interval_ms: u64,
    pub io_poll_interval_ms: u64,
    pub producer_idle_timeout_ms: u64,
    pub max_frame_bytes: usize,
    pub max_trace_id_bytes: usize,
    pub max_category_bytes: usize,
    pub max_description_bytes: usize,
    pub max_extras_bytes: usize,
    pub worker_thread_stack_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubscriberConfig {
    pub listen_addr: SocketAddr,
    pub allow_insecure_remote: bool,
    pub listen_backlog: i32,
    pub connection_limit: u32,
    pub accept_poll_interval_ms: u64,
    pub io_poll_interval_ms: u64,
    pub max_frame_bytes: usize,
    pub max_client_id_bytes: usize,
    pub max_request_id_bytes: usize,
    pub max_topics: usize,
    pub max_topic_bytes: usize,
    pub broadcast_queue_capacity: usize,
    pub broadcaster_thread_stack_bytes: usize,
    pub queue_capacity: usize,
    pub heartbeat_interval_ms: u64,
    pub pong_timeout_ms: u64,
    pub peer_idle_timeout_ms: u64,
    pub worker_thread_stack_bytes: usize,
    pub allowed_tokens: Vec<String>,
}

impl AlertProxyConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("read config {}: {error}", path.display()))?;
        let config: Self = toml::from_str(&raw).map_err(|_| {
            format!(
                "parse config {} failed: invalid TOML document",
                path.display()
            )
        })?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        self.daemon_ingress.validate()?;
        self.subscriber.validate()
    }
}

impl DaemonIngressConfig {
    pub(crate) fn socket_mode(&self) -> Result<u32, String> {
        u32::from_str_radix(&self.socket_mode_octal, 8)
            .map_err(|error| format!("daemon_ingress.socket_mode_octal: {error}"))
            .and_then(|mode| {
                if mode & !0o777 != 0 {
                    Err("daemon_ingress.socket_mode_octal must be within 000..777".to_string())
                } else {
                    Ok(mode)
                }
            })
    }

    pub(crate) fn accept_poll_interval(&self) -> Duration {
        Duration::from_millis(self.accept_poll_interval_ms)
    }

    pub(crate) fn io_poll_interval(&self) -> Duration {
        Duration::from_millis(self.io_poll_interval_ms)
    }

    pub(crate) fn producer_idle_timeout(&self) -> Duration {
        Duration::from_millis(self.producer_idle_timeout_ms)
    }

    fn validate(&self) -> Result<(), String> {
        if !self.socket_path.is_absolute() {
            return Err("daemon_ingress.socket_path must be absolute".to_string());
        }
        self.socket_mode()?;
        require_nonempty("daemon_ingress.allowed_uids", &self.allowed_uids)?;
        require_nonempty("daemon_ingress.allowed_gids", &self.allowed_gids)?;
        require_positive("daemon_ingress.connection_limit", self.connection_limit)?;
        require_duration_ms(
            "daemon_ingress.accept_poll_interval_ms",
            self.accept_poll_interval_ms,
        )?;
        require_duration_ms(
            "daemon_ingress.io_poll_interval_ms",
            self.io_poll_interval_ms,
        )?;
        require_duration_ms(
            "daemon_ingress.producer_idle_timeout_ms",
            self.producer_idle_timeout_ms,
        )?;
        if self.io_poll_interval_ms > self.producer_idle_timeout_ms {
            return Err(
                "daemon_ingress.io_poll_interval_ms must not exceed producer_idle_timeout_ms"
                    .to_string(),
            );
        }
        require_frame("daemon_ingress.max_frame_bytes", self.max_frame_bytes)?;
        require_range_u16("daemon_ingress.max_trace_id_bytes", self.max_trace_id_bytes)?;
        require_range_u16("daemon_ingress.max_category_bytes", self.max_category_bytes)?;
        require_range_u16(
            "daemon_ingress.max_description_bytes",
            self.max_description_bytes,
        )?;
        if self.max_extras_bytes == 0 || self.max_extras_bytes > u32::MAX as usize {
            return Err("daemon_ingress.max_extras_bytes must be within 1..=u32::MAX".to_string());
        }
        require_stack(
            "daemon_ingress.worker_thread_stack_bytes",
            self.worker_thread_stack_bytes,
        )
    }
}

impl SubscriberConfig {
    pub(crate) fn accept_poll_interval(&self) -> Duration {
        Duration::from_millis(self.accept_poll_interval_ms)
    }

    pub(crate) fn io_poll_interval(&self) -> Duration {
        Duration::from_millis(self.io_poll_interval_ms)
    }

    pub(crate) fn heartbeat_interval(&self) -> Duration {
        Duration::from_millis(self.heartbeat_interval_ms)
    }

    pub(crate) fn pong_timeout(&self) -> Duration {
        Duration::from_millis(self.pong_timeout_ms)
    }

    pub(crate) fn peer_idle_timeout(&self) -> Duration {
        Duration::from_millis(self.peer_idle_timeout_ms)
    }

    pub(crate) fn max_json_payload_bytes(&self) -> usize {
        self.max_frame_bytes - JSON_FRAME_HEADER_BYTES
    }

    fn validate(&self) -> Result<(), String> {
        if !self.listen_addr.ip().is_loopback() && !self.allow_insecure_remote {
            return Err(
                "subscriber.listen_addr must be loopback unless allow_insecure_remote=true"
                    .to_string(),
            );
        }
        if self.listen_backlog <= 0 {
            return Err("subscriber.listen_backlog must be positive".to_string());
        }
        require_positive("subscriber.connection_limit", self.connection_limit)?;
        require_duration_ms(
            "subscriber.accept_poll_interval_ms",
            self.accept_poll_interval_ms,
        )?;
        require_duration_ms("subscriber.io_poll_interval_ms", self.io_poll_interval_ms)?;
        require_frame("subscriber.max_frame_bytes", self.max_frame_bytes)?;
        require_range_u16("subscriber.max_client_id_bytes", self.max_client_id_bytes)?;
        require_range_u16("subscriber.max_request_id_bytes", self.max_request_id_bytes)?;
        if self.max_topics == 0 {
            return Err("subscriber.max_topics must be positive".to_string());
        }
        require_range_u16("subscriber.max_topic_bytes", self.max_topic_bytes)?;
        if self.broadcast_queue_capacity == 0 {
            return Err("subscriber.broadcast_queue_capacity must be positive".to_string());
        }
        require_stack(
            "subscriber.broadcaster_thread_stack_bytes",
            self.broadcaster_thread_stack_bytes,
        )?;
        if self.queue_capacity == 0 {
            return Err("subscriber.queue_capacity must be positive".to_string());
        }
        require_duration_ms(
            "subscriber.heartbeat_interval_ms",
            self.heartbeat_interval_ms,
        )?;
        if self.heartbeat_interval_ms < 1_000 || self.heartbeat_interval_ms % 1_000 != 0 {
            return Err(
                "subscriber.heartbeat_interval_ms must be whole seconds and at least 1000"
                    .to_string(),
            );
        }
        require_duration_ms("subscriber.pong_timeout_ms", self.pong_timeout_ms)?;
        require_duration_ms("subscriber.peer_idle_timeout_ms", self.peer_idle_timeout_ms)?;
        if self.pong_timeout_ms >= self.peer_idle_timeout_ms {
            return Err("subscriber.pong_timeout_ms must be less than peer_idle_timeout_ms".into());
        }
        if self.heartbeat_interval_ms >= self.peer_idle_timeout_ms {
            return Err(
                "subscriber.heartbeat_interval_ms must be less than peer_idle_timeout_ms".into(),
            );
        }
        require_stack(
            "subscriber.worker_thread_stack_bytes",
            self.worker_thread_stack_bytes,
        )?;
        if self.allowed_tokens.is_empty() || self.allowed_tokens.iter().any(String::is_empty) {
            return Err("subscriber.allowed_tokens must contain non-empty tokens".to_string());
        }
        if self
            .allowed_tokens
            .iter()
            .any(|token| token == "replace-before-deployment")
        {
            return Err(
                "subscriber.allowed_tokens must not contain the deployment placeholder".to_string(),
            );
        }
        Ok(())
    }
}

fn require_positive<T>(name: &str, value: T) -> Result<(), String>
where
    T: PartialEq + Default,
{
    if value == T::default() {
        Err(format!("{name} must be positive"))
    } else {
        Ok(())
    }
}

fn require_duration_ms(name: &str, value: u64) -> Result<(), String> {
    require_positive(name, value)?;
    Instant::now()
        .checked_add(Duration::from_millis(value))
        .map(|_| ())
        .ok_or_else(|| format!("{name} exceeds the platform duration range"))
}

fn require_nonempty<T>(name: &str, values: &[T]) -> Result<(), String> {
    if values.is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(())
    }
}

fn require_frame(name: &str, value: usize) -> Result<(), String> {
    if value < MIN_FRAME_BYTES || value > u32::MAX as usize {
        Err(format!(
            "{name} must be within {MIN_FRAME_BYTES}..=u32::MAX"
        ))
    } else {
        Ok(())
    }
}

fn require_range_u16(name: &str, value: usize) -> Result<(), String> {
    if value == 0 || value > u16::MAX as usize {
        Err(format!("{name} must be within 1..=u16::MAX"))
    } else {
        Ok(())
    }
}

fn require_stack(name: &str, value: usize) -> Result<(), String> {
    if value < MIN_THREAD_STACK_BYTES {
        Err(format!("{name} must be at least {MIN_THREAD_STACK_BYTES}"))
    } else {
        Ok(())
    }
}
