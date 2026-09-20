//! Historical stall intervals reconstructed from durable detector alerts.
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use alert_contract::AlertListLimit;
use model_core::ids::TraceId;
use serde::Deserialize;
use storage_core::StorageBackend;

use crate::json;

pub(super) struct AgentIdleInterval {
    id: u64,
    session_id: String,
    task_id: String,
    pub(super) start_time: SystemTime,
    pub(super) end_time: Option<SystemTime>,
}

impl AgentIdleInterval {
    pub(super) fn json(&self) -> String {
        format!(
            "{{\"id\":{},\"session_id\":{},\"task_id\":{},\"kind\":\"hang\",\"start_time_unix_nanos\":{},\"end_time_unix_nanos\":{}}}",
            json::string(&format!("agent-hang-{}", self.id)),
            json::string(&self.session_id),
            json::string(&self.task_id),
            json::time_nanos(self.start_time),
            json::optional_time_nanos(self.end_time),
        )
    }
}

#[derive(Deserialize)]
struct EpisodePayload {
    episode_key: String,
    session_id: String,
    task_id: String,
    started_at_unix_nanos: String,
    ended_at_unix_nanos: Option<String>,
}

impl EpisodePayload {
    fn time(raw: &str) -> Result<SystemTime, String> {
        let nanos = raw.parse::<u128>().map_err(|e| e.to_string())?;
        let seconds = u64::try_from(nanos / 1_000_000_000).map_err(|e| e.to_string())?;
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::new(seconds, (nanos % 1_000_000_000) as u32))
            .ok_or_else(|| "agent stall timestamp overflow".to_owned())
    }
}

pub(super) struct AgentIdleProjection;

impl AgentIdleProjection {
    pub(super) fn load(
        storage: &dyn StorageBackend,
        trace_id: TraceId,
    ) -> Result<Vec<AgentIdleInterval>, String> {
        // Complete history is required to pair opening and recovery records.
        let limit = AlertListLimit::new(i64::MAX as usize).expect("positive alert limit");
        let mut alerts = storage.trace_alerts(trace_id, limit).map_err(|error| {
            format!(
                "read agent stall alerts failed: {}: {}",
                error.stage, error.message
            )
        })?;
        alerts.sort_unstable_by_key(|view| view.record.alert_id);
        let trace = storage.get_trace(trace_id).map_err(|error| {
            format!(
                "read agent stall trace failed: {}: {}",
                error.stage, error.message
            )
        })?;
        let terminal = trace
            .filter(|trace| trace.lifecycle_state.is_terminal())
            .and_then(|trace| {
                [
                    trace.timings.completed_at,
                    trace.timings.exited_at,
                    trace.timings.failed_at,
                ]
                .into_iter()
                .flatten()
                .min()
            });
        let mut open = HashMap::<String, usize>::new();
        let mut intervals = Vec::<AgentIdleInterval>::new();
        for alert in alerts {
            if alert.definition.producer_plugin_id != "actrail.agent-idle" {
                continue;
            }
            let kind = alert.definition.definition_key.as_str();
            if !matches!(kind, "hang_detected" | "hang_recovered") {
                continue;
            }
            let payload: EpisodePayload = serde_json::from_str(&alert.record.payload_json)
                .map_err(|error| format!("invalid agent stall alert: {error}"))?;
            if kind == "hang_detected" {
                let start_time = EpisodePayload::time(&payload.started_at_unix_nanos)?;
                if terminal.is_some_and(|end| start_time >= end)
                    || open.contains_key(&payload.episode_key)
                {
                    continue;
                }
                open.insert(payload.episode_key, intervals.len());
                intervals.push(AgentIdleInterval {
                    id: alert.record.alert_id.get(),
                    session_id: payload.session_id,
                    task_id: payload.task_id,
                    start_time,
                    end_time: terminal,
                });
            } else if let Some(index) = open.remove(&payload.episode_key) {
                let raw = payload
                    .ended_at_unix_nanos
                    .as_deref()
                    .ok_or("agent recovery alert has no end time")?;
                let end = EpisodePayload::time(raw)?;
                intervals[index].end_time =
                    Some(terminal.map_or(end, |terminal| terminal.min(end)));
            }
        }
        intervals.retain(|interval| {
            interval
                .end_time
                .is_none_or(|end| end > interval.start_time)
        });
        Ok(intervals)
    }
}
