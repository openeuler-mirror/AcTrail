//! Incremental WebSocket decoding for LLM protocols transported after an HTTP upgrade.

use std::collections::{BTreeMap, VecDeque};
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadDirection, PayloadSegment, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;

use super::connection::{ConnectionObservation, WebSocketConnection};
use super::framing::{FrameDecoder, MAX_FRAME_BUFFER_BYTES};
use super::handshake::{ACCEPT_PREFIX, HandshakeCandidate, NegotiatedExtensions, REQUEST_PREFIX};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProcessKey {
    trace_id: TraceId,
    process: ProcessIdentity,
}

pub(in crate::llm_pipeline) struct WebSocketExchangeStreamPrefix(String);

pub(in crate::llm_pipeline) struct WebSocketTraceFinalization {
    pub(in crate::llm_pipeline) process: ProcessIdentity,
    pub(in crate::llm_pipeline) observation: WebSocketLlmObservation,
}

impl WebSocketExchangeStreamPrefix {
    fn new(value: String) -> Self {
        Self(value)
    }

    pub(in crate::llm_pipeline) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Default)]
pub(in crate::llm_pipeline) struct WebSocketLlmObservation {
    pub(in crate::llm_pipeline) projected: Vec<PayloadSegment>,
    pub(in crate::llm_pipeline) completed_exchange_streams: Vec<PayloadStreamKey>,
    pub(in crate::llm_pipeline) partial_exchange_streams: Vec<PayloadStreamKey>,
    pub(in crate::llm_pipeline) forgotten_exchange_streams: Vec<WebSocketExchangeStreamPrefix>,
    pub(in crate::llm_pipeline) capacity_evicted_entries: u64,
    pub(in crate::llm_pipeline) capacity_evicted_bytes: u64,
    pub(in crate::llm_pipeline) buffered_frame_discarded_bytes: u64,
    pub(in crate::llm_pipeline) oversized_response_discarded_bytes: u64,
    pub(in crate::llm_pipeline) superseded_responses: u64,
    pub(in crate::llm_pipeline) decode_failed_entries: u64,
    pub(in crate::llm_pipeline) decode_discarded_bytes: u64,
    pub(in crate::llm_pipeline) lifecycle_gap_entries: u64,
}

impl WebSocketLlmObservation {
    fn extend_connection(&mut self, observation: ConnectionObservation) {
        self.projected.extend(observation.projected);
        self.completed_exchange_streams
            .extend(observation.completed_exchange_streams);
        self.partial_exchange_streams
            .extend(observation.partial_exchange_streams);
        self.oversized_response_discarded_bytes = self
            .oversized_response_discarded_bytes
            .saturating_add(observation.oversized_response_discarded_bytes);
        self.superseded_responses = self
            .superseded_responses
            .saturating_add(observation.superseded_responses);
        self.decode_failed_entries = self
            .decode_failed_entries
            .saturating_add(observation.decode_failed_entries);
        self.decode_discarded_bytes = self
            .decode_discarded_bytes
            .saturating_add(observation.decode_discarded_bytes);
        self.lifecycle_gap_entries = self
            .lifecycle_gap_entries
            .saturating_add(observation.lifecycle_gap_entries);
    }
}

pub(in crate::llm_pipeline) struct WebSocketLlmAdapter {
    processes: BTreeMap<ProcessKey, ProcessWebSocket>,
    max_connections_per_process: usize,
    max_response_bytes: usize,
}

