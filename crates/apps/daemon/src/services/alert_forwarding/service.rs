use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alert_delivery_contract::{AtapCodec, AtapLimits};
use alert_forwarding::{
    AlertForwardingConfig as PluginConfig, AlertForwardingPlugin, AlertForwardingPluginStatus,
};
use config_core::daemon::AlertForwardingConfig;
use control_contract::reply::ControlError;

use super::link::AlertProxyLink;

pub(crate) const ALERT_FORWARDING_INSTANCE_ID: &str = "builtin.alert-forwarding";
pub(crate) const ALERT_FORWARDING_PLUGIN_ID: &str = "actrail.alert-forwarding";

#[derive(Clone)]
pub(crate) struct AlertForwardingService {
    inner: Arc<AlertForwardingServiceInner>,
}

struct AlertForwardingServiceInner {
    config: AlertForwardingConfig,
    codec: AtapCodec,
    plugin: AlertForwardingPlugin,
    lifecycle: Mutex<ProxyLifecycle>,
}

#[derive(Default)]
struct ProxyLifecycle {
    child: Option<Child>,
}

impl AlertForwardingService {
    pub(crate) fn start(config: AlertForwardingConfig) -> Result<Self, ControlError> {
        let plugin =
            AlertForwardingPlugin::load(config.plugin_config_path.clone()).map_err(plugin_error)?;
        let limits = AtapLimits::new(
            config.max_frame_bytes as usize,
            config.max_trace_id_bytes as usize,
            config.max_category_bytes as usize,
            config.max_description_bytes as usize,
            config.max_extras_bytes as usize,
        )
        .map_err(|error| ControlError::new("alert_forwarding_config", error.to_string()))?;
        let service = Self {
            inner: Arc::new(AlertForwardingServiceInner {
                config,
                codec: AtapCodec::new(limits),
                plugin,
                lifecycle: Mutex::new(ProxyLifecycle::default()),
            }),
        };
        if service.plugin().status().requested_enabled {
            service.ensure_connected()?;
        }
        Ok(service)
    }

    pub(crate) fn plugin(&self) -> AlertForwardingPlugin {
        self.inner.plugin.clone()
    }

    pub(crate) fn status(&self) -> AlertForwardingPluginStatus {
        self.inner.plugin.status()
    }

    pub(crate) fn config_json(&self) -> Result<String, ControlError> {
        self.inner.plugin.config_json().map_err(plugin_error)
    }

