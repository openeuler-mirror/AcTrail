//! One directional frame decoder establishes identity before body completion.
use super::deflate::DeflateDecoder;
use super::model::{WebSocketAbortReason, WebSocketMessageEvent, WebSocketMessageHead};
use crate::llm_pipeline::transport::evidence::EvidenceTracker;
use config_core::daemon::SemanticRetentionConfig;
use model_core::payload::{PayloadSegment, PayloadStreamIdentity};
use semantic_action::{SemanticEvidence, SemanticEvidenceKind};
use std::collections::BTreeSet;
use std::time::SystemTime;

#[derive(Clone)]
struct FrameHead {
    fin: bool,
    compressed: bool,
    opcode: u8,
    header_len: usize,
    payload_len: usize,
    mask: Option<[u8; 4]>,
}
impl FrameHead {
    fn parse(bytes: &[u8], masked_expected: bool) -> Result<Option<Self>, ()> {
        if bytes.len() < 2 {
            return Ok(None);
        }
        let opcode = bytes[0] & 15;
        let fin = bytes[0] & 128 != 0;
        let compressed = bytes[0] & 64 != 0;
        let masked = bytes[1] & 128 != 0;
        if !matches!(opcode, 0 | 1 | 2 | 8 | 9 | 10)
            || bytes[0] & 48 != 0
            || masked != masked_expected
            || (opcode >= 8 && (!fin || compressed))
        {
            return Err(());
        }
        let mut header_len = 2;
        let mut payload_len = usize::from(bytes[1] & 127);
        if payload_len == 126 {
            if bytes.len() < 4 {
                return Ok(None);
            }
            payload_len = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
            header_len = 4;
            if payload_len < 126 {
                return Err(());
            }
        } else if payload_len == 127 {
            if bytes.len() < 10 {
                return Ok(None);
            }
            let encoded = u64::from_be_bytes(bytes[2..10].try_into().map_err(|_| ())?);
            if encoded & (1 << 63) != 0 || encoded < 65536 {
                return Err(());
            }
            payload_len = usize::try_from(encoded).map_err(|_| ())?;
            header_len = 10;
        }
        if opcode >= 8 && payload_len > 125 {
            return Err(());
        }
        let mask = if masked {
            if bytes.len() < header_len + 4 {
                return Ok(None);
            }
            let value = bytes[header_len..header_len + 4]
                .try_into()
                .map_err(|_| ())?;
            header_len += 4;
            Some(value)
        } else {
            None
        };
        Ok(Some(Self {
            fin,
            compressed,
            opcode,
            header_len,
            payload_len,
            mask,
        }))
    }
}
struct Message {
    head: WebSocketMessageHead,
    compressed: bool,
    bytes: Vec<u8>,
    evidence: Vec<SemanticEvidence>,
    seen: BTreeSet<u64>,
    last_at: SystemTime,
}
pub(super) struct DirectionAssembler {
    buffer: Vec<u8>,
    cursor: usize,
    base: usize,
    evidence: EvidenceTracker,
    frame: Option<FrameHead>,
    message: Option<Message>,
    next_message: u64,
    prefix: String,
    connection_id: String,
    path: String,
    expected_masked: bool,
    deflate: DeflateDecoder,
    max_frame: usize,
    max_message: usize,
    max_ranges: usize,
    compact: usize,
}
#[derive(Default)]
pub(super) struct DirectionObservation {
    pub(super) events: Vec<WebSocketMessageEvent>,
    pub(super) closed: bool,
    pub(super) failed: bool,
}
impl DirectionAssembler {
    pub(super) fn new(
        connection_id: String,
        path: String,
        masked: bool,
        deflate: bool,
        no_context: bool,
        config: &SemanticRetentionConfig,
    ) -> Self {
        let limits = &config.l2_websocket;
        let prefix = format!(
            "{}:{}",
            connection_id,
            if masked { "outbound" } else { "inbound" }
        );
        Self {
            buffer: Vec::new(),
            cursor: 0,
            base: 0,
            evidence: EvidenceTracker::default(),
            frame: None,
            message: None,
            next_message: 0,
            prefix,
            expected_masked: masked,
            connection_id,
            path,
            deflate: DeflateDecoder::new(
                deflate,
                no_context,
                limits.max_decoded_message_bytes as usize,
            ),
            max_frame: limits.max_frame_buffer_bytes as usize,
            max_message: limits.max_message_bytes as usize,
            max_ranges: limits.max_evidence_ranges as usize,
            compact: limits.compact_threshold_bytes as usize,
        }
    }
    pub(super) fn push(&mut self, segment: &PayloadSegment) -> DirectionObservation {
        let mut out = DirectionObservation::default();
        if self
            .buffer
            .len()
            .saturating_sub(self.cursor)
            .saturating_add(segment.bytes.len())
            > self.max_frame
            || self.evidence.len() >= self.max_ranges
        {
            out.events
                .extend(self.abort(WebSocketAbortReason::CapacityExceeded, segment.observed_at));
            out.failed = true;
            return out;
        }
        let start = self.base + self.buffer.len();
        self.evidence
            .append(start, start + segment.bytes.len(), segment);
        self.buffer.extend_from_slice(&segment.bytes);
        loop {
            match self.advance(segment, &mut out.events) {
                Ok(Some(closed)) => {
                    if closed {
                        out.closed = true;
                        out.events.extend(
                            self.abort(WebSocketAbortReason::PeerClosed, segment.observed_at),
                        );
                        break;
                    }
                }
                Ok(None) => break,
                Err(reason) => {
                    out.events.extend(self.abort(reason, segment.observed_at));
                    out.failed = true;
                    break;
                }
            }
        }
        self.compact();
        out
    }
    fn advance(
        &mut self,
        segment: &PayloadSegment,
        out: &mut Vec<WebSocketMessageEvent>,
    ) -> Result<Option<bool>, WebSocketAbortReason> {
        if self.frame.is_none() {
            let Some(frame) = FrameHead::parse(&self.buffer[self.cursor..], self.expected_masked)
                .map_err(|_| WebSocketAbortReason::DecodeFailure)?
            else {
                return Ok(None);
            };
            if frame.opcode == 1 || frame.opcode == 2 {
                if self.message.is_some() {
                    return Err(WebSocketAbortReason::DecodeFailure);
                }
                self.next_message = self
                    .next_message
                    .checked_add(1)
                    .ok_or(WebSocketAbortReason::CapacityExceeded)?;
                let first = self
                    .evidence
                    .for_range(
                        self.base + self.cursor,
                        self.base + self.cursor + frame.header_len,
                    )
                    .first()
                    .copied()
                    .unwrap_or(segment);
                let evidence = vec![SemanticEvidence {
                    kind: SemanticEvidenceKind::PayloadSegment,
                    id: first.segment_id.get(),
                    role: "websocket.message.payload".into(),
                }];
                let head = WebSocketMessageHead {
                    message_id: format!("{}:{}", self.prefix, self.next_message),
                    connection_id: self.connection_id.clone(),
                    path: self.path.clone(),
                    identity: PayloadStreamIdentity::from_segment(first),
                    direction: first.direction,
                    opcode: frame.opcode,
                    start_at: first.observed_at,
                    evidence: evidence.clone(),
                };
                out.push(WebSocketMessageEvent::Start(head.clone()));
                self.message = Some(Message {
                    head,
                    compressed: frame.compressed,
                    bytes: Vec::new(),
                    seen: BTreeSet::new(),
                    evidence: Vec::new(),
                    last_at: first.observed_at,
                });
            } else if frame.opcode == 0 && (self.message.is_none() || frame.compressed) {
                return Err(WebSocketAbortReason::DecodeFailure);
            }
            if frame.header_len.saturating_add(frame.payload_len) > self.max_frame {
                return Err(WebSocketAbortReason::CapacityExceeded);
            }
            self.frame = Some(frame);
        }
        let frame = self
            .frame
            .as_ref()
            .cloned()
            .ok_or(WebSocketAbortReason::DecodeFailure)?;
        let size = frame.header_len + frame.payload_len;
        if self.buffer.len() - self.cursor < size {
            return Ok(None);
        }
        if frame.opcode < 8 {
            self.capture_evidence(self.base + self.cursor, self.base + self.cursor + size)?;
            let message = self
                .message
                .as_mut()
                .ok_or(WebSocketAbortReason::DecodeFailure)?;
            if message.bytes.len().saturating_add(frame.payload_len) > self.max_message {
                return Err(WebSocketAbortReason::CapacityExceeded);
            }
            let payload = &self.buffer[self.cursor + frame.header_len..self.cursor + size];
            if let Some(mask) = frame.mask {
                message.bytes.extend(
                    payload
                        .iter()
                        .enumerate()
                        .map(|(i, byte)| byte ^ mask[i % 4]),
                );
            } else {
                message.bytes.extend_from_slice(payload);
            }
        }
        self.cursor += size;
        self.frame = None;
        self.evidence.evict_before(self.base + self.cursor);
        if frame.opcode < 8 && frame.fin {
            let message = self
                .message
                .take()
                .ok_or(WebSocketAbortReason::DecodeFailure)?;
            let bytes = if message.compressed {
                match self.deflate.decode(message.bytes) {
                    Ok(bytes) => bytes,
                    Err(()) => {
                        out.push(WebSocketMessageEvent::Abort {
                            head: message.head,
                            evidence: message.evidence,
                            observed_at: message.last_at,
                            reason: WebSocketAbortReason::DecodeFailure,
                        });
                        return Err(WebSocketAbortReason::DecodeFailure);
                    }
                }
            } else {
                message.bytes
            };
            if message.head.opcode == 1 && std::str::from_utf8(&bytes).is_err() {
                out.push(WebSocketMessageEvent::Abort {
                    head: message.head,
                    evidence: message.evidence,
                    observed_at: message.last_at,
                    reason: WebSocketAbortReason::DecodeFailure,
                });
                return Err(WebSocketAbortReason::DecodeFailure);
            }
            out.push(WebSocketMessageEvent::Complete {
                head: message.head,
                bytes,
                evidence: message.evidence,
                observed_at: message.last_at,
            });
        }
        Ok(Some(frame.opcode == 8))
    }
    fn capture_evidence(&mut self, start: usize, end: usize) -> Result<(), WebSocketAbortReason> {
        let Some(message) = &mut self.message else {
            return Ok(());
        };
        for source in self.evidence.for_range(start, end) {
            message.last_at = source.observed_at;
            if message.seen.contains(&source.segment_id.get()) {
                continue;
            }
            if message.seen.len() >= self.max_ranges {
                return Err(WebSocketAbortReason::CapacityExceeded);
            }
            message.seen.insert(source.segment_id.get());
            message.evidence.push(SemanticEvidence {
                kind: SemanticEvidenceKind::PayloadSegment,
                id: source.segment_id.get(),
                role: "websocket.message.payload".into(),
            });
        }
        Ok(())
    }
    pub(super) fn abort(
        &mut self,
        reason: WebSocketAbortReason,
        _at: SystemTime,
    ) -> Option<WebSocketMessageEvent> {
        let _ = self.capture_evidence(self.base + self.cursor, self.base + self.buffer.len());
        let event = self
            .message
            .take()
            .map(|message| WebSocketMessageEvent::Abort {
                head: message.head,
                evidence: message.evidence,
                observed_at: message.last_at,
                reason,
            });
        self.buffer.clear();
        self.cursor = 0;
        self.base = 0;
        self.frame = None;
        self.evidence.reset();
        event
    }
    fn compact(&mut self) {
        if self.cursor == self.buffer.len() {
            self.base += self.cursor;
            self.cursor = 0;
            self.buffer.clear();
        } else if self.cursor >= self.compact && self.cursor >= self.buffer.len() / 2 {
            self.buffer.copy_within(self.cursor.., 0);
            self.buffer.truncate(self.buffer.len() - self.cursor);
            self.base += self.cursor;
            self.cursor = 0;
        }
    }
}
