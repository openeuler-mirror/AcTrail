//! Incremental WebSocket decoding for LLM protocols transported after an HTTP upgrade.

use std::collections::{BTreeMap, VecDeque};

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadDirection, PayloadSegment, PayloadSourceBoundary, PayloadStreamKey,
    PayloadTruncationState,
};
use model_core::process::ProcessIdentity;

use self::connection::WebSocketConnection;
use self::framing::{FrameDecoder, MAX_FRAME_BUFFER_BYTES};
use self::handshake::{ACCEPT_PREFIX, HandshakeCandidate, NegotiatedExtensions, REQUEST_PREFIX};

mod connection;
mod framing;
mod handshake;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProcessKey {
    trace_id: TraceId,
    process: ProcessIdentity,
}

pub(super) struct WebSocketExchangeStreamPrefix(String);

impl WebSocketExchangeStreamPrefix {
    fn new(value: String) -> Self {
        Self(value)
    }

    pub(super) fn matches(&self, stream_key: &str) -> bool {
        stream_key
            .strip_prefix(&self.0)
            .is_some_and(|suffix| suffix.starts_with(':'))
    }
}

#[derive(Default)]
pub(super) struct WebSocketLlmObservation {
    pub(super) projected: Vec<PayloadSegment>,
    pub(super) forgotten_exchange_streams: Vec<WebSocketExchangeStreamPrefix>,
}

pub(super) struct WebSocketLlmAdapter {
    processes: BTreeMap<ProcessKey, ProcessWebSocket>,
    max_connections_per_process: usize,
}

impl WebSocketLlmAdapter {
    pub(super) fn new(max_connections_per_process: u32) -> Self {
        Self {
            processes: BTreeMap::new(),
            max_connections_per_process: max_connections_per_process as usize,
        }
    }

    pub(super) fn observe(&mut self, segment: &PayloadSegment) -> WebSocketLlmObservation {
        if segment.source_boundary != PayloadSourceBoundary::TlsUserSpace {
            return WebSocketLlmObservation::default();
        }
        let key = ProcessKey {
            trace_id: segment.trace_id,
            process: segment.process,
        };
        if segment.truncation == PayloadTruncationState::Truncated {
            let mut observation = WebSocketLlmObservation::default();
            let empty = if let Some(process) = self.processes.get_mut(&key) {
                observation.forgotten_exchange_streams = process.forget_stream(&segment.stream_key);
                process.is_empty()
            } else {
                false
            };
            if empty {
                self.processes.remove(&key);
            }
            return observation;
        }
        let observation = self
            .processes
            .entry(key)
            .or_insert_with(|| ProcessWebSocket::new(self.max_connections_per_process))
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

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.processes.retain(|key, _| key.trace_id != trace_id);
    }

    pub(super) fn is_exchange_stream_key(stream_key: &str) -> bool {
        stream_key.starts_with("websocket:") && stream_key.contains(":exchange:")
    }
}

struct ProcessWebSocket {
    outbound_handshakes: VecDeque<HandshakeCandidate>,
    inbound_handshakes: VecDeque<HandshakeCandidate>,
    pending_offers: VecDeque<PendingOffer>,
    accepted: VecDeque<AcceptedHandshake>,
    connections: VecDeque<WebSocketConnection>,
    max_connections: usize,
}

impl ProcessWebSocket {
    fn new(max_connections: usize) -> Self {
        Self {
            outbound_handshakes: VecDeque::new(),
            inbound_handshakes: VecDeque::new(),
            pending_offers: VecDeque::new(),
            accepted: VecDeque::new(),
            connections: VecDeque::new(),
            max_connections,
        }
    }

