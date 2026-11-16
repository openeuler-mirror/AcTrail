use super::super::state::{
    JsonTextArtifact, PreparedCarrierSnapshot, ResponseTextWitness, ResponseWitnessObservation,
};
use super::rolling::RollingWindowIndex;
use super::{ArtifactFlow, ArtifactFlowObservation, CarrierSnapshotObservation, ContextShape};
use crate::llm_pipeline::provider::{RequestCarrierView, RequestHistorySource};

pub(in crate::llm_pipeline::projection::compact) struct ArtifactFlowDetector {
    max_artifact_bytes: usize,
    max_context_items: usize,
    max_context_scan_bytes: usize,
    min_flow_bytes: usize,
    min_coverage_percent: usize,
    max_fingerprint_positions: usize,
}

struct ArtifactFlowMatches<'a> {
    matcher: &'a ArtifactFlowDetector,
    response: &'a ResponseTextWitness,
    adoption: &'a PreparedCarrierSnapshot,
    carriers: RequestCarrierView<'a>,
    next_carrier: usize,
    exact: bool,
    skip_carriers: Vec<bool>,
    artifact: Option<&'a JsonTextArtifact>,
    pending_text: Option<usize>,
    index: Option<RollingWindowIndex<'a>>,
    work_limit_exceeded: bool,
    finished: bool,
}

impl ArtifactFlowDetector {
    pub(in crate::llm_pipeline::projection::compact) fn new(
        max_artifact_bytes: usize,
        max_context_items: usize,
        max_context_scan_bytes: usize,
        min_flow_bytes: usize,
        min_coverage_percent: usize,
        max_fingerprint_positions: usize,
    ) -> Self {
        Self {
            max_artifact_bytes,
            max_context_items,
            max_context_scan_bytes,
            min_flow_bytes,
            min_coverage_percent,
            max_fingerprint_positions,
        }
    }

    pub(in crate::llm_pipeline::projection::compact) fn request_shape(
        &self,
        context: &crate::llm_pipeline::provider::ParsedRequestContext,
    ) -> Option<ContextShape> {
        context.shape
    }

    pub(in crate::llm_pipeline::projection::compact) fn carrier_snapshot(
        &self,
        source: &RequestHistorySource,
    ) -> CarrierSnapshotObservation {
        source.carriers(self.max_context_items, self.max_context_scan_bytes)
    }

    pub(in crate::llm_pipeline::projection::compact) fn can_match_response(
        &self,
        response: &ResponseTextWitness,
    ) -> bool {
        response.byte_len() >= self.min_flow_bytes
    }

    pub(in crate::llm_pipeline::projection::compact) fn response_witness(
        &self,
        content: Option<String>,
    ) -> ResponseWitnessObservation {
        let Some(text) = content else {
            return ResponseWitnessObservation::Absent;
        };
        if text.len() > self.max_artifact_bytes {
            return ResponseWitnessObservation::LimitExceeded { bytes: text.len() };
        }
        ResponseTextWitness::new(text).map_or(
            ResponseWitnessObservation::Absent,
            ResponseWitnessObservation::Present,
        )
    }

    pub(super) fn match_flows<'a>(
        &'a self,
        response: &'a ResponseTextWitness,
        adoption: &'a PreparedCarrierSnapshot,
        source: &'a RequestHistorySource,
    ) -> impl Iterator<Item = ArtifactFlowObservation> + 'a {
        ArtifactFlowMatches {
            matcher: self,
            response,
            adoption,
            carriers: source.carrier_view(&adoption.snapshot),
            next_carrier: 0,
            exact: true,
            skip_carriers: vec![false; adoption.len()],
            artifact: None,
            pending_text: None,
            index: None,
            work_limit_exceeded: false,
            finished: false,
        }
    }
}

