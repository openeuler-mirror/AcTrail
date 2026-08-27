//! Socket plaintext HTTP admission.

mod connect;

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::payload::{
    PayloadOperationCompletionState, PayloadSourceBoundary, PayloadTruncationState,
};
use model_core::process::ProcessObservation;
use payload_event::{RawPayloadSegment, RawPayloadStreamClose};

use self::connect::{ClientConnectTunnelGate, ConnectTunnelDecision};

const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const SOCKET_SYSCALL_LIBRARY: &str = "socket-syscall";
const TLS_HELLO_PREFIX_SIZE: usize = 9;
const TLS_HANDSHAKE_CONTENT_TYPE: u8 = 22;
const TLS_CLIENT_HELLO: u8 = 1;
const TLS_SERVER_HELLO: u8 = 2;
const TLS_MIN_HELLO_BODY_SIZE: u32 = 38;
const TLS_MAX_PLAINTEXT_RECORD_SIZE: u32 = 16_384;

pub(in crate::services) struct SocketHttpPayloadGate {
    max_sniff_bytes: u64,
    max_streams: u32,
    streams: BTreeMap<SocketStreamKey, SocketStreamState>,
}

impl SocketHttpPayloadGate {
    pub(in crate::services) fn new(max_sniff_bytes: u64, max_streams: u32) -> Self {
        Self {
            max_sniff_bytes,
            max_streams,
            streams: BTreeMap::new(),
        }
    }

    pub(in crate::services) fn admit(
        &mut self,
        segment: RawPayloadSegment,
    ) -> Result<Vec<RawPayloadSegment>, String> {
        if segment.source_boundary != PayloadSourceBoundary::Syscall
            || segment.library != SOCKET_SYSCALL_LIBRARY
        {
            return Ok(vec![segment]);
        }

        let key = SocketStreamKey::from_segment(&segment);
        match self.streams.get_mut(&key) {
            Some(SocketStreamState::Accepted { protocol_hint }) => {
                return Ok(vec![with_protocol_hint(segment, protocol_hint)]);
            }
            Some(stream_state @ SocketStreamState::ClientConnect(_)) => {
                let decision = match stream_state {
                    SocketStreamState::ClientConnect(state) => state.admit(segment)?,
                    SocketStreamState::Sniffing(_)
                    | SocketStreamState::Accepted { .. }
                    | SocketStreamState::Rejected => unreachable!(),
                };
                return match decision {
                    AcceptedSocketDecision::Admit(segment) => Ok(vec![segment]),
                    AcceptedSocketDecision::TunnelEstablished(segment) => {
                        *stream_state = SocketStreamState::Rejected;
                        Ok(segment.into_iter().collect())
                    }
                };
            }
            Some(SocketStreamState::Rejected) => return Ok(Vec::new()),
            Some(SocketStreamState::Sniffing(state)) => return state.admit(segment),
            None => {}
        }

        if self.streams.len() >= self.max_streams as usize {
            return Err(format!(
                "socket HTTP payload stream count would exceed configured maximum {}",
                self.max_streams
            ));
        }
        let mut state = SniffingSocketStream::new(self.max_sniff_bytes);
        let admitted = state.admit(segment)?;
        match state.decision.clone() {
            Some(SocketSniffDecision::Accept(sniffed)) => {
                let stream_state = if sniffed.client_connect {
                    SocketStreamState::ClientConnect(ClientConnectSocketStream::new(
                        sniffed.protocol_hint,
                        self.max_sniff_bytes,
                    )?)
                } else {
                    SocketStreamState::Accepted {
                        protocol_hint: sniffed.protocol_hint,
                    }
                };
                self.streams.insert(key, stream_state);
            }
            Some(SocketSniffDecision::Reject) => {
                self.streams.insert(key, SocketStreamState::Rejected);
            }
            None => {
                self.streams.insert(key, SocketStreamState::Sniffing(state));
            }
        }
        Ok(admitted)
    }

    pub(in crate::services) fn forget_trace(&mut self, trace_id: TraceId) {
        self.streams.retain(|key, _| key.trace_id != trace_id.get());
    }

