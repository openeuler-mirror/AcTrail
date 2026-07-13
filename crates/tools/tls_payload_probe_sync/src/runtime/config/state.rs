use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tls_payload_core::{
    CoreError, Decision, EqualLenRewriteProcessor, PayloadContext, PayloadDirection, RewriteRule,
    SyncProcessor,
};
use tls_payload_sync::{EventClient, SyncEvent};

use crate::runtime::flow_control::{FlowControlConfig, FlowController, FlowDecision};

use super::plan::RuntimePlan;
use super::policy::{EventFilter, RedactionMode};

static CONFIG: OnceLock<RuntimeConfig> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct HookPoint {
    pub(in crate::runtime) symbol: String,
    pub(in crate::runtime) direction: PayloadDirection,
    pub(in crate::runtime) file_offset: u64,
}

pub(super) struct RuntimeConfigParts {
    pub(super) rules: Vec<RewriteRule>,
    pub(super) max_payload_bytes: usize,
    pub(super) flow_control: FlowControlConfig,
    pub(super) redaction: RedactionMode,
    pub(super) events: EventFilter,
    pub(super) trace_id: Option<u64>,
    pub(super) event_transport: Option<EventTransportConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum EventTransportConfig {
    InheritedFd {
        fd: i32,
        reconnect_path: Option<PathBuf>,
        pending_byte_budget: usize,
        write_buffer_bytes: usize,
    },
    Socket {
        path: PathBuf,
        pending_byte_budget: usize,
        write_buffer_bytes: usize,
    },
}

impl EventTransportConfig {
    fn connect(&self, reconnect: bool) -> Result<EventClient, String> {
        match self {
            Self::InheritedFd {
                fd,
                reconnect_path,
                pending_byte_budget,
                write_buffer_bytes,
            } => {
                if reconnect {
                    let path = reconnect_path.as_ref().ok_or_else(|| {
                        "forked sync runtime requires TLS_PAYLOAD_SYNC_EVENT_SOCKET to reconnect"
                            .to_string()
                    })?;
                    EventClient::connect(path, *pending_byte_budget, *write_buffer_bytes).map_err(
                        |error| format!("reconnect sync event socket {}: {error}", path.display()),
                    )
                } else {
                    EventClient::connect_inherited_fd(
                        *fd,
                        *pending_byte_budget,
                        *write_buffer_bytes,
                    )
                    .map_err(|error| format!("connect inherited sync event fd {fd}: {error}"))
                }
            }
            Self::Socket {
                path,
                pending_byte_budget,
                write_buffer_bytes,
            } => EventClient::connect(path, *pending_byte_budget, *write_buffer_bytes)
                .map_err(|error| format!("connect sync event socket {}: {error}", path.display())),
        }
    }
}

#[derive(Debug)]
pub(in crate::runtime) struct RuntimeConfig {
    processor: Mutex<EqualLenRewriteProcessor>,
    max_payload_bytes: usize,
    flow_control: FlowControlConfig,
    redaction: RedactionMode,
    events: EventFilter,
    trace_id: Option<u64>,
    event_transport: Option<EventTransportConfig>,
    event_client: Mutex<Option<EventClient>>,
    event_client_pid: AtomicU32,
    flow_controller: Mutex<FlowController>,
    sequence: AtomicU64,
    symbol_providers: Mutex<BTreeMap<String, String>>,
}

impl RuntimeConfig {
    pub(super) fn from_parts(parts: RuntimeConfigParts) -> Self {
        Self {
            processor: Mutex::new(EqualLenRewriteProcessor::new(parts.rules)),
            max_payload_bytes: parts.max_payload_bytes,
            flow_control: parts.flow_control,
            redaction: parts.redaction,
            events: parts.events,
            trace_id: parts.trace_id,
            event_transport: parts.event_transport,
            event_client: Mutex::new(None),
            event_client_pid: AtomicU32::new(0),
            flow_controller: Mutex::new(FlowController::default()),
            sequence: AtomicU64::new(0),
            symbol_providers: Mutex::new(BTreeMap::new()),
        }
    }

    pub(in crate::runtime) fn max_payload_bytes(&self) -> usize {
        self.max_payload_bytes
    }

    pub(in crate::runtime) fn should_print_payload(&self) -> bool {
        self.events.payload()
    }

    pub(in crate::runtime) fn should_print_decision(&self) -> bool {
        self.events.decision()
    }

    pub(in crate::runtime) fn should_print_target(&self) -> bool {
        self.events.target()
    }

    pub(in crate::runtime) fn trace_id(&self) -> Option<u64> {
        self.trace_id
    }