impl Iterator for ArtifactFlowMatches<'_> {
    type Item = ArtifactFlowObservation;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished || !self.matcher.can_match_response(self.response) {
            return None;
        }
        if self.exact {
            while self.next_carrier < self.adoption.len() {
                let position = self.next_carrier;
                self.next_carrier += 1;
                let Some((carrier, carrier_item)) = self.carriers.carrier(position) else {
                    self.skip_carriers[position] = true;
                    tracing::warn!(
                        target: "actrail::context_compact",
                        carrier_position = position,
                        "context compaction carrier could not be resolved from request source"
                    );
                    continue;
                };
                if carrier.contains(self.response.text()) {
                    self.skip_carriers[position] = true;
                    return Some(ArtifactFlowObservation::Present(ArtifactFlow {
                        chars: self.response.char_len(),
                        bytes: self.response.byte_len(),
                        carrier_item,
                        partial_summary: false,
                    }));
                }
            }
            self.exact = false;
            self.next_carrier = 0;
            self.artifact = self.response.json_artifact(self.matcher.min_flow_bytes);
        }
        loop {
            if let Some(position) = self.pending_text.take() {
                if let Some(flow) = self.match_text(position) {
                    return Some(ArtifactFlowObservation::Present(flow));
                }
                continue;
            }
            if self.next_carrier >= self.adoption.len() {
                self.finished = true;
                return self
                    .work_limit_exceeded
                    .then_some(ArtifactFlowObservation::WorkLimitExceeded);
            }
            let position = self.next_carrier;
            self.next_carrier += 1;
            // Full proofs and unreadable positions complete this carrier.
            // A consumer rejecting full evidence cannot accept a smaller proof
            // on the same role.
            if self.skip_carriers[position] {
                continue;
            }
            if let Some(flow) = self.match_json(position) {
                self.pending_text = Some(position);
                return Some(ArtifactFlowObservation::Structured(flow));
            }
            if let Some(flow) = self.match_text(position) {
                return Some(ArtifactFlowObservation::Present(flow));
            }
        }
    }
}

impl ArtifactFlowMatches<'_> {
    fn match_json(&self, position: usize) -> Option<ArtifactFlow> {
        let artifact = self.artifact?;
        let (carrier, carrier_item) = self.carriers.carrier(position)?;
        let match_carrier = |carrier: &str| {
            artifact.match_carrier(
                carrier,
                carrier_item,
                self.matcher.min_flow_bytes,
                self.matcher.min_coverage_percent,
            )
        };
        let raw = match_carrier(carrier);
        if raw.is_some_and(|flow| !flow.partial_summary) {
            return raw;
        }
        // Escaped values may complete a proof that is partial in the raw view.
        self.adoption
            .decoded_carrier(position, carrier)
            .and_then(match_carrier)
            .filter(|decoded| raw.is_none_or(|raw| decoded.bytes > raw.bytes))
            .or(raw)
    }

    fn match_text(&mut self, position: usize) -> Option<ArtifactFlow> {
        // Finish cheap structured evidence after exhaustion, so retained-history
        // rejection still precedes reporting the matching budget failure.
        if self.work_limit_exceeded {
            return None;
        }
        let (carrier, carrier_item) = self.carriers.carrier(position)?;
        let carrier = carrier.trim();
        if carrier.len() < self.matcher.min_flow_bytes {
            return None;
        }
        let index = self.index.get_or_insert_with(|| {
            RollingWindowIndex::new(
                self.response,
                self.matcher.min_flow_bytes,
                self.matcher.max_fingerprint_positions,
            )
        });
        let search = index.longest_verified_match(carrier.as_bytes());
        if let Some(span) = search.best
            && span.len() >= self.matcher.min_flow_bytes
            && span.len().saturating_mul(100)
                >= carrier
                    .len()
                    .saturating_mul(self.matcher.min_coverage_percent)
            && let Some(matched) = self
                .response
                .text()
                .get(span.response_start..span.response_end)
        {
            return Some(ArtifactFlow {
                chars: matched.chars().count(),
                bytes: matched.len(),
                carrier_item,
                partial_summary: matched.len() < self.response.byte_len(),
            });
        }
        self.work_limit_exceeded = search.work_limit_exceeded;
        None
    }
}
