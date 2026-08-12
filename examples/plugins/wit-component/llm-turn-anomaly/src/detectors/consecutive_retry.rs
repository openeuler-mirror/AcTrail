use alloc::format;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Serialize;

use super::ExchangeGroup;
use crate::actrail::plugin::types::{LlmResponseStatus, TraceActivityContext};
use crate::detectors::ResponseOutcome;
use crate::{LlmTurnAnomalyConfig, submit_alert_with_dedup};

const ALERT_KEY_CONSECUTIVE_RETRY: &str = "llm-consecutive-retry";

#[derive(Default)]
struct ConsecutiveRetryState {
    consecutive: usize,
    first_action_id: Option<String>,
    last_action_id: Option<String>,
    first_started_at_ms: u64,
    last_started_at_ms: u64,
}

#[derive(Clone)]
struct RetryEpisode {
    anchor_ms: u64,
    tail_run_first_action_id: Option<String>,
}

impl Default for RetryEpisode {
    fn default() -> Self {
        Self {
            anchor_ms: 0,
            tail_run_first_action_id: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct ConsecutiveRetryDetector {
    groups: BTreeMap<ExchangeGroup, ConsecutiveRetryState>,
    episode: RetryEpisode,
}

impl ConsecutiveRetryDetector {
    pub(crate) fn observe(
        &mut self,
        config: &LlmTurnAnomalyConfig,
        group: &ExchangeGroup,
        outcome: &ResponseOutcome,
    ) -> Result<(), String> {
        let detector = self.groups.entry(group.clone()).or_default();
        let is_error = outcome.status == LlmResponseStatus::Error;
        let size_ok =
            outcome.request_body_bytes >= config.consecutive_retry.min_request_bytes as u64;
        if is_error && size_ok {
            if detector.consecutive == 0 {
                detector.first_action_id = Some(outcome.request_action_id.clone());
                detector.first_started_at_ms = outcome.started_at;
        }
        detector.consecutive = detector
            .consecutive
            .checked_add(1)
            .ok_or_else(|| "consecutive-retry count overflow".to_string())?;
            detector.last_action_id = Some(outcome.request_action_id.clone());
            detector.last_started_at_ms = outcome.started_at;
    } else {
        *detector = ConsecutiveRetryState::default();
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
    let rule = &config.consecutive_retry;
    let mut tail_active = false;
    let mut tail_first_action_id = None;
    let mut tail_anchor = 0;
    let mut tail_findings = Vec::new();
    for (group, detector) in &self.groups {
        if detector.consecutive >= rule.consecutive_count {
            tail_active = true;
            tail_first_action_id = detector.first_action_id.clone();
            tail_anchor = detector.first_started_at_ms;
            if tail_findings.len() < config.finding_max_count {
                tail_findings.push(ConsecutiveRetryFinding {
                    process_id: group.process_id.clone(),
                    model: group.model.clone(),
                    retry_length: detector.consecutive,
                    first_action_id: detector.first_action_id.clone().unwrap_or_default(),
                    last_action_id: detector.last_action_id.clone().unwrap_or_default(),
                    first_started_at_ms: detector.first_started_at_ms,
                    last_started_at_ms: detector.last_started_at_ms,
                });
            }
        }
    }
    let episode = &mut self.episode;
    if tail_active {
        if episode.tail_run_first_action_id != tail_first_action_id {
            let payload = ConsecutiveRetryPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                consecutive_count: rule.consecutive_count,
                findings: tail_findings,
                truncated_count: 0,
            };
            let dedup_key = format!("{ALERT_KEY_CONSECUTIVE_RETRY}:{tail_anchor}");
            submit_alert_with_dedup(
                trace_id,
                alert_token,
                ALERT_KEY_CONSECUTIVE_RETRY,
                &payload,
                &dedup_key,
            )?;
            episode.anchor_ms = tail_anchor;
            episode.tail_run_first_action_id = tail_first_action_id;
        }
    } else {
        episode.tail_run_first_action_id = None;
    }
    Ok(())
    }
}

#[derive(Serialize)]
struct ConsecutiveRetryFinding {
    process_id: String,
    model: Option<String>,
    retry_length: usize,
    first_action_id: String,
    last_action_id: String,
    first_started_at_ms: u64,
    last_started_at_ms: u64,
}

#[derive(Serialize)]
struct ConsecutiveRetryPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    consecutive_count: usize,
    findings: Vec<ConsecutiveRetryFinding>,
    truncated_count: usize,
}