    pub(in crate::runtime) fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::Relaxed)
    }

    pub(in crate::runtime) fn register_plan(&self, plan: &RuntimePlan) -> Result<(), String> {
        let mut providers = self
            .symbol_providers
            .lock()
            .map_err(|_| "symbol provider mutex poisoned".to_string())?;
        for point in &plan.points {
            providers
                .entry(point.symbol.clone())
                .or_insert_with(|| plan.provider.clone());
        }
        Ok(())
    }

    pub(in crate::runtime) fn has_registered_plan(&self) -> bool {
        self.symbol_providers
            .lock()
            .map(|providers| !providers.is_empty())
            .unwrap_or(false)
    }

    pub(in crate::runtime) fn provider_for_symbol(&self, symbol: &str) -> String {
        self.symbol_providers
            .lock()
            .ok()
            .and_then(|providers| providers.get(symbol).cloned())
            .unwrap_or_else(|| "unknown".to_string())
    }

    pub(in crate::runtime) fn send_event(&self, event: SyncEvent) -> Result<(), String> {
        let current_pid = std::process::id();
        let mut client = self
            .event_client
            .lock()
            .map_err(|_| "sync event client mutex poisoned".to_string())?;
        let previous_pid = self.event_client_pid.load(Ordering::Acquire);
        let reconnect = previous_pid != 0 && previous_pid != current_pid;
        if reconnect {
            *client = None;
        }
        if client.is_none() {
            let Some(transport) = &self.event_transport else {
                return Ok(());
            };
            *client = Some(transport.connect(reconnect)?);
            self.event_client_pid.store(current_pid, Ordering::Release);
        }
        let Some(client) = client.as_ref() else {
            return Ok(());
        };
        client.send(event).map_err(|error| error.to_string())
    }

    pub(in crate::runtime) fn classify_flow(
        &self,
        direction: PayloadDirection,
        stream_key: usize,
        payload: &[u8],
    ) -> FlowDecision {
        self.flow_controller
            .lock()
            .map(|mut controller| {
                controller.observe(self.flow_control, direction, stream_key, payload)
            })
            .unwrap_or(FlowDecision::EmitPayload)
    }

    pub(in crate::runtime) fn close_event_client(&self) -> Result<(), String> {
        let client = self
            .event_client
            .lock()
            .map_err(|_| "sync event client mutex poisoned".to_string())?
            .take();
        let Some(client) = client else {
            return Ok(());
        };
        client.close_and_join().map_err(|error| error.to_string())
    }

    pub(in crate::runtime) fn redact_payload(&self, payload: &[u8]) -> String {
        match self.redaction {
            RedactionMode::Redact => format!("<redacted bytes={}>", payload.len()),
            RedactionMode::None => String::from_utf8_lossy(payload).into_owned(),
        }
    }

    pub(in crate::runtime) fn decide(
        &self,
        provider: &str,
        symbol: &str,
        direction: PayloadDirection,
        stream_key: usize,
        payload: &[u8],
    ) -> Result<Decision, CoreError> {
        let context = PayloadContext {
            direction,
            provider,
            symbol,
            stream_key,
        };
        self.processor
            .lock()
            .map_err(|_| CoreError::new("processor mutex poisoned"))?
            .decide(&context, payload)
    }
}

pub(in crate::runtime) fn set(config: RuntimeConfig) -> Result<(), String> {
    CONFIG
        .set(config)
        .map_err(|_| "runtime config already initialized".to_string())
}

pub(in crate::runtime) fn get() -> Option<&'static RuntimeConfig> {
    CONFIG.get()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tls_payload_core::PayloadDirection;

    use super::{HookPoint, RuntimeConfig, RuntimeConfigParts};
    use crate::runtime::config::plan::RuntimePlan;
    use crate::runtime::config::policy::{EventFilter, RedactionMode};
    use crate::runtime::flow_control::FlowControlConfig;

    #[test]
    fn runtime_config_registers_provider_after_plan_install() {
        let config = RuntimeConfig::from_parts(RuntimeConfigParts {
            rules: Vec::new(),
            max_payload_bytes: 4096,
            flow_control: FlowControlConfig {
                enabled: true,
                sniff_bytes: 65536,
                max_header_bytes: 16384,
                large_transfer_bytes: 1048576,
                unknown_stream_bytes: 65536,
                h2_data_probe_bytes: 65536,
            },
            redaction: RedactionMode::Redact,
            events: EventFilter::parse(Some("")).expect("empty event filter"),
            trace_id: None,
            event_transport: None,
        });
        assert_eq!(config.provider_for_symbol("SSL_write"), "unknown");
        assert!(!config.has_registered_plan());

        config
            .register_plan(&openssl_plan())
            .expect("register plan");

        assert!(config.has_registered_plan());
        assert_eq!(config.provider_for_symbol("SSL_write"), "openssl");
        assert_eq!(config.provider_for_symbol("SSL_read"), "openssl");
        assert_eq!(config.provider_for_symbol("SSL_write_ex"), "unknown");
    }

    fn openssl_plan() -> RuntimePlan {
        RuntimePlan {
            target: PathBuf::from("/usr/bin/python3"),
            binary: PathBuf::from("/usr/lib64/libssl.so.1.1"),
            provider: "openssl".to_string(),
            points: vec![
                HookPoint {
                    symbol: "SSL_write".to_string(),
                    direction: PayloadDirection::Outbound,
                    file_offset: 0,
                },
                HookPoint {
                    symbol: "SSL_read".to_string(),
                    direction: PayloadDirection::Inbound,
                    file_offset: 0,
                },
            ],
        }
    }
}
