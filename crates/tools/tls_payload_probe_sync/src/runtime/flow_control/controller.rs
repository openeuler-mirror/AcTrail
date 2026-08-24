use std::collections::BTreeMap;

use memchr::memchr_iter;
use tls_payload_core::PayloadDirection;

use super::eviction::{EvictionPolicy, LruPolicy};
use super::text::body_looks_binary;
use super::types::{
    FlowControlConfig, FlowDecision, FlowDirection, FlowEmission, FlowKey, FlowSummary,
};
use super::{http1, http2};

#[derive(Debug, Default)]
pub(in crate::runtime) struct FlowController<P: EvictionPolicy = LruPolicy> {
    streams: BTreeMap<FlowKey, FlowState>,
    policy: P,
}

impl FlowController {
    pub(in crate::runtime) fn observe(
        &mut self,
        config: FlowControlConfig,
        direction: PayloadDirection,
        stream_key: usize,
        payload: &[u8],
    ) -> FlowDecision {
        if !config.enabled || payload.is_empty() {
            return FlowDecision::EmitPayload;
        }
        // TLS-sync does not track SSL object generations yet; keep 0 until
        // SSL_new/SSL_free lifecycle information is available.
        let generation = 0;
        let flow_direction = FlowDirection::from(direction);
        if let Some(decision) = self.observe_http2_frames(
            config,
            direction,
            stream_key,
            flow_direction,
            generation,
            payload,
        ) {
            return decision;
        }
        let key = FlowKey::connection(stream_key, flow_direction, generation);
        self.observe_with_key(config, direction, key, payload)
    }

    fn observe_http2_frames(
        &mut self,
        config: FlowControlConfig,
        direction: PayloadDirection,
        stream_key: usize,
        flow_direction: FlowDirection,
        generation: u32,
        payload: &[u8],
    ) -> Option<FlowDecision> {
        let mut cursor = 0_usize;
        let mut saw_frame = false;
        let mut actions = Vec::new();
        if payload.starts_with(http2::CONNECTION_PREFACE) {
            actions.push(FrameAction::PassThrough {
                start: 0,
                end: http2::CONNECTION_PREFACE.len(),
            });
            cursor = http2::CONNECTION_PREFACE.len();
            saw_frame = true;
        }
        while cursor < payload.len() {
            let Some(frame) = http2::decode_frame(&payload[cursor..]) else {
                break;
            };
            saw_frame = true;
            let start = cursor;
            let end = cursor + frame.encoded_len;
            if frame.frame_type == http2::DATA_FRAME_TYPE {
                let key =
                    FlowKey::http2_stream(stream_key, flow_direction, generation, frame.stream_id);
                let decision = self.observe_with_key(config, direction, key, frame.payload);
                match decision {
                    FlowDecision::EmitPayload => {
                        actions.push(FrameAction::PassThrough { start, end })
                    }
                    FlowDecision::EmitSummary(summary) => {
                        actions.push(FrameAction::Summary(summary));
                    }
                    FlowDecision::DropBody => actions.push(FrameAction::Drop),
                    FlowDecision::EmitMany(frame_emissions) => {
                        let has_payload = frame_emissions
                            .iter()
                            .any(|emission| matches!(emission, FlowEmission::Payload(_)));
                        if has_payload {
                            actions.push(FrameAction::PassThrough { start, end });
                        }
                        for emission in frame_emissions {
                            if let FlowEmission::Summary(summary) = emission {
                                actions.push(FrameAction::Summary(summary));
                            }
                        }
                    }
                }
            } else {
                actions.push(FrameAction::PassThrough { start, end });
            }
            cursor = end;
        }
        if !saw_frame {
            return None;
        }
        if cursor < payload.len() {
            actions.push(FrameAction::PassThrough {
                start: cursor,
                end: payload.len(),
            });
        }
        if actions
            .iter()
            .all(|action| matches!(action, FrameAction::PassThrough { .. }))
        {
            return Some(FlowDecision::EmitPayload);
        }
        let mut emissions = Vec::new();
        for action in actions {
            match action {
                FrameAction::PassThrough { start, end } => {
                    emissions.push(FlowEmission::Payload(payload[start..end].to_vec()))
                }
                FrameAction::Summary(summary) => emissions.push(FlowEmission::Summary(summary)),
                FrameAction::Drop => {}
            }
        }
        Some(emissions_to_decision(emissions))
    }

