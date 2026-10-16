//! Startup-validated budgets and query decoding for semantic timelines.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use model_core::{ids::TraceId, process::ProcessIdentity};
use semantic_action::SemanticActionKind;
use storage_core::{TimelineCursor, TimelineQuery};

use super::query::{optional_query_param, parse_u64};

#[derive(Clone)]
pub(super) struct TimelineLimits {
    default_limit: usize,
    max_limit: usize,
    candidates: usize,
    links: usize,
}

impl TimelineLimits {
    pub(super) fn load() -> Result<Self, String> {
        let limits = Self {
            default_limit: Self::setting("ACTRAIL_WEB_TIMELINE_DEFAULT_LIMIT", 250)?,
            max_limit: Self::setting("ACTRAIL_WEB_TIMELINE_MAX_LIMIT", 1000)?,
            candidates: Self::setting("ACTRAIL_WEB_TIMELINE_CANDIDATES", 2000)?,
            links: Self::setting("ACTRAIL_WEB_TIMELINE_LINKS", 2000)?,
        };
        if limits.default_limit > limits.max_limit || limits.max_limit > limits.candidates {
            return Err(
                "timeline limits must satisfy default_limit <= max_limit <= candidates".into(),
            );
        }
        Ok(limits)
    }

    fn setting(name: &str, default: usize) -> Result<usize, String> {
        match std::env::var(name) {
            Ok(value) => value
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0 && *value < i64::MAX as usize)
                .ok_or_else(|| format!("{name} must be a positive integer below i64::MAX")),
            Err(std::env::VarError::NotPresent) => Ok(default),
            Err(error) => Err(format!("invalid {name}: {error}")),
        }
    }

    pub(super) fn parse(&self, trace: &str, raw: &str) -> Result<TimelineQuery, String> {
        let limit = optional_query_param(raw, "limit")?
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| "invalid timeline limit".to_string())
            })
            .transpose()?
            .unwrap_or(self.default_limit);
        if limit == 0 || limit > self.max_limit {
            return Err(format!(
                "timeline limit must be between 1 and {}",
                self.max_limit
            ));
        }
        let kinds = optional_query_param(raw, "kinds")?
            .unwrap_or_default()
            .split(',')
            .filter(|kind| !kind.is_empty())
            .map(|kind| {
                SemanticActionKind::parse(kind)
                    .ok_or_else(|| format!("unknown action kind: {kind}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if kinds
            .iter()
            .map(|kind| kind.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > self.candidates
        {
            return Err("timeline candidate budget must cover every selected kind".into());
        }
        let process = optional_query_param(raw, "process_id")?
            .map(|value| parse_u64(&value).map(ProcessIdentity::new))
            .transpose()?;
        let start_time = optional_query_param(raw, "from")?
            .map(|value| Self::time(&value))
            .transpose()?;
        let end_time = optional_query_param(raw, "to")?
            .map(|value| Self::time(&value))
            .transpose()?;
        if start_time
            .zip(end_time)
            .is_some_and(|(start, end)| start >= end)
        {
            return Err("timeline from must be less than to".into());
        }
        let after = optional_query_param(raw, "cursor")?
            .map(|value| {
                let (time, key) = value.split_once(':').ok_or("invalid timeline cursor")?;
                let action_key = key
                    .parse::<i64>()
                    .ok()
                    .filter(|key| *key > 0)
                    .ok_or("invalid timeline cursor key")?;
                Ok::<_, String>(TimelineCursor {
                    start_time: Self::time(time)?,
                    action_key,
                })
            })
            .transpose()?;
        Ok(TimelineQuery {
            trace_id: TraceId::new(parse_u64(trace)?),
            kinds,
            process,
            start_time,
            end_time,
            after,
            limit,
            candidate_limit: self.candidates,
            link_limit: self.links,
        })
    }

    fn time(raw: &str) -> Result<SystemTime, String> {
        let nanos = raw
            .parse::<u64>()
            .map_err(|_| "invalid timeline unix nanoseconds".to_string())?;
        if nanos > i64::MAX as u64 {
            return Err("timeline timestamp exceeds SQLite nanosecond range".into());
        }
        UNIX_EPOCH
            .checked_add(Duration::from_nanos(nanos))
            .ok_or_else(|| "timeline timestamp overflow".into())
    }
}