impl WebSocketLlmAdapter {
    pub(in crate::llm_pipeline) fn new(
        max_connections_per_process: u32,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            processes: BTreeMap::new(),
            max_connections_per_process: max_connections_per_process as usize,
            max_response_bytes,
        }
    }

    pub(in crate::llm_pipeline) fn observe(
        &mut self,
        segment: &PayloadSegment,
    ) -> WebSocketLlmObservation {
        if segment.source_boundary != PayloadSourceBoundary::TlsUserSpace {
            return WebSocketLlmObservation::default();
        }
        let key = ProcessKey {
            trace_id: segment.trace_id,
            process: segment.process,
        };
        if segment.truncation == PayloadTruncationState::Truncated {
            let (observation, empty) = if let Some(process) = self.processes.get_mut(&key) {
                let observation = process.forget_stream(segment);
                (observation, process.is_empty())
            } else {
                (WebSocketLlmObservation::default(), false)
            };
            if empty {
                self.processes.remove(&key);
            }
            return observation;
        }
        let observation = self
            .processes
            .entry(key)
            .or_insert_with(|| {
                ProcessWebSocket::new(self.max_connections_per_process, self.max_response_bytes)
            })
            .observe(segment);
        if self
            .processes
            .get(&key)
            .is_some_and(ProcessWebSocket::is_empty)
        {
            self.processes.remove(&key);
        }
        observation
    }

    pub(in crate::llm_pipeline) fn forget_trace(&mut self, trace_id: TraceId) {
        self.processes.retain(|key, _| key.trace_id != trace_id);
    }

    pub(in crate::llm_pipeline) fn finalize_trace(
        &mut self,
        trace_id: TraceId,
        finished_at: SystemTime,
    ) -> Vec<WebSocketTraceFinalization> {
        let keys = self
            .processes
            .keys()
            .filter(|key| key.trace_id == trace_id)
            .copied()
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter_map(|key| {
                let process = self.processes.remove(&key)?;
                Some(WebSocketTraceFinalization {
                    process: key.process,
                    observation: process.finalize_trace(finished_at),
                })
            })
            .collect()
    }

    pub(in crate::llm_pipeline) fn is_exchange_stream_key(stream_key: &str) -> bool {
        stream_key.starts_with("websocket:") && stream_key.contains(":exchange:")
    }

    pub(in crate::llm_pipeline) fn exchange_stream_prefix(stream_key: &str) -> Option<&str> {
        let (prefix, exchange_id) = stream_key.rsplit_once(':')?;
        (prefix.starts_with("websocket:")
            && prefix.ends_with(":exchange")
            && exchange_id.parse::<u64>().is_ok())
        .then_some(prefix)
    }
}

struct ProcessWebSocket {
    outbound_handshakes: VecDeque<HandshakeCandidate>,
    inbound_handshakes: VecDeque<HandshakeCandidate>,
    pending_offers: VecDeque<PendingOffer>,
    accepted: VecDeque<AcceptedHandshake>,
    connections: VecDeque<WebSocketConnection>,
    max_connections: usize,
    max_response_bytes: usize,
    evicted_exchange_streams: Vec<WebSocketExchangeStreamPrefix>,
    capacity_evicted_entries: u64,
    capacity_evicted_bytes: u64,
}

impl ProcessWebSocket {
    fn new(max_connections: usize, max_response_bytes: usize) -> Self {
        Self {
            outbound_handshakes: VecDeque::new(),
            inbound_handshakes: VecDeque::new(),
            pending_offers: VecDeque::new(),
            accepted: VecDeque::new(),
            connections: VecDeque::new(),
            max_connections,
            max_response_bytes,
            evicted_exchange_streams: Vec::new(),
            capacity_evicted_entries: 0,
            capacity_evicted_bytes: 0,
        }
    }

    fn observe(&mut self, segment: &PayloadSegment) -> WebSocketLlmObservation {
        let mut observation = self.observe_inner(segment);
        observation
            .forgotten_exchange_streams
            .append(&mut self.evicted_exchange_streams);
        observation.capacity_evicted_entries = self.capacity_evicted_entries;
        observation.capacity_evicted_bytes = self.capacity_evicted_bytes;
        self.capacity_evicted_entries = 0;
        self.capacity_evicted_bytes = 0;
        observation
    }

