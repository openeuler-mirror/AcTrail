use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tls_payload_core::{
    CoreError, Decision, EqualLenRewriteProcessor, PayloadContext, PayloadDirection, RewriteRule,
    SyncProcessor,
};
use tls_payload_sync::{EventClient, SyncEvent};

use super::policy::{EventFilter, RedactionMode};

static CONFIG: OnceLock<RuntimeConfig> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct HookPoint {
    pub(in crate::runtime) symbol: String,
    pub(in crate::runtime) direction: PayloadDirection,
    pub(in crate::runtime) file_offset: u64,
}

pub(super) struct RuntimeConfigParts {
    pub(super) inline_hooks: bool,
    pub(super) binary: PathBuf,
    pub(super) provider: String,
    pub(super) points: Vec<HookPoint>,
    pub(super) rules: Vec<RewriteRule>,
    pub(super) max_payload_bytes: usize,
    pub(super) redaction: RedactionMode,
    pub(super) events: EventFilter,
    pub(super) trace_id: Option<u64>,
    pub(super) event_client: Option<EventClient>,
}

#[derive(Debug)]
pub(in crate::runtime) struct RuntimeConfig {
    inline_hooks: bool,
    binary: PathBuf,
    provider: String,
    points: Vec<HookPoint>,
    processor: Mutex<EqualLenRewriteProcessor>,
    max_payload_bytes: usize,
    redaction: RedactionMode,
    events: EventFilter,
    trace_id: Option<u64>,
    event_client: Option<EventClient>,
    sequence: AtomicU64,
}

impl RuntimeConfig {
    pub(super) fn from_parts(parts: RuntimeConfigParts) -> Self {
        Self {
            inline_hooks: parts.inline_hooks,
            binary: parts.binary,
            provider: parts.provider,
            points: parts.points,
            processor: Mutex::new(EqualLenRewriteProcessor::new(parts.rules)),
            max_payload_bytes: parts.max_payload_bytes,
            redaction: parts.redaction,
            events: parts.events,
            trace_id: parts.trace_id,
            event_client: parts.event_client,
            sequence: AtomicU64::new(0),
        }
    }

    pub(in crate::runtime) fn binary(&self) -> &PathBuf {
        &self.binary
    }

    pub(in crate::runtime) fn inline_hooks(&self) -> bool {
        self.inline_hooks
    }

    pub(in crate::runtime) fn provider(&self) -> &str {
        &self.provider
    }

    pub(in crate::runtime) fn points(&self) -> &[HookPoint] {
        &self.points
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

    pub(in crate::runtime) fn send_event(&self, event: SyncEvent) -> Result<(), String> {
        let Some(client) = &self.event_client else {
            return Ok(());
        };
        client.send(event).map_err(|error| error.to_string())
    }

    pub(in crate::runtime) fn redact_payload(&self, payload: &[u8]) -> String {
        match self.redaction {
            RedactionMode::Redact => format!("<redacted bytes={}>", payload.len()),
            RedactionMode::None => String::from_utf8_lossy(payload).into_owned(),
        }
    }

    pub(in crate::runtime) fn decide(
        &self,
        symbol: &str,
        direction: PayloadDirection,
        stream_key: usize,
        payload: &[u8],
    ) -> Result<Decision, CoreError> {
        let context = PayloadContext {
            direction,
            provider: &self.provider,
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
