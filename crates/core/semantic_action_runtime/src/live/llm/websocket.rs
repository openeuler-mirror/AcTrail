//! Incremental WebSocket decoding for LLM protocols transported after an HTTP upgrade.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadDirection, PayloadSegment, PayloadSourceBoundary, PayloadTruncationState,
};
use model_core::process::ProcessIdentity;

use self::connection::WebSocketConnection;
use self::framing::FrameDecoder;
use self::handshake::{ACCEPT_PREFIX, HandshakeCandidate, NegotiatedExtensions, REQUEST_PREFIX};

mod connection;
mod framing;
mod handshake;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProcessKey {
    trace_id: TraceId,
    process: ProcessIdentity,
}

#[derive(Default)]
pub(super) struct WebSocketLlmAdapter {
    processes: BTreeMap<ProcessKey, ProcessWebSocket>,
}

impl WebSocketLlmAdapter {
    pub(super) fn observe(&mut self, segment: &PayloadSegment) -> Vec<PayloadSegment> {
        if segment.source_boundary != PayloadSourceBoundary::TlsUserSpace {
            return Vec::new();
        }
        let key = ProcessKey {
            trace_id: segment.trace_id,
            process: segment.process.clone(),
        };
        if segment.truncation == PayloadTruncationState::Truncated {
            self.processes.remove(&key);
            return Vec::new();
        }
        self.processes.entry(key).or_default().observe(segment)
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.processes.retain(|key, _| key.trace_id != trace_id);
    }
}

#[derive(Default)]
struct ProcessWebSocket {
    outbound_handshake: HandshakeCandidate,
    inbound_handshake: HandshakeCandidate,
    pending_offer: Option<String>,
    accepted: Option<AcceptedHandshake>,
    connection: Option<WebSocketConnection>,
}

impl ProcessWebSocket {
    fn observe(&mut self, segment: &PayloadSegment) -> Vec<PayloadSegment> {
        if self.observe_handshake(segment) {
            return Vec::new();
        }
        if self.connection.is_none() {
            let expected_masked = segment.direction == PayloadDirection::Outbound;
            if self.accepted.is_some()
                && FrameDecoder::looks_like_frame(&segment.bytes, expected_masked)
            {
                let Some(accepted) = self.accepted.take() else {
                    return Vec::new();
                };
                self.connection = Some(WebSocketConnection::new(
                    segment.stream_key.clone(),
                    accepted.path,
                    accepted.extensions,
                ));
            }
        }
        let Some(connection) = self.connection.as_mut() else {
            return Vec::new();
        };
        match connection.observe(segment) {
            Ok(Some(observation)) => {
                if observation.closed {
                    self.connection = None;
                }
                observation.projected
            }
            Ok(None) => Vec::new(),
            Err(()) => {
                self.connection = None;
                Vec::new()
            }
        }
    }

    fn observe_handshake(&mut self, segment: &PayloadSegment) -> bool {
        match segment.direction {
            PayloadDirection::Outbound => {
                let observed = self.outbound_handshake.observe(segment, REQUEST_PREFIX);
                if let Some(path) = self.outbound_handshake.request_path() {
                    self.pending_offer = Some(path);
                    self.accepted = None;
                }
                observed
            }
            PayloadDirection::Inbound => {
                let observed = self.inbound_handshake.observe(segment, ACCEPT_PREFIX);
                if let Some(extensions) = self.inbound_handshake.accepted_extensions()
                    && let Some(path) = self.pending_offer.take()
                {
                    self.accepted = Some(AcceptedHandshake { path, extensions });
                    self.connection = None;
                }
                observed
            }
        }
    }
}

struct AcceptedHandshake {
    path: String,
    extensions: NegotiatedExtensions,
}