    fn observe_inner(&mut self, segment: &PayloadSegment) -> WebSocketLlmObservation {
        if self.observe_handshake(segment) {
            return self.activate_accepted();
        }
        if let Some(projected) = self.observe_bound_connection(segment) {
            return projected;
        }
        if let Some(observation) = self.buffer_pending_frame(segment) {
            return observation;
        }
        let expected_masked = segment.direction == PayloadDirection::Outbound;
        if self.accepted.is_empty()
            || !FrameDecoder::looks_like_frame(&segment.bytes, expected_masked)
        {
            return WebSocketLlmObservation::default();
        }
        let Some(accepted) = self.accepted.pop_front() else {
            return WebSocketLlmObservation::default();
        };
        let mut connection = WebSocketConnection::new(
            accepted.outbound_stream_key,
            accepted.inbound_stream_key,
            accepted.path,
            accepted.extensions,
            self.max_response_bytes,
        );
        tracing::debug!(
            target: "actrail::semantic_projection",
            direction = ?segment.direction,
            stream_key = %segment.stream_key,
            bytes = segment.bytes.len(),
            "created WebSocket LLM connection"
        );
        match connection.observe(segment) {
            Ok(Some(observation)) => {
                let closed = observation.closed;
                let mut projected = WebSocketLlmObservation::default();
                projected.extend_connection(observation);
                if !closed {
                    self.push_connection(connection);
                } else {
                    projected
                        .forgotten_exchange_streams
                        .push(WebSocketExchangeStreamPrefix::new(
                            connection.synthetic_stream_key_prefix().to_string(),
                        ));
                }
                projected
            }
            Ok(None) => WebSocketLlmObservation::default(),
            Err(()) => {
                Self::warn_invalid_connection(segment);
                let mut projected = WebSocketLlmObservation::default();
                projected.extend_connection(connection.materialize_decode_failure(segment));
                projected
                    .forgotten_exchange_streams
                    .push(WebSocketExchangeStreamPrefix::new(
                        connection.synthetic_stream_key_prefix().to_string(),
                    ));
                projected
            }
        }
    }

    fn forget_stream(&mut self, segment: &PayloadSegment) -> WebSocketLlmObservation {
        let stream_key = &segment.stream_key;
        let mut observation = WebSocketLlmObservation::default();
        let pending_before = self
            .outbound_handshakes
            .len()
            .saturating_add(self.inbound_handshakes.len())
            .saturating_add(self.pending_offers.len())
            .saturating_add(self.accepted.len());
        self.outbound_handshakes
            .retain(|candidate| !candidate.is_for_stream(stream_key));
        self.inbound_handshakes
            .retain(|candidate| !candidate.is_for_stream(stream_key));
        self.pending_offers
            .retain(|offer| !offer.uses_stream(stream_key));
        self.accepted
            .retain(|accepted| !accepted.uses_stream(stream_key));
        let pending_after = self
            .outbound_handshakes
            .len()
            .saturating_add(self.inbound_handshakes.len())
            .saturating_add(self.pending_offers.len())
            .saturating_add(self.accepted.len());
        observation.lifecycle_gap_entries = observation.lifecycle_gap_entries.saturating_add(
            u64::try_from(pending_before.saturating_sub(pending_after)).unwrap_or(u64::MAX),
        );
        let mut retained = VecDeque::with_capacity(self.connections.len());
        while let Some(mut connection) = self.connections.pop_front() {
            if connection.is_bound_to(stream_key) {
                observation
                    .forgotten_exchange_streams
                    .push(WebSocketExchangeStreamPrefix::new(
                        connection.synthetic_stream_key_prefix().to_string(),
                    ));
                observation.extend_connection(connection.materialize_lifecycle_gap(segment));
            } else {
                retained.push_back(connection);
            }
        }
        self.connections = retained;
        observation
    }

    fn is_empty(&self) -> bool {
        self.outbound_handshakes.is_empty()
            && self.inbound_handshakes.is_empty()
            && self.pending_offers.is_empty()
            && self.accepted.is_empty()
            && self.connections.is_empty()
    }

