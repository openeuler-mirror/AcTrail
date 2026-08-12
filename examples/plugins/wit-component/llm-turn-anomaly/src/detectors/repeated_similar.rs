use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Serialize;

use super::ExchangeGroup;
use crate::actrail::plugin::types::{LlmExchangeRecord, TraceActivityContext};
use crate::{LlmTurnAnomalyConfig, submit_alert_with_dedup};

const ALERT_KEY_REPEATED_SIMILAR: &str = "llm-repeated-similar";

struct SimilarRequestSample {
    request_action_id: String,
    request_body_bytes: u64,
    started_at_ms: u64,
}

#[derive(Default)]
struct RepeatedSimilarState {
    requests: VecDeque<SimilarRequestSample>,
    episode_alerted: bool,
}

#[derive(Default)]
pub(crate) struct RepeatedSimilarDetector {
    groups: BTreeMap<ExchangeGroup, RepeatedSimilarState>,
    pending_findings: VecDeque<RepeatedSimilarFinding>,
    pending_finding_count: usize,
}

impl RepeatedSimilarDetector {
    pub(crate) fn observe(
        &mut self,
        config: &LlmTurnAnomalyConfig,
        group: &ExchangeGroup,
        exchange: &LlmExchangeRecord,
    ) -> Result<(), String> {
        let rule = &config.repeated_similar;
        let finding = {
            let state = self.groups.entry(group.clone()).or_default();
            let continues = state.requests.back().is_some_and(|previous| {
                similar_request_bytes(
                    previous.request_body_bytes,
                    exchange.request_body_bytes,
                    rule.similarity_tolerance_ratio_per_mille,
                )
            });

            if !continues {
                state.requests.clear();
                state.episode_alerted = false;
            }

            state.requests.push_back(SimilarRequestSample {
                request_action_id: exchange.request_action_id.clone(),
                request_body_bytes: exchange.request_body_bytes,
                started_at_ms: exchange.started_at,
            });
            while state.requests.len() > rule.similarity_window {
                state.requests.pop_front();
            }

            if state.requests.len() >= rule.min_repeat_count && !state.episode_alerted {
                state.episode_alerted = true;
                let first = state.requests.front();
                let last = state.requests.back();
                first.zip(last).map(|(first, last)| RepeatedSimilarFinding {
                    process_id: group.process_id.clone(),
                    model: group.model.clone(),
                    repeat_count: state.requests.len(),
                    representative_action_id: first.request_action_id.clone(),
                    representative_request_bytes: first.request_body_bytes,
                    first_started_at_ms: first.started_at_ms,
                    last_started_at_ms: last.started_at_ms,
                })
            } else {
                None
            }
        };

        if let Some(finding) = finding {
            self.queue_finding(config, finding)?;
        }
        Ok(())
    }

    fn queue_finding(
        &mut self,
        config: &LlmTurnAnomalyConfig,
        finding: RepeatedSimilarFinding,
    ) -> Result<(), String> {
        self.pending_finding_count = self
            .pending_finding_count
            .checked_add(1)
            .ok_or_else(|| "repeated-similar finding count overflow".to_string())?;
        let position = self
            .pending_findings
            .iter()
            .position(|retained| repeated_similar_finding_cmp(&finding, retained).is_lt())
            .unwrap_or(self.pending_findings.len());
        if position < config.finding_max_count {
            self.pending_findings.insert(position, finding);
            if self.pending_findings.len() > config.finding_max_count {
                self.pending_findings.pop_back();
            }
        }
        Ok(())
    }

    pub(crate) fn evaluate(
        &mut self,
        trace_id: &str,
        alert_token: &[u8],
        context: &TraceActivityContext,
        config: &LlmTurnAnomalyConfig,
    ) -> Result<(), String> {
        let rule = &config.repeated_similar;
        let retained_count = self
            .pending_finding_count
            .min(config.finding_max_count);
        let truncated_count = self
            .pending_finding_count
            .saturating_sub(retained_count);

        while let Some(finding) = self.pending_findings.front().cloned() {
            let anchor = finding.first_started_at_ms;
            let action_id = finding.representative_action_id.clone();
            let payload = RepeatedSimilarPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                similarity_window: rule.similarity_window,
                similarity_tolerance_ratio_per_mille: rule.similarity_tolerance_ratio_per_mille,
                min_repeat_count: rule.min_repeat_count,
                findings: alloc::vec![finding],
                truncated_count,
            };
            let dedup_key = format!("{ALERT_KEY_REPEATED_SIMILAR}:{anchor}:{action_id}");
            submit_alert_with_dedup(
                trace_id,
                alert_token,
                ALERT_KEY_REPEATED_SIMILAR,
                &payload,
                &dedup_key,
            )?;
            self.pending_findings.pop_front();
        }
        self.pending_finding_count = 0;
        Ok(())
    }
}

fn repeated_similar_finding_cmp(
    left: &RepeatedSimilarFinding,
    right: &RepeatedSimilarFinding,
) -> core::cmp::Ordering {
    (
        &left.process_id,
        &left.model,
        left.first_started_at_ms,
        &left.representative_action_id,
    )
        .cmp(&(
            &right.process_id,
            &right.model,
            right.first_started_at_ms,
            &right.representative_action_id,
        ))
}

fn similar_request_bytes(a_bytes: u64, b_bytes: u64, tolerance_per_mille: u64) -> bool {
    if a_bytes == b_bytes {
        return true;
    }
    if tolerance_per_mille == 0 {
        return false;
    }
    let (larger, smaller) = if a_bytes >= b_bytes {
        (a_bytes, b_bytes)
    } else {
        (b_bytes, a_bytes)
    };
    if smaller == 0 {
        return larger == 0;
    }
    let diff = larger - smaller;
    diff * 1000 <= larger * tolerance_per_mille
}

#[derive(Clone, Serialize)]
struct RepeatedSimilarFinding {
    process_id: String,
    model: Option<String>,
    repeat_count: usize,
    representative_action_id: String,
    representative_request_bytes: u64,
    first_started_at_ms: u64,
    last_started_at_ms: u64,
}

#[derive(Serialize)]
struct RepeatedSimilarPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    similarity_window: usize,
    similarity_tolerance_ratio_per_mille: u64,
    min_repeat_count: usize,
    findings: Vec<RepeatedSimilarFinding>,
    truncated_count: usize,
}
