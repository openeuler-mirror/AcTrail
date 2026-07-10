use std::collections::BTreeMap;

use tls_payload_core::PayloadDirection;

use super::text::body_looks_binary;
use super::types::{
    FlowControlConfig, FlowDecision, FlowDirection, FlowEmission, FlowKey, FlowSummary,
};
use super::{http1, http2};

#[derive(Debug, Default)]
pub(in crate::runtime) struct FlowController {
    streams: BTreeMap<FlowKey, FlowState>,
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
        let key = FlowKey {
            stream_key,
            direction: FlowDirection::from(direction),
        };
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
            if matches!(
                &step.emission,
                StepEmission::Summary(_) | StepEmission::Drop
            ) {
                ensure_altered(&mut altered, &mut emissions, payload, cursor);
            }
            match step.emission {
                StepEmission::Pass => {
                    if altered {
                        push_payload(&mut emissions, &payload[cursor..cursor + consumed]);
                    }
                }
                StepEmission::Summary(summary) => emissions.push(FlowEmission::Summary(summary)),
                StepEmission::Drop => {}
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
    Active { observed: u64, prefix: Vec<u8> },
    SummaryOnly { observed: u64, scope: SummaryScope },
}

impl Default for FlowState {
    fn default() -> Self {
        Self::Active {
            observed: 0,
            prefix: Vec::new(),
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
            Self::Active { observed, prefix } => {
                observe_active(config, direction, observed, prefix, payload)
            }
            Self::SummaryOnly { observed, scope } => observe_summary(observed, scope, payload),
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
enum StepEmission {
    Pass,
    Summary(FlowSummary),
    Drop,
}

fn observe_summary(observed: u64, scope: SummaryScope, payload: &[u8]) -> FlowStep {
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
                }
            };
            FlowStep {
                consumed,
                emission: StepEmission::Drop,
                next,
            }
        }
        SummaryScope::Unbounded => FlowStep {
            consumed: payload.len(),
            emission: StepEmission::Drop,
            next: FlowState::SummaryOnly {
                observed: observed.saturating_add(payload.len() as u64),
                scope: SummaryScope::Unbounded,
            },
        },
    }
}

fn observe_active(
    config: FlowControlConfig,
    direction: PayloadDirection,
    observed: u64,
    mut prefix: Vec<u8>,
    payload: &[u8],
) -> FlowStep {
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
                FlowState::Active { observed, prefix }
            };
            return FlowStep {
                consumed,
                emission: StepEmission::Pass,
                next,
            };
        }
    }
    if let Some(summary) = http2::classify(config, direction, observed_if_all, &prefix)
        .or_else(|| classify_binary_prefix(config, observed_if_all, &prefix))
        .or_else(|| classify_unknown_threshold(config, observed_if_all, &prefix))
    {
        return FlowStep {
            consumed: payload.len(),
            emission: StepEmission::Summary(summary),
            next: FlowState::SummaryOnly {
                observed: observed_if_all,
                scope: SummaryScope::Unbounded,
            },
        };
    }
    FlowStep {
        consumed: payload.len(),
        emission: StepEmission::Pass,
        next: FlowState::Active {
            observed: observed_if_all,
            prefix,
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
        },
        None => FlowState::SummaryOnly {
            observed,
            scope: SummaryScope::Unbounded,
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