    fn finalize_trace(mut self, finished_at: SystemTime) -> WebSocketLlmObservation {
        let mut observation = WebSocketLlmObservation::default();
        observation.lifecycle_gap_entries = u64::try_from(
            self.outbound_handshakes
                .len()
                .saturating_add(self.inbound_handshakes.len())
                .saturating_add(self.pending_offers.len())
                .saturating_add(self.accepted.len()),
        )
        .unwrap_or(u64::MAX);
        while let Some(mut connection) = self.connections.pop_front() {
            observation
                .forgotten_exchange_streams
                .push(WebSocketExchangeStreamPrefix::new(
                    connection.synthetic_stream_key_prefix().to_string(),
                ));
            if let Some(finalized) = connection.materialize_trace_close(finished_at) {
                observation.extend_connection(finalized);
            }
        }
        observation
    }

    fn buffer_pending_frame(
        &mut self,
        segment: &PayloadSegment,
    ) -> Option<WebSocketLlmObservation> {
        if segment.direction != PayloadDirection::Outbound || segment.bytes.is_empty() {
            return None;
        }
        let position = self
            .pending_offers
            .iter()
            .position(|offer| offer.early_outbound_stream_key.as_ref() == Some(&segment.stream_key))
            .or_else(|| {
                FrameDecoder::looks_like_frame(&segment.bytes, true).then(|| {
                    self.pending_offers
                        .iter()
                        .position(|offer| offer.early_outbound_stream_key.is_none())
                })?
            });
        let Some(position) = position else {
            return None;
        };
        let keep_offer = self
            .pending_offers
            .get_mut(position)
            .is_some_and(|offer| offer.buffer_outbound_frame(segment));
        if !keep_offer {
            let discarded_bytes =
                self.pending_offers
                    .remove(position)
                    .map_or(segment.bytes.len(), |offer| {
                        offer
                            .pending_frame_bytes
                            .saturating_add(segment.bytes.len())
                    });
            return Some(WebSocketLlmObservation {
                buffered_frame_discarded_bytes: discarded_bytes as u64,
                ..WebSocketLlmObservation::default()
            });
        }
        Some(WebSocketLlmObservation::default())
    }

    fn observe_bound_connection(
        &mut self,
        segment: &PayloadSegment,
    ) -> Option<WebSocketLlmObservation> {
        let position = self
            .connections
            .iter()
            .position(|connection| connection.is_bound_to(&segment.stream_key))?;
        let mut connection = self.connections.remove(position)?;
        match connection.observe(segment) {
            Ok(Some(observation)) => {
                let closed = observation.closed;
                let mut projected = WebSocketLlmObservation::default();
                projected.extend_connection(observation);
                if !closed {
                    self.push_connection(connection);
                } else {
                    projected
                        .forgotten_exchange_streams
                        .push(WebSocketExchangeStreamPrefix::new(
                            connection.synthetic_stream_key_prefix().to_string(),
                        ));
                }
                Some(projected)
            }
            Ok(None) => {
                self.push_connection(connection);
                Some(WebSocketLlmObservation::default())
            }
            Err(()) => {
                Self::warn_invalid_connection(segment);
                let mut projected = WebSocketLlmObservation::default();
                projected.extend_connection(connection.materialize_decode_failure(segment));
                projected
                    .forgotten_exchange_streams
                    .push(WebSocketExchangeStreamPrefix::new(
                        connection.synthetic_stream_key_prefix().to_string(),
                    ));
                Some(projected)
            }
        }
    }