    pub(crate) fn schema_json(&self) -> &'static str {
        AlertForwardingPlugin::schema_json()
    }

    pub(crate) fn validate_config(&self, raw: &str) -> Result<PluginConfig, ControlError> {
        AlertForwardingPlugin::validate_config(raw).map_err(plugin_error)
    }

    pub(crate) fn update_config(&self, raw: &str) -> Result<String, ControlError> {
        let requested = self.validate_config(raw)?;
        if requested.enabled() && self.inner.plugin.status().active_generation.is_none() {
            self.ensure_connected()?;
        }
        self.inner.plugin.update_config(raw).map_err(plugin_error)?;
        self.config_json()
    }

    pub(crate) fn queue_capacity(&self) -> u32 {
        self.inner.config.queue_capacity
    }

    pub(crate) fn shutdown(&self) {
        self.inner.plugin.deactivate();
        let mut lifecycle = self
            .inner
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(mut child) = lifecycle.child.take() {
            let _ = child.try_wait();
        }
    }

    fn ensure_connected(&self) -> Result<(), ControlError> {
        let mut lifecycle = self
            .inner
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.inner.plugin.status().active_generation.is_some() {
            return Ok(());
        }
        let startup_deadline = Instant::now()
            .checked_add(Duration::from_millis(self.inner.config.startup_timeout_ms))
            .ok_or_else(|| {
                ControlError::new(
                    "alert_proxy_startup",
                    "startup timeout exceeds the platform duration range",
                )
            })?;
        lifecycle.reap_finished_child();
        let stream = match UnixStream::connect(&self.inner.config.socket_path) {
            Ok(stream) => stream,
            Err(_) => {
                lifecycle.ensure_child(&self.inner.config)?;
                self.wait_for_proxy(&mut lifecycle, startup_deadline)?
            }
        };
        self.activate_link(stream, startup_deadline)
    }

    fn wait_for_proxy(
        &self,
        lifecycle: &mut ProxyLifecycle,
        deadline: Instant,
    ) -> Result<UnixStream, ControlError> {
        let poll = Duration::from_millis(self.inner.config.startup_poll_interval_ms);
        loop {
            if let Ok(stream) = UnixStream::connect(&self.inner.config.socket_path) {
                return Ok(stream);
            }
            if lifecycle.child_exited()? {
                return Err(ControlError::new(
                    "alert_proxy_startup",
                    "actraild-alert-proxy exited before accepting daemon connections",
                ));
            }
            if Instant::now() >= deadline {
                return Err(ControlError::new(
                    "alert_proxy_startup",
                    format!(
                        "timed out waiting for {}",
                        self.inner.config.socket_path.display()
                    ),
                ));
            }
            std::thread::sleep(poll);
        }
    }

    fn activate_link(
        &self,
        stream: UnixStream,
        startup_deadline: Instant,
    ) -> Result<(), ControlError> {
        let handshake_timeout = startup_deadline.saturating_duration_since(Instant::now());
        if handshake_timeout.is_zero() {
            return Err(ControlError::new(
                "alert_proxy_handshake",
                "alert proxy startup deadline expired before producer handshake",
            ));
        }
        stream
            .set_read_timeout(Some(handshake_timeout))
            .map_err(link_io_error)?;
        stream
            .set_write_timeout(Some(Duration::from_millis(
                self.inner.config.write_timeout_ms,
            )))
            .map_err(link_io_error)?;
        let generation = self.inner.plugin.next_generation().map_err(plugin_error)?;
        let (sender, receiver) = sync_channel(self.inner.config.queue_capacity as usize);
        let link = AlertProxyLink::handshake(
            stream,
            self.inner.codec.clone(),
            generation,
            receiver,
            self.inner.plugin.clone(),
            Duration::from_millis(self.inner.config.heartbeat_interval_ms),
            Duration::from_millis(self.inner.config.heartbeat_ack_timeout_ms),
            self.inner.config.link_thread_stack_bytes,
        )
        .map_err(|message| ControlError::new("alert_proxy_handshake", message))?;
        link.set_read_timeout(Duration::from_millis(self.inner.config.read_timeout_ms))
            .map_err(|message| ControlError::new("alert_proxy_connection", message))?;
        if !self.inner.plugin.activate(generation, sender) {
            return Err(ControlError::new(
                "alert_proxy_connection",
                "alert proxy connection generation was superseded",
            ));
        }
        if let Err(error) = link.start() {
            self.inner.plugin.disable_if_generation(generation);
            return Err(ControlError::new("alert_proxy_connection", error));
        }
        Ok(())
    }
}

impl ProxyLifecycle {
    fn ensure_child(&mut self, config: &AlertForwardingConfig) -> Result<(), ControlError> {
        if self.child.is_some() {
            return Ok(());
        }
        let child = Command::new(&config.proxy_executable)
            .arg("--config")
            .arg(&config.proxy_config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                ControlError::new(
                    "alert_proxy_startup",
                    format!("start {}: {error}", config.proxy_executable.display()),
                )
            })?;
        self.child = Some(child);
        Ok(())
    }

    fn child_exited(&mut self) -> Result<bool, ControlError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(false);
        };
        child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|error| ControlError::new("alert_proxy_startup", error.to_string()))
    }

    fn reap_finished_child(&mut self) {
        let finished = self
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok())
            .flatten()
            .is_some();
        if finished {
            self.child = None;
        }
    }
}

fn plugin_error(error: impl ToString) -> ControlError {
    ControlError::new("alert_forwarding", error.to_string())
}

fn link_io_error(error: std::io::Error) -> ControlError {
    ControlError::new("alert_proxy_connection", error.to_string())
}
