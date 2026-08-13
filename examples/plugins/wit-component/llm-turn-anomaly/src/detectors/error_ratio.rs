use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Serialize;

use super::ExchangeGroup;
use crate::actrail::plugin::types::{LlmResponseStatus, TraceActivityContext};
use crate::detectors::ResponseOutcome;
use crate::{LlmTurnAnomalyConfig, submit_alert_with_dedup};

const ALERT_KEY_ERROR_RATIO: &str = "llm-error-ratio";

#[derive(Default)]
struct ErrorRatioState {
    response_complete: VecDeque<bool>,
    started_at_ms: VecDeque<u64>,
    error_count: usize,
}

#[derive(Clone, Default)]
struct RatioEpisode {
    anchor_ms: u64,
    tail_was_anomalous: bool,
}

#[derive(Default)]
pub(crate) struct ErrorRatioDetector {
    groups: BTreeMap<ExchangeGroup, ErrorRatioState>,
    episode: RatioEpisode,
}

impl ErrorRatioDetector {
    pub(crate) fn observe(
        &mut self,
        config: &LlmTurnAnomalyConfig,
        group: &ExchangeGroup,
        outcome: &ResponseOutcome,
    ) -> Result<(), String> {
        if outcome.status == LlmResponseStatus::Unknown {
            return Ok(());
        }
        let detector = self.groups.entry(group.clone()).or_default();
        let succeeded = outcome.status == LlmResponseStatus::Success;
        detector.response_complete.push_back(succeeded);
        detector.started_at_ms.push_back(outcome.started_at);
        if outcome.status == LlmResponseStatus::Error {
        detector.error_count = detector
            .error_count
            .checked_add(1)
            .ok_or_else(|| "error-ratio count overflow".to_string())?;
    }
    while detector.response_complete.len() > config.error_ratio.window_size {
        if detector.response_complete.pop_front() == Some(false) {
            detector.error_count = detector.error_count.saturating_sub(1);
        }
        detector.started_at_ms.pop_front();
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
    let rule = &config.error_ratio;
    let mut tail_anomalous = false;
    let mut tail_anchor = 0;
    let mut findings = Vec::new();
    let mut total_count = 0usize;
    for (group, detector) in &self.groups {
        let window_total = detector.response_complete.len();
        if window_total < rule.minimum_exchanges || detector.error_count == 0 {
            continue;
        }
        let actual_ratio = (detector.error_count as u64) * 1000 / (window_total as u64);
        if actual_ratio >= rule.error_ratio_per_mille {
            tail_anomalous = true;
            tail_anchor = detector.started_at_ms.front().copied().unwrap_or(0);
            total_count += 1;
            if findings.len() < config.finding_max_count {
                findings.push(ErrorRatioFinding {
                    process_id: group.process_id.clone(),
                    model: group.model.clone(),
                    total_exchanges: window_total,
                    error_count: detector.error_count,
                    actual_ratio_per_mille: actual_ratio,
                });
            }
        }
    }
    let episode = &mut self.episode;
    if tail_anomalous {
        if !episode.tail_was_anomalous {
            let payload = ErrorRatioPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                minimum_exchanges: rule.minimum_exchanges,
                error_ratio_per_mille: rule.error_ratio_per_mille,
                window_size: rule.window_size,
                truncated_count: total_count.saturating_sub(findings.len()),
                findings,
            };
            let dedup_key = format!("{ALERT_KEY_ERROR_RATIO}:{tail_anchor}");
            submit_alert_with_dedup(
                trace_id,
                alert_token,
                ALERT_KEY_ERROR_RATIO,
                &payload,
                &dedup_key,
            )?;
            episode.anchor_ms = tail_anchor;
            episode.tail_was_anomalous = true;
        }
    } else {
        episode.tail_was_anomalous = false;
    }
    Ok(())
    }
}

#[derive(Serialize)]
struct ErrorRatioFinding {
    process_id: String,
    model: Option<String>,
    total_exchanges: usize,
    error_count: usize,
    actual_ratio_per_mille: u64,
}

#[derive(Serialize)]
struct ErrorRatioPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    minimum_exchanges: usize,
    error_ratio_per_mille: u64,
    window_size: usize,
    findings: Vec<ErrorRatioFinding>,
    truncated_count: usize,
}