    fn observe_with_key(
        &mut self,
        config: FlowControlConfig,
        direction: PayloadDirection,
        key: FlowKey,
        payload: &[u8],
    ) -> FlowDecision {
        self.policy.touch(key);
        if !self.streams.contains_key(&key) {
            while config.max_streams > 0 && self.streams.len() >= config.max_streams {
                let Some(oldest) = self.policy.evict_candidate() else {
                    break;
                };
                self.streams.remove(&oldest);
            }
        }
        let state = self.streams.entry(key).or_default();
        let mut cursor = 0_usize;
        let mut altered = false;
        let mut emissions = Vec::new();
        while cursor < payload.len() {
            let current = std::mem::take(state);
            let step = current.observe(config, direction, &payload[cursor..]);
            let consumed = step.consumed;
            let pass_without_progress =
                consumed == 0 && matches!(&step.emission, StepEmission::Pass);
            match step.emission {
                StepEmission::Summary(_) | StepEmission::Drop => {
                    ensure_altered(&mut altered, &mut emissions, payload, cursor);
                }
                StepEmission::DiscardPrefix => {
                    altered = true;
                }
                StepEmission::Pass => {}
            }
            match step.emission {
                StepEmission::Pass => {
                    if altered {
                        push_payload(&mut emissions, &payload[cursor..cursor + consumed]);
                    }
                }
                StepEmission::Summary(summary) => emissions.push(FlowEmission::Summary(summary)),
                StepEmission::Drop => {}
                StepEmission::DiscardPrefix => {}
            }
            *state = step.next;
            cursor += consumed;
            if pass_without_progress {
                continue;
            }
            if consumed == 0 {
                break;
            }
        }
        if !altered {
            FlowDecision::EmitPayload
        } else {
            emissions_to_decision(emissions)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FlowState {
    Active {
        observed: u64,
        prefix: Vec<u8>,
        /// HTTP/1 message total size once the header has been parsed. While
        /// `Some` and the message is still in flight, the body chunks skip the
        /// header re-scan (the header is parsed only once per message).
        message_size: Option<u64>,
    },
    SummaryOnly {
        observed: u64,
        scope: SummaryScope,
        drop_reported: bool,
    },
}

impl Default for FlowState {
    fn default() -> Self {
        Self::Active {
            observed: 0,
            prefix: Vec::new(),
            message_size: None,
        }
    }
}

impl FlowState {
    fn observe(
        self,
        config: FlowControlConfig,
        direction: PayloadDirection,
        payload: &[u8],
    ) -> FlowStep {
        match self {
            Self::Active {
                observed,
                prefix,
                message_size,
            } => observe_active(config, direction, observed, prefix, message_size, payload),
            Self::SummaryOnly {
                observed,
                scope,
                drop_reported,
            } => observe_summary(observed, scope, drop_reported, payload),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SummaryScope {
    KnownRemaining { bytes: u64 },
    Unbounded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FlowStep {
    consumed: usize,
    emission: StepEmission,
    next: FlowState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FrameAction {
    PassThrough { start: usize, end: usize },
    Summary(FlowSummary),
    Drop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum StepEmission {
    Pass,
    Summary(FlowSummary),
    Drop,
    DiscardPrefix,
}

fn observe_summary(
    observed: u64,
    scope: SummaryScope,
    drop_reported: bool,
    payload: &[u8],
) -> FlowStep {
    let emission = if drop_reported {
        StepEmission::Drop
    } else {
        StepEmission::Summary(FlowSummary {
            observed_size: observed,
            reason: "flow_drop_discontinuity",
            protocol_hint: "unknown",
            bytes: Vec::new(),
        })
    };
    match scope {
        SummaryScope::KnownRemaining { bytes } => {
            let consumed = consume_up_to(payload.len(), bytes);
            let remaining = bytes.saturating_sub(consumed as u64);
            let next = if remaining == 0 {
                FlowState::default()
            } else {
                FlowState::SummaryOnly {
                    observed: observed.saturating_add(consumed as u64),
                    scope: SummaryScope::KnownRemaining { bytes: remaining },
                    drop_reported: true,
                }
            };
            FlowStep {
                consumed,
                emission,
                next,
            }
        }
        SummaryScope::Unbounded => {
            // HTTP/1 messages are sequential on a connection. If an unbounded
            // summary/drop region is followed by a new HTTP/1 message header,
            // treat the previous message as finished and start the next
            // message cleanly instead of poisoning every later exchange.
            if let Some(start) = http1_message_start(payload) {
                if start == 0 {
                    return FlowStep {
                        consumed: 0,
                        emission: StepEmission::Pass,
                        next: FlowState::default(),
                    };
                }
                return FlowStep {
                    consumed: start,
                    emission: StepEmission::DiscardPrefix,
                    next: FlowState::default(),
                };
            }
            FlowStep {
                consumed: payload.len(),
                emission,
                next: FlowState::SummaryOnly {
                    observed: observed.saturating_add(payload.len() as u64),
                    scope: SummaryScope::Unbounded,
                    drop_reported: true,
                },
            }
        }
    }
}

fn observe_active(
    config: FlowControlConfig,
    direction: PayloadDirection,
    observed: u64,
    mut prefix: Vec<u8>,
    message_size: Option<u64>,
    payload: &[u8],
) -> FlowStep {
    // Fast path: the header was already parsed for this message. Body chunks
    // pass straight through without rescanning the accumulated prefix (which
    // is O(prefix) per chunk and dominated the HTTP/1 hot path).
    if let Some(size) = message_size {
        let end = observed.saturating_add(payload.len() as u64);
        if end < size {
            return FlowStep {
                consumed: payload.len(),
                emission: StepEmission::Pass,
                next: FlowState::Active {
                    observed: end,
                    prefix,
                    message_size: Some(size),
                },
            };
        }
        // The message ends within this chunk; consume up to the boundary and
        // reset so the next message is parsed afresh.
        let consumed = consume_up_to(payload.len(), size.saturating_sub(observed));
        return FlowStep {
            consumed,
            emission: StepEmission::Pass,
            next: FlowState::default(),
        };
    }
    append_prefix(&mut prefix, payload, config.sniff_bytes);
    let observed_if_all = observed.saturating_add(payload.len() as u64);
    if let Some(inspection) = http1::inspect(config, direction, observed_if_all, &prefix) {
        if let Some(summary) = inspection.summary {
            let consumed = scoped_consumed(payload.len(), observed, inspection.message_size);
            return FlowStep {
                consumed,
                emission: StepEmission::Summary(summary),
                next: summary_next_state(observed, consumed, inspection.message_size),
            };
        }
        if let Some(message_size) = inspection.message_size {
            let consumed = scoped_consumed(payload.len(), observed, Some(message_size));
            let observed = observed.saturating_add(consumed as u64);
            let next = if observed >= message_size {
                FlowState::default()
            } else {
                FlowState::Active {
                    observed,
                    prefix,
                    message_size: Some(message_size),
                }
            };
            return FlowStep {
                consumed,
                emission: StepEmission::Pass,
                next,
            };
        }
    }
    if let Some(summary) = http2::classify(config, direction, observed_if_all, &prefix) {
        return FlowStep {
            consumed: payload.len(),
            emission: StepEmission::Summary(summary),
            next: FlowState::SummaryOnly {
                observed: observed_if_all,
                scope: SummaryScope::Unbounded,
                drop_reported: true,
            },
        };
    }
    if let Some(summary) = classify_binary_prefix(config, observed_if_all, &prefix)
        .or_else(|| classify_unknown_threshold(config, observed_if_all, &prefix))
    {
        return FlowStep {
            consumed: payload.len(),
            emission: StepEmission::Summary(summary),
            next: FlowState::SummaryOnly {
                observed: observed_if_all,
                scope: SummaryScope::Unbounded,
                drop_reported: true,
            },
        };
    }
    FlowStep {
        consumed: payload.len(),
        emission: StepEmission::Pass,
        next: FlowState::Active {
            observed: observed_if_all,
            prefix,
            message_size: None,
        },
    }
}

fn summary_next_state(
    observed_before: u64,
    consumed: usize,
    message_size: Option<u64>,
) -> FlowState {
    let observed = observed_before.saturating_add(consumed as u64);
    match message_size {
        Some(size) if observed >= size => FlowState::default(),
        Some(size) => FlowState::SummaryOnly {
            observed,
            scope: SummaryScope::KnownRemaining {
                bytes: size.saturating_sub(observed),
            },
            // The originating summary already marks the drop region; do not
            // emit a redundant flow_drop_discontinuity row on the next chunk.
            drop_reported: true,
        },
        None => FlowState::SummaryOnly {
            observed,
            scope: SummaryScope::Unbounded,
            drop_reported: true,
        },
    }
}

fn scoped_consumed(payload_len: usize, observed: u64, message_size: Option<u64>) -> usize {
    let Some(message_size) = message_size else {
        return payload_len;
    };
    if observed >= message_size {
        return 0;
    }
    consume_up_to(payload_len, message_size - observed)
}

fn consume_up_to(payload_len: usize, remaining: u64) -> usize {
    payload_len.min(remaining.min(usize::MAX as u64) as usize)
}

fn append_prefix(prefix: &mut Vec<u8>, payload: &[u8], limit: usize) {
    if prefix.len() >= limit {
        return;
    }
    let remaining = limit - prefix.len();
    prefix.extend_from_slice(&payload[..payload.len().min(remaining)]);
}

fn classify_binary_prefix(
    config: FlowControlConfig,
    observed: u64,
    payload: &[u8],
) -> Option<FlowSummary> {
    if observed < config.unknown_stream_bytes || !body_looks_binary(payload) {
        return None;
    }
    Some(FlowSummary {
        observed_size: observed,
        reason: "binary_unknown_stream",
        protocol_hint: "unknown",
        bytes: Vec::new(),
    })
}

fn classify_unknown_threshold(
    config: FlowControlConfig,
    observed: u64,
    prefix: &[u8],
) -> Option<FlowSummary> {
    if observed <= config.unknown_stream_bytes || !unknown_prefix(prefix) {
        return None;
    }
    Some(FlowSummary {
        observed_size: observed,
        reason: "unknown_stream_threshold",
        protocol_hint: "unknown",
        bytes: Vec::new(),
    })
}

fn ensure_altered(
    altered: &mut bool,
    emissions: &mut Vec<FlowEmission>,
    payload: &[u8],
    cursor: usize,
) {
    if *altered {
        return;
    }
    if cursor > 0 {
        push_payload(emissions, &payload[..cursor]);
    }
    *altered = true;
}

fn push_payload(emissions: &mut Vec<FlowEmission>, payload: &[u8]) {
    if payload.is_empty() {
        return;
    }
    emissions.push(FlowEmission::Payload(payload.to_vec()));
}

fn emissions_to_decision(emissions: Vec<FlowEmission>) -> FlowDecision {
    match emissions.as_slice() {
        [] => FlowDecision::DropBody,
        [FlowEmission::Summary(summary)] => FlowDecision::EmitSummary(summary.clone()),
        _ => FlowDecision::EmitMany(emissions),
    }
}

fn unknown_prefix(prefix: &[u8]) -> bool {
    !http1::looks_like_header(prefix) && !http2::starts_with_preface(prefix)
}

fn http1_message_start(payload: &[u8]) -> Option<usize> {
    if http1::looks_like_header(payload) {
        return Some(0);
    }
    for newline in memchr_iter(b'\n', payload) {
        let pos = newline + 1;
        if pos < payload.len() && http1::looks_like_header(&payload[pos..]) {
            return Some(pos);
        }
    }
    None
}