    pub(in crate::services) fn forget_stream(&mut self, close: &RawPayloadStreamClose) {
        let stream_key = close.stream_key.to_string();
        self.streams.retain(|key, _| {
            key.trace_id != close.trace_id.get()
                || key.process != close.process
                || key.stream_key != stream_key
        });
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SocketStreamKey {
    trace_id: u64,
    process: ProcessObservation,
    stream_key: String,
}

impl SocketStreamKey {
    fn from_segment(segment: &RawPayloadSegment) -> Self {
        Self {
            trace_id: segment.trace_id.get(),
            process: segment.process.clone(),
            stream_key: segment.stream_key.to_string(),
        }
    }
}

enum SocketStreamState {
    Sniffing(SniffingSocketStream),
    Accepted { protocol_hint: String },
    ClientConnect(ClientConnectSocketStream),
    Rejected,
}

struct ClientConnectSocketStream {
    protocol_hint: String,
    connect_tunnel: ClientConnectTunnelGate,
}

impl ClientConnectSocketStream {
    fn new(protocol_hint: String, max_sniff_bytes: u64) -> Result<Self, String> {
        Ok(Self {
            protocol_hint,
            connect_tunnel: ClientConnectTunnelGate::awaiting_response(max_sniff_bytes)?,
        })
    }

    fn admit(&mut self, mut segment: RawPayloadSegment) -> Result<AcceptedSocketDecision, String> {
        segment.protocol_hint = Some(self.protocol_hint.clone());
        match self
            .connect_tunnel
            .observe(segment.direction, &segment.bytes)
        {
            ConnectTunnelDecision::Admit => Ok(AcceptedSocketDecision::Admit(segment)),
            ConnectTunnelDecision::Established {
                admitted_prefix_len,
            } => {
                if admitted_prefix_len == 0 {
                    return Ok(AcceptedSocketDecision::TunnelEstablished(None));
                }
                truncate_to_http_prefix(&mut segment, admitted_prefix_len)?;
                Ok(AcceptedSocketDecision::TunnelEstablished(Some(segment)))
            }
        }
    }
}

enum AcceptedSocketDecision {
    Admit(RawPayloadSegment),
    TunnelEstablished(Option<RawPayloadSegment>),
}

struct SniffingSocketStream {
    max_sniff_bytes: u64,
    observed_bytes: u64,
    buffer: Vec<u8>,
    pending: Vec<RawPayloadSegment>,
    decision: Option<SocketSniffDecision>,
}

impl SniffingSocketStream {
    fn new(max_sniff_bytes: u64) -> Self {
        Self {
            max_sniff_bytes,
            observed_bytes: 0,
            buffer: Vec::new(),
            pending: Vec::new(),
            decision: None,
        }
    }

    fn admit(&mut self, segment: RawPayloadSegment) -> Result<Vec<RawPayloadSegment>, String> {
        if let Some(decision) = &self.decision {
            return Ok(match decision {
                SocketSniffDecision::Accept(sniffed) => {
                    vec![with_protocol_hint(segment, &sniffed.protocol_hint)]
                }
                SocketSniffDecision::Reject => Vec::new(),
            });
        }

        self.observed_bytes = self
            .observed_bytes
            .checked_add(segment.bytes.len() as u64)
            .ok_or_else(|| "socket HTTP sniff byte count overflow".to_string())?;
        self.buffer.extend_from_slice(&segment.bytes);
        self.pending.push(segment);

        match sniff_http_protocol(&self.buffer) {
            SocketSniffOutcome::Accept(sniffed) => {
                self.decision = Some(SocketSniffDecision::Accept(sniffed.clone()));
                let pending = std::mem::take(&mut self.pending)
                    .into_iter()
                    .map(|segment| with_protocol_hint(segment, &sniffed.protocol_hint))
                    .collect();
                self.buffer.clear();
                return Ok(pending);
            }
            SocketSniffOutcome::Reject => {
                self.decision = Some(SocketSniffDecision::Reject);
                self.pending.clear();
                self.buffer.clear();
                return Ok(Vec::new());
            }
            SocketSniffOutcome::NeedMore => {}
        }
        if self.observed_bytes >= self.max_sniff_bytes {
            self.decision = Some(SocketSniffDecision::Reject);
            self.pending.clear();
            self.buffer.clear();
        }
        Ok(Vec::new())
    }
}

#[derive(Clone)]
enum SocketSniffDecision {
    Accept(SniffedHttpProtocol),
    Reject,
}

fn with_protocol_hint(mut segment: RawPayloadSegment, protocol_hint: &str) -> RawPayloadSegment {
    segment.protocol_hint = Some(protocol_hint.to_string());
    segment
}

enum SocketSniffOutcome {
    Accept(SniffedHttpProtocol),
    Reject,
    NeedMore,
}

#[derive(Clone)]
struct SniffedHttpProtocol {
    protocol_hint: String,
    client_connect: bool,
}

fn sniff_http_protocol(bytes: &[u8]) -> SocketSniffOutcome {
    if bytes.starts_with(HTTP2_CONNECTION_PREFACE) {
        return SocketSniffOutcome::Accept(SniffedHttpProtocol {
            protocol_hint: "http/2".to_string(),
            client_connect: false,
        });
    }
    if HTTP2_CONNECTION_PREFACE.starts_with(bytes) {
        return SocketSniffOutcome::NeedMore;
    }
    let Some(line_end) = bytes.iter().position(|byte| *byte == b'\n') else {
        return SocketSniffOutcome::NeedMore;
    };
    let first_line = match std::str::from_utf8(&bytes[..line_end]) {
        Ok(text) => text.trim_end_matches('\r').trim(),
        Err(_) => return SocketSniffOutcome::Reject,
    };
    if first_line.starts_with("HTTP/") {
        return SocketSniffOutcome::Accept(SniffedHttpProtocol {
            protocol_hint: "http/1.x".to_string(),
            client_connect: false,
        });
    }
    let parts = first_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() == 3 && parts[2].starts_with("HTTP/") {
        return SocketSniffOutcome::Accept(SniffedHttpProtocol {
            protocol_hint: "http/1.x".to_string(),
            client_connect: parts[0] == "CONNECT" && matches!(parts[2], "HTTP/1.0" | "HTTP/1.1"),
        });
    }
    SocketSniffOutcome::Reject
}

fn truncate_to_http_prefix(
    segment: &mut RawPayloadSegment,
    admitted_prefix_len: usize,
) -> Result<(), String> {
    if admitted_prefix_len > segment.bytes.len() {
        return Err(format!(
            "CONNECT response prefix {} exceeds captured segment size {}",
            admitted_prefix_len,
            segment.bytes.len()
        ));
    }
    let removed = segment.bytes.len() - admitted_prefix_len;
    segment.bytes.truncate(admitted_prefix_len);
    segment.captured_size = admitted_prefix_len as u64;
    segment.operation_captured_size = segment
        .operation_captured_size
        .checked_sub(removed as u64)
        .ok_or_else(|| "CONNECT response captured size underflow".to_string())?;
    if removed != 0 {
        segment.truncation = PayloadTruncationState::Truncated;
        segment.operation_completion_state = PayloadOperationCompletionState::Partial;
    }
    Ok(())
}

pub(in crate::services) fn socket_payload_prefix_is_http_candidate(
    bytes: &[u8],
    reached_sniff_limit: bool,
) -> bool {
    match sniff_http_protocol(bytes) {
        SocketSniffOutcome::Accept(_) => true,
        SocketSniffOutcome::NeedMore => !reached_sniff_limit,
        SocketSniffOutcome::Reject => false,
    }
}

pub(in crate::services) fn socket_payload_prefix_is_tls_hello(bytes: &[u8]) -> bool {
    if bytes.len() < TLS_HELLO_PREFIX_SIZE
        || bytes[0] != TLS_HANDSHAKE_CONTENT_TYPE
        || bytes[1] != 3
        || !(1..=3).contains(&bytes[2])
        || (bytes[5] != TLS_CLIENT_HELLO && bytes[5] != TLS_SERVER_HELLO)
    {
        return false;
    }
    let record_size = u32::from_be_bytes([0, 0, bytes[3], bytes[4]]);
    let handshake_size = u32::from_be_bytes([0, bytes[6], bytes[7], bytes[8]]);
    record_size >= (TLS_HELLO_PREFIX_SIZE - 5) as u32
        && record_size <= TLS_MAX_PLAINTEXT_RECORD_SIZE
        && handshake_size >= TLS_MIN_HELLO_BODY_SIZE
        && handshake_size
            .checked_add(4)
            .is_some_and(|size| size <= record_size)
}
