//! Bounded protocol selection before any HTTP body decoder is entered.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::payload::{PayloadDirection, PayloadSegment, PayloadStreamIdentity};

use crate::llm_pipeline::assembly::router::AssemblyLimits;
use crate::llm_pipeline::transport::http2::HTTP2_CONNECTION_PREFACE;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ProtocolRoute {
    Pending,
    Discard,
    DiscardHttp2,
    Http1,
    Http2,
    Legacy,
    WebSocket,
}

#[derive(Clone, Eq, PartialEq)]
struct ProtocolDirectionKey {
    identity: PayloadStreamIdentity,
    direction: PayloadDirection,
}

impl Ord for ProtocolDirectionKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Enum ordinals are local ordering only; they are never persisted.
        (
            self.identity.trace_id,
            self.identity.process,
            self.identity.source_boundary as u8,
            &self.identity.stream_key,
            self.direction as u8,
        )
            .cmp(&(
                other.identity.trace_id,
                other.identity.process,
                other.identity.source_boundary as u8,
                &other.identity.stream_key,
                other.direction as u8,
            ))
    }
}

impl PartialOrd for ProtocolDirectionKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

struct RouteState {
    route: ProtocolRoute,
    prefix: Vec<u8>,
    segments: Vec<PayloadSegment>,
}

pub(super) struct ProtocolRouter {
    streams: BTreeMap<ProtocolDirectionKey, RouteState>,
    limits: AssemblyLimits,
    max_directions: usize,
}

impl ProtocolRouter {
    pub(super) fn new(limits: AssemblyLimits, max_directions: u32) -> Self {
        Self {
            streams: BTreeMap::new(),
            limits,
            max_directions: max_directions as usize,
        }
    }

    pub(super) fn route(
        &mut self,
        segment: &PayloadSegment,
    ) -> (ProtocolRoute, Vec<PayloadSegment>) {
        let key = ProtocolDirectionKey {
            identity: PayloadStreamIdentity::from_segment(segment),
            direction: segment.direction,
        };
        let opposite = ProtocolDirectionKey {
            identity: key.identity.clone(),
            direction: match segment.direction {
                PayloadDirection::Inbound => PayloadDirection::Outbound,
                PayloadDirection::Outbound => PayloadDirection::Inbound,
            },
        };
        let poisoned_h2 = self
            .streams
            .get(&opposite)
            .is_some_and(|state| state.route == ProtocolRoute::DiscardHttp2);
        let known_h2 = segment.protocol_hint.as_deref() == Some("h2")
            || self
                .streams
                .get(&opposite)
                .is_some_and(|state| state.route == ProtocolRoute::Http2);
        if let Some(state) = self.streams.get(&key) {
            if state.route != ProtocolRoute::Pending {
                return (state.route, Vec::new());
            }
        } else if self.streams.len() >= self.max_directions {
            tracing::warn!(trace_id = %segment.trace_id, process_id = %segment.process.get(), stream_key = %segment.stream_key, max_directions = self.max_directions, "protocol router stream capacity reached");
            return (ProtocolRoute::Discard, Vec::new());
        }
        let state = self.streams.entry(key).or_insert_with(|| RouteState {
            route: ProtocolRoute::Pending,
            prefix: Vec::new(),
            segments: Vec::new(),
        });
        let route = if poisoned_h2 {
            ProtocolRoute::DiscardHttp2
        } else if known_h2 {
            ProtocolRoute::Http2
        } else if state.prefix.is_empty() {
            Self::classify(&segment.bytes)
        } else {
            let remaining = self
                .limits
                .max_buffer_bytes
                .saturating_sub(state.prefix.len());
            let line_bytes = if state.prefix.first().is_some_and(|byte| *byte == 0)
                || HTTP2_CONNECTION_PREFACE.starts_with(&state.prefix)
            {
                segment.bytes.len().min(HTTP2_CONNECTION_PREFACE.len())
            } else {
                segment
                    .bytes
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(segment.bytes.len(), |index| index + 1)
            };
            state
                .prefix
                .extend_from_slice(&segment.bytes[..line_bytes.min(remaining)]);
            Self::classify(&state.prefix)
        };
        state.route = route;
        if route == ProtocolRoute::Pending {
            if state.prefix.is_empty() {
                state.prefix.extend_from_slice(
                    &segment.bytes[..segment.bytes.len().min(self.limits.max_buffer_bytes)],
                );
            }
            if state.prefix.len() >= self.limits.max_buffer_bytes
                || state.segments.len() >= self.limits.max_segment_ranges
            {
                state.route = ProtocolRoute::Discard;
            } else {
                state.segments.push(segment.clone());
                return (ProtocolRoute::Pending, Vec::new());
            }
        }
        state.prefix.clear();
        (state.route, std::mem::take(&mut state.segments))
    }

