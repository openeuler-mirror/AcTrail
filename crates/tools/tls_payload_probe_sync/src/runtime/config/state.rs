use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tls_payload_core::{
    CoreError, Decision, EqualLenRewriteProcessor, PayloadContext, PayloadDirection, RewriteRule,
    SyncProcessor,
};
use tls_payload_sync::{EventClient, EventClientConfig, EventRegistration, EventTransportFds, SyncEvent};

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
pub(super) struct EventTransportConfig {
    pub(super) client: EventClientConfig,
    pub(super) path: PathBuf,
    pub(super) registration: EventRegistration,
}

impl EventTransportConfig {
    fn connect(&self, transport: &ProcessTransport) -> Result<EventClient, String> {
        EventClient::connect(
            &self.path,
            self.client,
            self.registration.clone(),
            Arc::clone(&transport.fds),
        )
        .map_err(|error| format!("connect sync event socket {}: {error}", self.path.display()))
    }
}

#[derive(Debug)]
struct ProcessTransport {
    client: OnceLock<Result<EventClient, String>>,
    fds: Arc<EventTransportFds>,
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
    process_transport: AtomicPtr<ProcessTransport>,
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
            process_transport: AtomicPtr::new(std::ptr::null_mut()),
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

    fn transport(&self) -> &ProcessTransport {
        let mut current = self.process_transport.load(Ordering::Acquire);
        if current.is_null() {
            let created = Box::into_raw(Box::new(ProcessTransport {
                client: OnceLock::new(),
                fds: Arc::new(EventTransportFds::default()),
            }));
            current = match self.process_transport.compare_exchange(
                std::ptr::null_mut(),
                created,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => created,
                Err(existing) => {
                    unsafe { drop(Box::from_raw(created)) };
                    existing
                }
            };
        }
        // Published transports live for the process lifetime. A fork child abandons the copy.
        unsafe { &*current }
    }

    pub(in crate::runtime) fn send_event(&self, event: SyncEvent) -> Result<(), String> {
        let Some(config) = &self.event_transport else {
            return Ok(());
        };
        let transport = self.transport();
        let client = transport.client.get_or_init(|| config.connect(transport));
        match client {
            Ok(client) => client.send(event).map_err(|error| error.to_string()),
            Err(error) => Err(error.clone()),
        }
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
        let current = self.process_transport.load(Ordering::Acquire);
        if current.is_null() {
            return Ok(());
        }
        let transport = unsafe { &*current };
        match transport.client.get() {
            Some(Ok(client)) => client.close().map_err(|error| error.to_string()),
            _ => Ok(()),
        }
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

unsafe extern "C" fn after_fork_child() {
    if let Some(config) = CONFIG.get() {
        let previous = config
            .process_transport
            .swap(std::ptr::null_mut(), Ordering::Relaxed);
        if !previous.is_null() {
            // No inherited mutex, Arc refcount or destructor is touched here.
            unsafe { &*previous }.fds.close_after_fork();
        }
    }
}

pub(in crate::runtime) fn set(config: RuntimeConfig) -> Result<(), String> {
    let result = unsafe { libc::pthread_atfork(None, None, Some(after_fork_child)) };
    if result != 0 {
        return Err(format!(
            "register TLS-sync fork handler: {}",
            std::io::Error::from_raw_os_error(result)
        ));
    }
    CONFIG
        .set(config)
        .map_err(|_| "runtime config already initialized".to_string())
}

pub(in crate::runtime) fn get() -> Option<&'static RuntimeConfig> {
    CONFIG.get()
}
