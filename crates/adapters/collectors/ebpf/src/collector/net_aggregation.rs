//! Send/recv aggregation for net events.
//!
//! Aggregates consecutive `send`/`recv` events on the same connection into a
//! single event carrying `size` (total bytes) and `cnt` (packet count). Totals
//! are preserved exactly, so anomalous high-volume writes stay visible as a
//! high-frequency, large-`size` flush stream rather than being hidden.
//!
//! The flush decision is isolated behind [`FlushPolicy`] so the strategy can be
//! swapped without touching the aggregator.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use collector_event::{RawCollectorEvent, RawEventEnvelope, RawObservationPayload};
use model_core::process::ProcessObservation;

const AGGREGATED_MARKER: &str = "aggregated";

pub struct NetAggregator {
    enabled: Arc<AtomicBool>,
    policy: ThresholdFlushPolicy,
    states: HashMap<AggKey, AggState>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct AggKey {
    process: ProcessObservation,
    descriptor: DescriptorIdentity,
    transport: String,
    local: Option<String>,
    remote: Option<String>,
    direction: String,
    operation: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum DescriptorIdentity {
    Generation(u64),
    Fd(String),
}

struct AggState {
    envelope: RawEventEnvelope,
    transport: String,
    local: Option<String>,
    remote: Option<String>,
    direction: String,
    operation: String,
    bytes_sum: u64,
    cnt: u64,
    last_observed_at: SystemTime,
}

pub enum ObserveOutcome {
    /// Not a send/recv event, or aggregation disabled — forward unchanged.
    PassThrough(RawCollectorEvent),
    /// Event absorbed into the aggregation state.
    Buffered,
    /// A flush was triggered, emitting the aggregated event.
    Flushed(RawCollectorEvent),
}

pub trait FlushPolicy {
    /// Decide whether an aggregated connection should be flushed, given its
    /// running count, byte total, and the time it was last observed.
    fn should_flush(
        &self,
        cnt: u64,
        bytes: u64,
        last_observed_at: SystemTime,
        now: SystemTime,
    ) -> bool;
}

pub struct ThresholdFlushPolicy {
    pub max_cnt: u64,
    pub max_bytes: u64,
    pub max_idle: Duration,
}

impl Default for ThresholdFlushPolicy {
    fn default() -> Self {
        Self {
            max_cnt: 64,
            max_bytes: 64 * 1024,
            max_idle: Duration::from_millis(500),
        }
    }
}

impl FlushPolicy for ThresholdFlushPolicy {
    fn should_flush(
        &self,
        cnt: u64,
        bytes: u64,
        last_observed_at: SystemTime,
        now: SystemTime,
    ) -> bool {
        cnt >= self.max_cnt
            || bytes >= self.max_bytes
            || now.duration_since(last_observed_at).unwrap_or_default() >= self.max_idle
    }
}

impl NetAggregator {
    pub fn new(enabled: Arc<AtomicBool>) -> Self {
        Self {
            enabled,
            policy: ThresholdFlushPolicy::default(),
            states: HashMap::new(),
        }
    }

    /// Observe a net event, aggregating `send`/`recv` and passing through the rest.
    pub fn observe(
        &mut self,
        event: RawCollectorEvent,
        descriptor_generation: u64,
    ) -> ObserveOutcome {
        if !self.enabled.load(Ordering::Relaxed) {
            return ObserveOutcome::PassThrough(event);
        }
        let Some((key, state)) = Self::ingest_state(&event, descriptor_generation) else {
            return ObserveOutcome::PassThrough(event);
        };
        match self.states.remove(&key) {
            Some(mut existing) => {
                existing.bytes_sum = existing.bytes_sum.saturating_add(state.bytes_sum);
                existing.cnt = existing.cnt.saturating_add(state.cnt);
                existing.last_observed_at = state.last_observed_at;
                if self.policy.should_flush(
                    existing.cnt,
                    existing.bytes_sum,
                    existing.last_observed_at,
                    state.last_observed_at,
                ) {
                    ObserveOutcome::Flushed(existing.into_event())
                } else {
                    self.states.insert(key, existing);
                    ObserveOutcome::Buffered
                }
            }
            None => {
                if self.policy.should_flush(
                    state.cnt,
                    state.bytes_sum,
                    state.last_observed_at,
                    state.last_observed_at,
                ) {
                    ObserveOutcome::Flushed(state.into_event())
                } else {
                    self.states.insert(key, state);
                    ObserveOutcome::Buffered
                }
            }
        }
    }

    /// Flush states whose idle timeout has elapsed.
    pub fn drain_timeout(&mut self, now: SystemTime) -> Vec<RawCollectorEvent> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let timed_out: Vec<AggKey> = self
            .states
            .iter()
            .filter(|(_, state)| {
                self.policy
                    .should_flush(state.cnt, state.bytes_sum, state.last_observed_at, now)
            })
            .map(|(key, _)| key.clone())
            .collect();
        timed_out
            .into_iter()
            .filter_map(|key| self.states.remove(&key).map(AggState::into_event))
            .collect()
    }

    /// Flush all buffered send/recv states for one connection, identified by
    /// process + FD-object generation + local/remote endpoints, regardless of
    /// direction or operation.
    /// A net close/shutdown event calls this so a finished connection is
    /// emitted immediately instead of waiting on the idle timeout.
    pub fn flush_connection(
        &mut self,
        envelope: &RawEventEnvelope,
        transport: &str,
        local: &Option<String>,
        remote: &Option<String>,
        descriptor_generation: u64,
        fd: Option<&str>,
    ) -> Vec<RawCollectorEvent> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let Some(descriptor) = Self::descriptor_identity(descriptor_generation, fd) else {
            return Vec::new();
        };
        [("outbound", "send"), ("inbound", "recv")]
            .into_iter()
            .map(|(direction, operation)| AggKey {
                process: envelope.process.clone(),
                descriptor: descriptor.clone(),
                transport: transport.to_string(),
                local: local.clone(),
                remote: remote.clone(),
                direction: direction.to_string(),
                operation: operation.to_string(),
            })
            .filter_map(|key| self.states.remove(&key).map(AggState::into_event))
            .collect()
    }

