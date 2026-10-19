//! Explicit idle-episode queries and startup-validated work budgets.
use model_core::ids::TraceId;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage_core::IdleQuery;

use super::query::{optional_query_param, parse_u64};

#[derive(Clone)]
pub(super) struct IdleLimits {
    default_limit: usize,
    max_limit: usize,
    candidates: usize,
}

impl IdleLimits {
    pub(super) fn load() -> Result<Self, String> {
        let limits = Self {
            default_limit: Self::setting("ACTRAIL_WEB_IDLE_DEFAULT_LIMIT", 100)?,
            max_limit: Self::setting("ACTRAIL_WEB_IDLE_MAX_LIMIT", 500)?,
            candidates: Self::setting("ACTRAIL_WEB_IDLE_CANDIDATES", 1000)?,
        };
        if limits.default_limit > limits.max_limit || limits.max_limit > limits.candidates {
            return Err("idle limits must satisfy default_limit <= max_limit <= candidates".into());
        }
        Ok(limits)
    }

    fn setting(name: &str, default: usize) -> Result<usize, String> {
        match std::env::var(name) {
            Ok(raw) => raw
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0 && *value < i64::MAX as usize)
                .ok_or_else(|| format!("{name} must be a positive integer below i64::MAX")),
            Err(std::env::VarError::NotPresent) => Ok(default),
            Err(error) => Err(format!("invalid {name}: {error}")),
        }
    }

    pub(super) fn parse(&self, trace_id: u64, raw: &str) -> Result<IdleQuery, String> {
        let limit = optional_query_param(raw, "limit")?
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| "invalid idle page limit".to_string())
            })
            .transpose()?
            .unwrap_or(self.default_limit);
        if limit == 0 || limit > self.max_limit {
            return Err(format!(
                "idle page limit must be between 1 and {}",
                self.max_limit
            ));
        }
        let from = optional_query_param(raw, "from")?
            .map(|value| Self::time(&value))
            .transpose()?;
        let to = optional_query_param(raw, "to")?
            .map(|value| Self::time(&value))
            .transpose()?;
        if from.zip(to).is_some_and(|(from, to)| from > to) {
            return Err("idle from must not exceed to".into());
        }
        let after = optional_query_param(raw, "cursor")?
            .map(|value| {
                parse_u64(&value).and_then(|id| {
                    if id == 0 || id > i64::MAX as u64 {
                        Err("invalid idle cursor".into())
                    } else {
                        Ok(id)
                    }
                })
            })
            .transpose()?;
        if trace_id == 0 || trace_id > i64::MAX as u64 {
            return Err("invalid idle trace ID".into());
        }
        Ok(IdleQuery {
            trace_id: TraceId::new(trace_id),
            from,
            to,
            after,
            limit,
            candidate_limit: self.candidates,
        })
    }

    fn time(raw: &str) -> Result<SystemTime, String> {
        let nanos = raw
            .parse::<u64>()
            .ok()
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or("invalid idle Unix nanoseconds")?;
        UNIX_EPOCH
            .checked_add(Duration::from_nanos(nanos))
            .ok_or_else(|| "idle timestamp overflow".into())
    }
}
