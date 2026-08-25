use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::daemon::AlertForwardingConfig;

use super::{require_positive_u32, require_positive_u64};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct AlertForwardingDocument {
    pub proxy_executable: String,
    pub proxy_config_path: String,
    pub plugin_config_path: String,
    pub socket_path: String,
    pub queue_capacity: u32,
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_ack_timeout_ms: u64,
    pub startup_timeout_ms: u64,
    pub startup_poll_interval_ms: u64,
    pub max_frame_bytes: u32,
    pub max_trace_id_bytes: u32,
    pub max_category_bytes: u32,
    pub max_description_bytes: u32,
    pub max_extras_bytes: u32,
    pub link_thread_stack_bytes: usize,
}

impl Default for AlertForwardingDocument {
    fn default() -> Self {
        Self {
            proxy_executable: "/usr/local/bin/actraild-alert-proxy".to_string(),
            proxy_config_path: "/etc/actrail/actraild-alert-proxy.toml".to_string(),
            plugin_config_path:
                "/etc/actrail/plugins/alert-forwarding/alert-forwarding.config.json".to_string(),
            socket_path: "/run/actrail/alert-proxy.sock".to_string(),
            queue_capacity: 1024,
            read_timeout_ms: 1_000,
            write_timeout_ms: 1_000,
            heartbeat_interval_ms: 5_000,
            heartbeat_ack_timeout_ms: 2_000,
            startup_timeout_ms: 5_000,
            startup_poll_interval_ms: 20,
            max_frame_bytes: 262_144,
            max_trace_id_bytes: 128,
            max_category_bytes: 128,
            max_description_bytes: 4_096,
            max_extras_bytes: 131_072,
            link_thread_stack_bytes: 524_288,
        }
    }
}

impl AlertForwardingDocument {
    pub(super) fn from_config(config: &AlertForwardingConfig) -> Self {
        Self {
            proxy_executable: config.proxy_executable.display().to_string(),
            proxy_config_path: config.proxy_config_path.display().to_string(),
            plugin_config_path: config.plugin_config_path.display().to_string(),
            socket_path: config.socket_path.display().to_string(),
            queue_capacity: config.queue_capacity,
            read_timeout_ms: config.read_timeout_ms,
            write_timeout_ms: config.write_timeout_ms,
            heartbeat_interval_ms: config.heartbeat_interval_ms,
            heartbeat_ack_timeout_ms: config.heartbeat_ack_timeout_ms,
            startup_timeout_ms: config.startup_timeout_ms,
            startup_poll_interval_ms: config.startup_poll_interval_ms,
            max_frame_bytes: config.max_frame_bytes,
            max_trace_id_bytes: config.max_trace_id_bytes,
            max_category_bytes: config.max_category_bytes,
            max_description_bytes: config.max_description_bytes,
            max_extras_bytes: config.max_extras_bytes,
            link_thread_stack_bytes: config.link_thread_stack_bytes,
        }
    }

    pub(super) fn to_config(&self) -> Result<AlertForwardingConfig, String> {
        let proxy_executable =
            require_absolute_file("alert_forwarding.proxy_executable", &self.proxy_executable)?;
        let proxy_config_path = require_absolute_file(
            "alert_forwarding.proxy_config_path",
            &self.proxy_config_path,
        )?;
        let plugin_config_path = require_absolute_file(
            "alert_forwarding.plugin_config_path",
            &self.plugin_config_path,
        )?;
        let socket_path = require_absolute_file("alert_forwarding.socket_path", &self.socket_path)?;
        if self.link_thread_stack_bytes < 65_536 {
            return Err(
                "alert_forwarding.link_thread_stack_bytes must be at least 65536".to_string(),
            );
        }
        let heartbeat_interval_ms = require_duration_ms(
            "alert_forwarding.heartbeat_interval_ms",
            self.heartbeat_interval_ms,
        )?;
        let heartbeat_ack_timeout_ms = require_duration_ms(
            "alert_forwarding.heartbeat_ack_timeout_ms",
            self.heartbeat_ack_timeout_ms,
        )?;
        if heartbeat_ack_timeout_ms >= heartbeat_interval_ms {
            return Err(
                "alert_forwarding.heartbeat_ack_timeout_ms must be less than heartbeat_interval_ms"
                    .to_string(),
            );
        }
        Ok(AlertForwardingConfig {
            proxy_executable,
            proxy_config_path,
            plugin_config_path,
            socket_path,
            queue_capacity: require_positive_u32(
                "alert_forwarding.queue_capacity",
                self.queue_capacity,
            )?,
            read_timeout_ms: require_duration_ms(
                "alert_forwarding.read_timeout_ms",
                self.read_timeout_ms,
            )?,
            write_timeout_ms: require_duration_ms(
                "alert_forwarding.write_timeout_ms",
                self.write_timeout_ms,
            )?,
            heartbeat_interval_ms,
            heartbeat_ack_timeout_ms,
            startup_timeout_ms: require_duration_ms(
                "alert_forwarding.startup_timeout_ms",
                self.startup_timeout_ms,
            )?,
            startup_poll_interval_ms: require_duration_ms(
                "alert_forwarding.startup_poll_interval_ms",
                self.startup_poll_interval_ms,
            )?,
            max_frame_bytes: require_positive_u32(
                "alert_forwarding.max_frame_bytes",
                self.max_frame_bytes,
            )?,
            max_trace_id_bytes: require_positive_u32(
                "alert_forwarding.max_trace_id_bytes",
                self.max_trace_id_bytes,
            )?,
            max_category_bytes: require_positive_u32(
                "alert_forwarding.max_category_bytes",
                self.max_category_bytes,
            )?,
            max_description_bytes: require_positive_u32(
                "alert_forwarding.max_description_bytes",
                self.max_description_bytes,
            )?,
            max_extras_bytes: require_positive_u32(
                "alert_forwarding.max_extras_bytes",
                self.max_extras_bytes,
            )?,
            link_thread_stack_bytes: self.link_thread_stack_bytes,
        })
    }
}

fn require_absolute_file(field: &str, raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(format!("{field} must be an absolute file path"));
    }
    Ok(path)
}

fn require_duration_ms(field: &'static str, value: u64) -> Result<u64, String> {
    let value = require_positive_u64(field, value)?;
    Instant::now()
        .checked_add(Duration::from_millis(value))
        .map(|_| value)
        .ok_or_else(|| format!("{field} exceeds the platform duration range"))
}