    fn classify(bytes: &[u8]) -> ProtocolRoute {
        if bytes.starts_with(HTTP2_CONNECTION_PREFACE) {
            return ProtocolRoute::Http2;
        }
        if HTTP2_CONNECTION_PREFACE.starts_with(bytes) {
            return ProtocolRoute::Pending;
        }
        if bytes.first() == Some(&0) {
            if bytes.len() < 9 {
                return ProtocolRoute::Pending;
            }
            let stream_id = u32::from_be_bytes([bytes[5] & 0x7f, bytes[6], bytes[7], bytes[8]]);
            let valid_stream = match bytes[3] {
                0 | 1 | 2 | 3 | 5 | 9 => stream_id != 0,
                4 | 6 | 7 => stream_id == 0,
                8 => true,
                _ => false,
            };
            if valid_stream {
                return ProtocolRoute::Http2;
            }
        }

        const PREFIXES: &[&[u8]] = &[
            b"HTTP/1.",
            b"GET ",
            b"POST ",
            b"PUT ",
            b"DELETE ",
            b"HEAD ",
            b"OPTIONS ",
            b"CONNECT ",
            b"PATCH ",
            b"TRACE ",
        ];
        if PREFIXES.iter().any(|prefix| bytes.starts_with(prefix)) {
            return ProtocolRoute::Http1;
        }
        if PREFIXES.iter().any(|prefix| prefix.starts_with(bytes)) {
            return ProtocolRoute::Pending;
        }
        // Extension methods use the same HTTP framing. Wait for their start line
        // instead of letting the legacy path create a second HTTP/1 decoder.
        let method_end = bytes.iter().position(|byte| *byte == b' ');
        let method = &bytes[..method_end.unwrap_or(bytes.len())];
        if method.is_empty()
            || !method
                .iter()
                .all(|byte| byte.is_ascii_alphabetic() || *byte == b'-')
        {
            return ProtocolRoute::Legacy;
        }
        let Some(end) = bytes.windows(2).position(|part| part == b"\r\n") else {
            return ProtocolRoute::Pending;
        };
        if bytes[..end].ends_with(b" HTTP/1.0") || bytes[..end].ends_with(b" HTTP/1.1") {
            ProtocolRoute::Http1
        } else {
            ProtocolRoute::Legacy
        }
    }

    pub(super) fn upgrade(&mut self, identity: &PayloadStreamIdentity) {
        for direction in [PayloadDirection::Outbound, PayloadDirection::Inbound] {
            let key = ProtocolDirectionKey {
                identity: identity.clone(),
                direction,
            };
            if let Some(state) = self.streams.get_mut(&key) {
                state.route = ProtocolRoute::WebSocket;
                state.prefix.clear();
                state.segments.clear();
            }
        }
    }

    pub(super) fn invalidate_identity(&mut self, identity: &PayloadStreamIdentity) {
        let keys = [PayloadDirection::Outbound, PayloadDirection::Inbound].map(|direction| {
            ProtocolDirectionKey {
                identity: identity.clone(),
                direction,
            }
        });
        let is_h2 = keys.iter().any(|key| {
            self.streams.get(key).is_some_and(|state| {
                matches!(
                    state.route,
                    ProtocolRoute::Http2 | ProtocolRoute::DiscardHttp2
                )
            })
        });
        if is_h2 {
            for key in &keys {
                if let Some(state) = self.streams.get_mut(key) {
                    state.route = ProtocolRoute::DiscardHttp2;
                    state.prefix.clear();
                    state.segments.clear();
                }
            }
        } else if keys.iter().any(|key| {
            self.streams
                .get(key)
                .is_some_and(|state| state.route == ProtocolRoute::WebSocket)
        }) {
            for key in &keys {
                if let Some(state) = self.streams.get_mut(key) {
                    state.route = ProtocolRoute::WebSocket;
                    state.prefix.clear();
                    state.segments.clear();
                }
            }
        } else {
            self.forget_identity(identity);
        }
    }

    pub(super) fn forget_identity(&mut self, identity: &PayloadStreamIdentity) {
        for direction in [PayloadDirection::Outbound, PayloadDirection::Inbound] {
            self.streams.remove(&ProtocolDirectionKey {
                identity: identity.clone(),
                direction,
            });
        }
    }

    pub(super) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams
            .retain(|key, _| key.identity.trace_id != trace_id);
    }
}
