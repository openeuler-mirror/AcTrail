use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::Serialize;

use super::ExchangeGroup;
use crate::actrail::plugin::types::{LlmExchangeRecord, TraceActivityContext};
use crate::{LlmTurnAnomalyConfig, submit_alert_with_dedup};

const ALERT_KEY_FREQUENCY: &str = "llm-high-frequency";

#[derive(Default)]
struct HighFrequencyState {
    total_exchanges: usize,
    timestamps: VecDeque<u64>,
}

#[derive(Clone, Default)]
struct FrequencyEpisode {
    anchor_ms: u64,
    tail_was_anomalous: bool,
}

#[derive(Default)]
pub(crate) struct HighFrequencyDetector {
    groups: BTreeMap<ExchangeGroup, HighFrequencyState>,
    episode: FrequencyEpisode,
}

impl HighFrequencyDetector {
    pub(crate) fn observe(
        &mut self,
        config: &LlmTurnAnomalyConfig,
        group: &ExchangeGroup,
        exchange: &LlmExchangeRecord,
    ) -> Result<(), String> {
        let state = self.groups.entry(group.clone()).or_default();
        state.total_exchanges = state
        .total_exchanges
        .checked_add(1)
        .ok_or_else(|| "high-frequency exchange count overflow".to_string())?;
        state.timestamps.push_back(exchange.started_at);
        let window_start = exchange
            .started_at
            .saturating_sub(config.high_frequency.window_size_ms);
        while state
            .timestamps
            .front()
            .is_some_and(|timestamp| *timestamp < window_start)
        {
            state.timestamps.pop_front();
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
    let rule = &config.high_frequency;
    let mut tail_anomalous = false;
    let mut tail_anchor = 0;
    let mut findings = Vec::new();
    let mut total_count = 0usize;
    for (group, detector) in &self.groups {
        if detector.total_exchanges < rule.min_exchanges {
            continue;
        }
        let count = detector.timestamps.len();
        if count >= rule.threshold {
            let window_start_ms = detector.timestamps.front().copied().unwrap_or(0);
            let window_end_ms = detector.timestamps.back().copied().unwrap_or(0);
            tail_anomalous = true;
            tail_anchor = window_start_ms;
            total_count += 1;
            if findings.len() < config.finding_max_count {
                findings.push(HighFrequencyFinding {
                    process_id: group.process_id.clone(),
                    model: group.model.clone(),
                    exchange_count: count,
                    window_start_ms,
                    window_end_ms,
                });
            }
        }
    }
    let episode = &mut self.episode;
    if tail_anomalous {
        if !episode.tail_was_anomalous {
            let payload = HighFrequencyPayload {
                root_container_id: context.root_container_id.clone(),
                root_process_id: context.root_process_id.clone(),
                display_name: context.display_name.clone(),
                profile_name: context.profile_name.clone(),
                window_size_ms: rule.window_size_ms,
                threshold: rule.threshold,
                truncated_count: total_count.saturating_sub(findings.len()),
                findings,
            };
            let dedup_key = format!("{ALERT_KEY_FREQUENCY}:{tail_anchor}");
            submit_alert_with_dedup(
                trace_id,
                alert_token,
                ALERT_KEY_FREQUENCY,
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
struct HighFrequencyFinding {
    process_id: String,
    model: Option<String>,
    exchange_count: usize,
    window_start_ms: u64,
    window_end_ms: u64,
}

#[derive(Serialize)]
struct HighFrequencyPayload {
    root_container_id: Option<String>,
    root_process_id: String,
    display_name: String,
    profile_name: String,
    window_size_ms: u64,
    threshold: usize,
    findings: Vec<HighFrequencyFinding>,
    truncated_count: usize,
}