    fn observe(&mut self, segment: &PayloadSegment) -> WebSocketLlmObservation {
        if self.observe_handshake(segment) {
            return self.activate_accepted();
        }
        if let Some(projected) = self.observe_bound_connection(segment) {
            return projected;
        }
        if self.buffer_pending_frame(segment) {
            return WebSocketLlmObservation::default();
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
                let mut projected = WebSocketLlmObservation {
                    projected: observation.projected,
                    ..WebSocketLlmObservation::default()
                };
                if !observation.closed {
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
                WebSocketLlmObservation {
                    forgotten_exchange_streams: vec![WebSocketExchangeStreamPrefix::new(
                        connection.synthetic_stream_key_prefix().to_string(),
                    )],
                    ..WebSocketLlmObservation::default()
                }
            }
        }
    }

    fn forget_stream(
        &mut self,
        stream_key: &PayloadStreamKey,
    ) -> Vec<WebSocketExchangeStreamPrefix> {
        let forgotten_exchange_streams = self
            .connections
            .iter()
            .filter(|connection| connection.is_bound_to(stream_key))
            .map(|connection| {
                WebSocketExchangeStreamPrefix::new(
                    connection.synthetic_stream_key_prefix().to_string(),
                )
            })
            .collect();
        self.outbound_handshakes
            .retain(|candidate| !candidate.is_for_stream(stream_key));
        self.inbound_handshakes
            .retain(|candidate| !candidate.is_for_stream(stream_key));
        self.pending_offers
            .retain(|offer| !offer.uses_stream(stream_key));
        self.accepted
            .retain(|accepted| !accepted.uses_stream(stream_key));
        self.connections
            .retain(|connection| !connection.is_bound_to(stream_key));
        forgotten_exchange_streams
    }

    fn is_empty(&self) -> bool {
        self.outbound_handshakes.is_empty()
            && self.inbound_handshakes.is_empty()
            && self.pending_offers.is_empty()
            && self.accepted.is_empty()
            && self.connections.is_empty()
    }

    fn buffer_pending_frame(&mut self, segment: &PayloadSegment) -> bool {
        if segment.direction != PayloadDirection::Outbound || segment.bytes.is_empty() {
            return false;
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
            return false;
        };
        let keep_offer = self
            .pending_offers
            .get_mut(position)
            .is_some_and(|offer| offer.buffer_outbound_frame(segment));
        if !keep_offer {
            self.pending_offers.remove(position);
        }
        true
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
                let mut projected = WebSocketLlmObservation {
                    projected: observation.projected,
                    ..WebSocketLlmObservation::default()
                };
                if !observation.closed {
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
                Some(WebSocketLlmObservation {
                    forgotten_exchange_streams: vec![WebSocketExchangeStreamPrefix::new(
                        connection.synthetic_stream_key_prefix().to_string(),
                    )],
                    ..WebSocketLlmObservation::default()
                })
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
            );
            let mut keep_connection = true;
            for segment in accepted.pending_frames {
                match connection.observe(&segment) {
                    Ok(Some(observation)) => {
                        projected.projected.extend(observation.projected);
                        if observation.closed {
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
        if self.accepted.pop_front().is_none() {
            self.connections.pop_front();
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

    fn observe_handshake(&mut self, segment: &PayloadSegment) -> bool {
        match segment.direction {
            PayloadDirection::Outbound => {
                let Some(position) = Self::handshake_position(
                    &mut self.outbound_handshakes,
                    segment,
                    REQUEST_PREFIX,
                    self.max_connections,
                ) else {
                    return false;
                };
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
                let Some(position) = Self::handshake_position(
                    &mut self.inbound_handshakes,
                    segment,
                    ACCEPT_PREFIX,
                    self.max_connections,
                ) else {
                    return false;
                };
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
    ) -> Option<usize> {
        if let Some(position) = candidates
            .iter()
            .position(|candidate| candidate.is_for_stream(&segment.stream_key))
        {
            return Some(position);
        }
        if segment.operation_offset != 0 || !segment.bytes.starts_with(prefix) {
            return None;
        }
        if candidates.len() >= capacity {
            candidates.pop_front();
        }
        candidates.push_back(HandshakeCandidate::default());
        Some(candidates.len() - 1)
    }

    fn push_pending_offer(&mut self, offer: PendingOffer) {
        if self.pending_offers.len() >= self.max_connections {
            self.pending_offers.pop_front();
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
