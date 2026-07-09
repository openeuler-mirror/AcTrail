//! Socket plaintext HTTP admission.

use std::collections::BTreeMap;

use model_core::ids::TraceId;
use model_core::payload::PayloadSourceBoundary;
use payload_event::RawPayloadSegment;

const HTTP2_CONNECTION_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
const SOCKET_SYSCALL_LIBRARY: &str = "socket-syscall";

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
        mut segment: RawPayloadSegment,
    ) -> Result<Vec<RawPayloadSegment>, String> {
        if segment.source_boundary != PayloadSourceBoundary::Syscall
            || segment.library != SOCKET_SYSCALL_LIBRARY
        {
            return Ok(vec![segment]);
        }

        let key = SocketStreamKey::from_segment(&segment);
        match self.streams.get_mut(&key) {
            Some(SocketStreamState::Accepted { protocol_hint }) => {
                segment.protocol_hint = Some(protocol_hint.clone());
                return Ok(vec![segment]);
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
            Some(SocketSniffDecision::Accept(protocol_hint)) => {
                self.streams
                    .insert(key, SocketStreamState::Accepted { protocol_hint });
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
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SocketStreamKey {
    trace_id: u64,
    pid: u32,
    stream_key: String,
}

impl SocketStreamKey {
    fn from_segment(segment: &RawPayloadSegment) -> Self {
        Self {
            trace_id: segment.trace_id.get(),
            pid: segment.process.pid,
            stream_key: segment.stream_key.to_string(),
        }
    }
}

enum SocketStreamState {
    Sniffing(SniffingSocketStream),
    Accepted { protocol_hint: String },
    Rejected,
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
                SocketSniffDecision::Accept(protocol_hint) => {
                    vec![with_protocol_hint(segment, protocol_hint)]
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
            SocketSniffOutcome::Accept(protocol_hint) => {
                self.decision = Some(SocketSniffDecision::Accept(protocol_hint.clone()));
                let pending = std::mem::take(&mut self.pending)
                    .into_iter()
                    .map(|segment| with_protocol_hint(segment, &protocol_hint))
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
    Accept(String),
    Reject,
}

fn with_protocol_hint(mut segment: RawPayloadSegment, protocol_hint: &str) -> RawPayloadSegment {
    segment.protocol_hint = Some(protocol_hint.to_string());
    segment
}

enum SocketSniffOutcome {
    Accept(String),
    Reject,
    NeedMore,
}

fn sniff_http_protocol(bytes: &[u8]) -> SocketSniffOutcome {
    if bytes.starts_with(HTTP2_CONNECTION_PREFACE) {
        return SocketSniffOutcome::Accept("http/2".to_string());
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
        return SocketSniffOutcome::Accept("http/1.x".to_string());
    }
    let parts = first_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() == 3 && parts[2].starts_with("HTTP/") {
        return SocketSniffOutcome::Accept("http/1.x".to_string());
    }
    SocketSniffOutcome::Reject
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