    fn activate_accepted(&mut self) -> WebSocketLlmObservation {
        let mut projected = WebSocketLlmObservation::default();
        while let Some(position) = self
            .accepted
            .iter()
            .position(|accepted| !accepted.pending_frames.is_empty())
        {
            let Some(accepted) = self.accepted.remove(position) else {
                break;
            };
            let mut connection = WebSocketConnection::new(
                accepted.outbound_stream_key,
                accepted.inbound_stream_key,
                accepted.path,
                accepted.extensions,
                self.max_response_bytes,
            );
            let mut keep_connection = true;
            for segment in accepted.pending_frames {
                match connection.observe(&segment) {
                    Ok(Some(observation)) => {
                        let closed = observation.closed;
                        projected.extend_connection(observation);
                        if closed {
                            projected.forgotten_exchange_streams.push(
                                WebSocketExchangeStreamPrefix::new(
                                    connection.synthetic_stream_key_prefix().to_string(),
                                ),
                            );
                            keep_connection = false;
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(()) => {
                        projected
                            .extend_connection(connection.materialize_decode_failure(&segment));
                        projected.forgotten_exchange_streams.push(
                            WebSocketExchangeStreamPrefix::new(
                                connection.synthetic_stream_key_prefix().to_string(),
                            ),
                        );
                        keep_connection = false;
                        break;
                    }
                }
            }
            if keep_connection {
                self.push_connection(connection);
            }
        }
        projected
    }

    fn push_connection(&mut self, connection: WebSocketConnection) {
        self.reserve_connection_slot();
        self.connections.push_back(connection);
    }

    fn reserve_connection_slot(&mut self) {
        if self.connections.len().saturating_add(self.accepted.len()) < self.max_connections {
            return;
        }
        if let Some(accepted) = self.accepted.pop_front() {
            self.record_capacity_eviction(accepted.pending_frame_bytes);
        } else if let Some(connection) = self.connections.pop_front() {
            self.record_capacity_eviction(connection.retained_response_bytes());
            self.evicted_exchange_streams
                .push(WebSocketExchangeStreamPrefix::new(
                    connection.synthetic_stream_key_prefix().to_string(),
                ));
        }
    }

    fn warn_invalid_connection(segment: &PayloadSegment) {
        tracing::warn!(
            target: "actrail::semantic_projection",
            direction = ?segment.direction,
            stream_key = %segment.stream_key,
            bytes = segment.bytes.len(),
            "discarded invalid WebSocket LLM connection state"
        );
    }

    fn record_capacity_eviction(&mut self, discarded_bytes: usize) {
        self.capacity_evicted_entries = self.capacity_evicted_entries.saturating_add(1);
        self.capacity_evicted_bytes = self
            .capacity_evicted_bytes
            .saturating_add(discarded_bytes as u64);
    }

    fn observe_handshake(&mut self, segment: &PayloadSegment) -> bool {
        match segment.direction {
            PayloadDirection::Outbound => {
                let Some((position, eviction)) = Self::handshake_position(
                    &mut self.outbound_handshakes,
                    segment,
                    REQUEST_PREFIX,
                    self.max_connections,
                ) else {
                    return false;
                };
                if let Some(discarded_bytes) = eviction {
                    self.record_capacity_eviction(discarded_bytes);
                }
                let candidate = &mut self.outbound_handshakes[position];
                let observed = candidate.observe(segment, REQUEST_PREFIX);
                if let Some((path, stream_key)) = candidate.request_path() {
                    self.outbound_handshakes.remove(position);
                    self.push_pending_offer(PendingOffer {
                        path,
                        outbound_stream_key: stream_key,
                        early_outbound_stream_key: None,
                        pending_frames: Vec::new(),
                        pending_frame_bytes: 0,
                    });
                }
                observed
            }
            PayloadDirection::Inbound => {
                let Some((position, eviction)) = Self::handshake_position(
                    &mut self.inbound_handshakes,
                    segment,
                    ACCEPT_PREFIX,
                    self.max_connections,
                ) else {
                    return false;
                };
                if let Some(discarded_bytes) = eviction {
                    self.record_capacity_eviction(discarded_bytes);
                }
                let candidate = &mut self.inbound_handshakes[position];
                let observed = candidate.observe(segment, ACCEPT_PREFIX);
                if let Some((extensions, inbound_stream_key)) = candidate.accepted_extensions() {
                    self.inbound_handshakes.remove(position);
                    if let Some(offer) = self.pending_offers.pop_front() {
                        tracing::debug!(
                            target: "actrail::semantic_projection",
                            outbound_stream_key = %offer.outbound_stream_key,
                            inbound_stream_key = %inbound_stream_key,
                            "accepted WebSocket LLM handshake"
                        );
                        self.push_accepted(AcceptedHandshake {
                            path: offer.path,
                            outbound_stream_key: offer.outbound_stream_key,
                            inbound_stream_key,
                            extensions,
                            pending_frames: offer.pending_frames,
                            pending_frame_bytes: offer.pending_frame_bytes,
                        });
                    }
                }
                observed
            }
        }
    }

    fn handshake_position(
        candidates: &mut VecDeque<HandshakeCandidate>,
        segment: &PayloadSegment,
        prefix: &[u8],
        capacity: usize,
    ) -> Option<(usize, Option<usize>)> {
        if let Some(position) = candidates
            .iter()
            .position(|candidate| candidate.is_for_stream(&segment.stream_key))
        {
            return Some((position, None));
        }
        if segment.operation_offset != 0 || !segment.bytes.starts_with(prefix) {
            return None;
        }
        let eviction = (candidates.len() >= capacity)
            .then(|| {
                candidates
                    .pop_front()
                    .map(|candidate| candidate.buffered_bytes())
            })
            .flatten();
        candidates.push_back(HandshakeCandidate::default());
        Some((candidates.len() - 1, eviction))
    }

    fn push_pending_offer(&mut self, offer: PendingOffer) {
        if self.pending_offers.len() >= self.max_connections {
            if let Some(evicted) = self.pending_offers.pop_front() {
                self.record_capacity_eviction(evicted.pending_frame_bytes);
            }
        }
        self.pending_offers.push_back(offer);
    }

    fn push_accepted(&mut self, accepted: AcceptedHandshake) {
        self.reserve_connection_slot();
        self.accepted.push_back(accepted);
    }
}

struct PendingOffer {
    path: String,
    outbound_stream_key: PayloadStreamKey,
    early_outbound_stream_key: Option<PayloadStreamKey>,
    pending_frames: Vec<PayloadSegment>,
    pending_frame_bytes: usize,
}

impl PendingOffer {
    fn uses_stream(&self, stream_key: &PayloadStreamKey) -> bool {
        &self.outbound_stream_key == stream_key
            || self.early_outbound_stream_key.as_ref() == Some(stream_key)
            || self
                .pending_frames
                .first()
                .is_some_and(|segment| &segment.stream_key == stream_key)
    }

    fn buffer_outbound_frame(&mut self, segment: &PayloadSegment) -> bool {
        if segment.direction != PayloadDirection::Outbound || segment.bytes.is_empty() {
            return true;
        }
        if let Some(stream_key) = self.early_outbound_stream_key.as_ref() {
            if stream_key != &segment.stream_key {
                return true;
            }
        } else {
            if !FrameDecoder::looks_like_frame(&segment.bytes, true) {
                return true;
            }
            self.early_outbound_stream_key = Some(segment.stream_key.clone());
        }
        let Some(next_bytes) = self.pending_frame_bytes.checked_add(segment.bytes.len()) else {
            return false;
        };
        if next_bytes > MAX_FRAME_BUFFER_BYTES {
            return false;
        }
        self.pending_frames.push(segment.clone());
        self.pending_frame_bytes = next_bytes;
        true
    }
}

struct AcceptedHandshake {
    path: String,
    outbound_stream_key: PayloadStreamKey,
    inbound_stream_key: PayloadStreamKey,
    extensions: NegotiatedExtensions,
    pending_frames: Vec<PayloadSegment>,
    pending_frame_bytes: usize,
}

impl AcceptedHandshake {
    fn uses_stream(&self, stream_key: &PayloadStreamKey) -> bool {
        &self.outbound_stream_key == stream_key
            || &self.inbound_stream_key == stream_key
            || self
                .pending_frames
                .first()
                .is_some_and(|segment| &segment.stream_key == stream_key)
    }
}