    /// Flush all states belonging to a trace (process exit / trace close).
    pub fn flush_trace(&mut self, trace_id: model_core::ids::TraceId) -> Vec<RawCollectorEvent> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Vec::new();
        }
        let keys: Vec<AggKey> = self
            .states
            .iter()
            .filter(|(_, state)| state.envelope.trace_id == Some(trace_id))
            .map(|(key, _)| key.clone())
            .collect();
        keys.into_iter()
            .filter_map(|key| self.states.remove(&key).map(AggState::into_event))
            .collect()
    }

    /// Build the aggregation key + initial state for a send/recv net event.
    fn ingest_state(
        event: &RawCollectorEvent,
        descriptor_generation: u64,
    ) -> Option<(AggKey, AggState)> {
        let observed_at = event.envelope.observed_at;
        let RawObservationPayload::Net {
            transport,
            local,
            remote,
            size,
            metadata,
            ..
        } = &event.payload
        else {
            return None;
        };
        let operation = metadata.get("operation")?;
        if operation != "send" && operation != "recv" {
            return None;
        }
        let direction = metadata.get("direction").cloned().unwrap_or_default();
        let descriptor = Self::descriptor_identity(
            descriptor_generation,
            metadata.get("fd").map(String::as_str),
        )?;
        let key = AggKey {
            process: event.envelope.process.clone(),
            descriptor,
            transport: transport.clone(),
            local: local.clone(),
            remote: remote.clone(),
            direction: direction.clone(),
            operation: operation.clone(),
        };
        let state = AggState {
            envelope: event.envelope.clone(),
            transport: transport.clone(),
            local: local.clone(),
            remote: remote.clone(),
            direction,
            operation: operation.clone(),
            bytes_sum: size.unwrap_or(0),
            cnt: 1,
            last_observed_at: observed_at,
        };
        Some((key, state))
    }

    fn descriptor_identity(
        generation: u64,
        fallback_fd: Option<&str>,
    ) -> Option<DescriptorIdentity> {
        if generation != 0 {
            Some(DescriptorIdentity::Generation(generation))
        } else {
            fallback_fd.map(|fd| DescriptorIdentity::Fd(fd.to_string()))
        }
    }
}

impl AggState {
    fn into_event(self) -> RawCollectorEvent {
        let mut metadata = BTreeMap::new();
        metadata.insert("operation".to_string(), self.operation);
        metadata.insert("direction".to_string(), self.direction);
        metadata.insert("cnt".to_string(), self.cnt.to_string());
        metadata.insert(AGGREGATED_MARKER.to_string(), "true".to_string());
        RawCollectorEvent {
            envelope: RawEventEnvelope {
                observed_at: self.last_observed_at,
                ..self.envelope
            },
            payload: RawObservationPayload::Net {
                transport: self.transport,
                local: self.local,
                remote: self.remote,
                size: Some(self.bytes_sum),
                result: None,
                metadata,
            },
        }
    }
}
