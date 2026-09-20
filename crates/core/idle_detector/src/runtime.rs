use crate::{IdleAlert, IdleDetectionConfig};
use agent_host::{ExecutionKey, ExecutionStates};
use model_core::ids::TraceId;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

struct Episode {
    since: Instant,
    started_at: SystemTime,
}

/// A single periodic scan; execution state remains owned by the agent integration.
pub struct IdleDetector {
    config: IdleDetectionConfig,
    next_poll: Instant,
    active: HashMap<ExecutionKey, Episode>,
}

impl IdleDetector {
    pub const PRODUCER_ID: &'static str = crate::alert::PRODUCER_ID;

    pub fn alert_definitions() -> Vec<(alert_contract::AlertDefinition, serde_json::Value)> {
        IdleAlert::definitions()
    }

    pub fn new(config: IdleDetectionConfig) -> Result<Self, String> {
        config.validate()?;
        let next_poll = Instant::now()
            .checked_add(config.poll_interval)
            .ok_or("idle detection clock overflow")?;
        Ok(Self {
            config,
            next_poll,
            active: HashMap::new(),
        })
    }

    pub fn poll_timeout(&self) -> Option<Duration> {
        self.config
            .enabled
            .then(|| self.next_poll.saturating_duration_since(Instant::now()))
    }

    pub fn poll(&mut self, executions: &ExecutionStates) -> Vec<IdleAlert> {
        if !self.config.enabled {
            return Vec::new();
        }
        let now = Instant::now();
        if now < self.next_poll {
            return Vec::new();
        }
        let Some(next_poll) = now.checked_add(self.config.poll_interval) else {
            // Failure of this optional consumer must not busy-loop the host.
            self.config.enabled = false;
            return Vec::new();
        };
        self.next_poll = next_poll;
        let observed_at = SystemTime::now();
        let mut alerts = Vec::new();
        self.active.retain(|key, episode| {
            let continuing = executions.get(key).is_some_and(|state| {
                // Missing lifecycle information is not evidence of recovery.
                state.is_unknown()
                    || state
                        .candidate_since()
                        .is_some_and(|(since, _)| since == episode.since)
            });
            if !continuing {
                alerts.push(IdleAlert::new(
                    key,
                    episode.started_at,
                    observed_at,
                    self.config.threshold,
                    true,
                ));
            }
            continuing
        });
        for (key, state) in executions.iter() {
            let Some((since, started_at)) = state.candidate_since() else {
                continue;
            };
            if now.saturating_duration_since(since) < self.config.threshold
                || self.active.contains_key(key)
            {
                continue;
            }
            alerts.push(IdleAlert::new(
                key,
                started_at,
                observed_at,
                self.config.threshold,
                false,
            ));
            self.active
                .insert(key.clone(), Episode { since, started_at });
        }
        alerts
    }

    pub fn finish_trace(&mut self, trace_id: TraceId) {
        self.active.retain(|key, _| key.trace_id != trace_id);
    }
}
