use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Serialize;

use super::ExchangeGroup;
use crate::actrail::plugin::types::{LlmExchangeRecord, TraceActivityContext};
use crate::{LlmTurnAnomalyConfig, submit_alert_with_dedup};

const ALERT_KEY_CONTEXT_GROWTH: &str = "llm-context-growth";

#[derive(Default)]
struct ContextGrowthState {
    request_body_bytes: VecDeque<u64>,
    last_growth_action_id: Option<String>,
    last_growth_anchor_ms: u64,
}

#[derive(Clone)]
struct GrowthEpisode {
    anchor_ms: u64,
    last_alerted_action_id: Option<String>,
}

impl Default for GrowthEpisode {
    fn default() -> Self {
        Self {
            anchor_ms: 0,
            last_alerted_action_id: None,
        }
    }
}

#[derive(Default)]
pub(crate) struct ContextGrowthDetector {
    groups: BTreeMap<ExchangeGroup, ContextGrowthState>,
    findings: Vec<ContextGrowthFinding>,
    finding_count: usize,
    episode: GrowthEpisode,
}

impl ContextGrowthDetector {
    pub(crate) fn observe(
        &mut self,
        config: &LlmTurnAnomalyConfig,
        group: &ExchangeGroup,
        exchange: &LlmExchangeRecord,
    ) -> Result<(), String> {
    let rule = &config.context_growth;
    let detector = self.groups.entry(group.clone()).or_default();
    let baseline = if detector.request_body_bytes.len() >= rule.minimum_samples {
        let history = detector
            .request_body_bytes
            .iter()
            .copied()
            .collect::<Vec<_>>();
        Some(median(&history))
    } else {
        None
    };
    let bytes = exchange.request_body_bytes;
    let triggered = bytes >= rule.minimum_growth_bytes
        && baseline.is_some_and(|baseline| {
            baseline >= rule.minimum_baseline_bytes
                && bytes.saturating_sub(baseline) >= rule.minimum_growth_bytes
                && u128::from(bytes) * 1000
                    >= u128::from(baseline) * u128::from(rule.growth_ratio_per_mille)
        });
    if triggered {
        let baseline = baseline.unwrap_or(0);
        let ratio = if baseline > 0 {
            let ratio = u128::from(bytes) * 1000 / u128::from(baseline);
            ratio.min(u128::from(u64::MAX)) as u64
        } else {
            0
        };
        self.finding_count = self
            .finding_count
            .checked_add(1)
            .ok_or_else(|| "context-growth finding count overflow".to_string())?;
        detector.last_growth_action_id = Some(exchange.request_action_id.clone());
        detector.last_growth_anchor_ms = exchange.started_at;
        let finding = ContextGrowthFinding {
            action_id: exchange.request_action_id.clone(),
            call_action_id: exchange.call_action_id.clone(),
            process_id: exchange.process_id.clone(),
            model: exchange.model.clone(),
            observed_bytes: bytes,
            baseline_median_bytes: baseline,
            observed_ratio_per_mille: ratio,
            started_at_ms: exchange.started_at,
        };
        let position = self
            .findings
            .binary_search_by(|retained| context_growth_finding_cmp(retained, &finding))
            .unwrap_or_else(|position| position);
        if position < config.finding_max_count {
            self.findings.insert(position, finding);
            if self.findings.len() > config.finding_max_count {
                self.findings.pop();
            }
        }
    }
    detector.request_body_bytes.push_back(bytes);
    while detector.request_body_bytes.len() > rule.window_size {
        detector.request_body_bytes.pop_front();
    }
        Ok(())
    }
}

fn context_growth_finding_cmp(
    left: &ContextGrowthFinding,
    right: &ContextGrowthFinding,
) -> core::cmp::Ordering {
    (
        &left.process_id,
        &left.model,
        left.started_at_ms,
        &left.action_id,
    )
        .cmp(&(
            &right.process_id,
            &right.model,
            right.started_at_ms,
            &right.action_id,
        ))
}

fn median(values: &[u64]) -> u64 {
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let midpoint = ordered.len() / 2;
    if ordered.len() % 2 == 1 {
        ordered[midpoint]
    } else {
        ((u128::from(ordered[midpoint - 1]) + u128::from(ordered[midpoint])) / 2) as u64
    }
}

impl ContextGrowthDetector {
    pub(crate) fn evaluate(
        &mut self,
        trace_id: &str,
        alert_token: &[u8],
        context: &TraceActivityContext,
        config: &LlmTurnAnomalyConfig,
    ) -> Result<(), String> {
    let rule = &config.context_growth;
    let tail = self
        .groups
        .iter()
        .rev()
        .find_map(|(_, detector)| {
            detector
                .last_growth_action_id
                .clone()
                .map(|action_id| (action_id, detector.last_growth_anchor_ms))
        });
    let Some((action_id, anchor)) = tail else {
        self.episode.last_alerted_action_id = None;
        return Ok(());
    };
    if self.episode.last_alerted_action_id.as_ref() == Some(&action_id) {
        return Ok(());
    }
    let payload = ContextGrowthPayload {
        root_container_id: context.root_container_id.clone(),
        root_process_id: context.root_process_id.clone(),
        display_name: context.display_name.clone(),
        profile_name: context.profile_name.clone(),
        growth_ratio_per_mille: rule.growth_ratio_per_mille,
        minimum_baseline_bytes: rule.minimum_baseline_bytes,
        minimum_growth_bytes: rule.minimum_growth_bytes,
        window_size: rule.window_size,
        minimum_samples: rule.minimum_samples,
        findings: self.findings.clone(),
        truncated_count: self.finding_count.saturating_sub(self.findings.len()),
    };
    let dedup_key = format!("{ALERT_KEY_CONTEXT_GROWTH}:{anchor}:{action_id}");
    submit_alert_with_dedup(
        trace_id,
        alert_token,
        ALERT_KEY_CONTEXT_GROWTH,
        &payload,
        &dedup_key,
    )?;
    self.episode.anchor_ms = anchor;
    self.episode.last_alerted_action_id = Some(action_id);
    Ok(())
    }
}

#[derive(Clone, Serialize)]
struct ContextGrowthFinding {
    action_id: String,
    call_action_id: String,
    process_id: String,
    model: Option<String>,
    observed_bytes: u64,
    baseline_median_bytes: u64,
    observed_ratio_per_mille: u64,
    started_at_ms: u64,
}

#[derive(Serialize)]
struct ContextGrowthPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    growth_ratio_per_mille: u64,
    minimum_baseline_bytes: u64,
    minimum_growth_bytes: u64,
    window_size: usize,
    minimum_samples: usize,
    findings: Vec<ContextGrowthFinding>,
    truncated_count: usize,
}
