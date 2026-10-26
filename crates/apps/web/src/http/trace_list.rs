//! Numbered trace-list limits and metadata filters.
use super::query::{optional_query_param, parse_u64};
use model_core::ids::TraceId;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage_core::TracePageQuery;

#[derive(Clone)]
pub(super) struct TraceListLimits {
    pub(super) default_limit: usize,
    pub(super) max_limit: usize,
}

impl TraceListLimits {
    pub(super) fn parse_cluster(
        &self,
        raw: &str,
    ) -> Result<crate::view::cluster::ClusterTracePageQuery, String> {
        let (page, limit) = self.pagination(raw)?;
        let (start_time, end_time) = Self::times(raw)?;
        Ok(crate::view::cluster::ClusterTracePageQuery {
            page,
            limit,
            start_time,
            end_time,
            trace_id: optional_query_param(raw, "trace_id")?.filter(|value| !value.is_empty()),
            name: optional_query_param(raw, "name")?.filter(|value| !value.is_empty()),
        })
    }

    pub(super) fn load() -> Result<Self, String> {
        let result = Self {
            default_limit: Self::setting("ACTRAIL_WEB_TRACES_DEFAULT_LIMIT", 50)?,
            max_limit: Self::setting("ACTRAIL_WEB_TRACES_MAX_LIMIT", 250)?,
        };
        if result.default_limit > result.max_limit {
            return Err("trace limits must satisfy default_limit <= max_limit".into());
        }
        Ok(result)
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

    fn pagination(&self, raw: &str) -> Result<(usize, usize), String> {
        if optional_query_param(raw, "cursor")?.is_some() {
            return Err("trace list uses page rather than cursor".into());
        }
        let page = optional_query_param(raw, "page")?
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| "invalid trace page".to_string())
            })
            .transpose()?
            .unwrap_or(1);
        let limit = optional_query_param(raw, "limit")?
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| "invalid trace page limit".to_string())
            })
            .transpose()?
            .unwrap_or(self.default_limit);
        if page == 0 || limit == 0 || limit > self.max_limit {
            return Err(format!(
                "page must be positive and limit must be between 1 and {}",
                self.max_limit
            ));
        }
        page.checked_sub(1)
            .and_then(|page| page.checked_mul(limit))
            .filter(|offset| *offset <= i64::MAX as usize)
            .ok_or_else(|| "trace page offset exceeds SQLite range".to_string())?;
        Ok((page, limit))
    }

    fn times(raw: &str) -> Result<(Option<SystemTime>, Option<SystemTime>), String> {
        let start = optional_query_param(raw, "from")?
            .map(|value| Self::time(&value))
            .transpose()?;
        let end = optional_query_param(raw, "to")?
            .map(|value| Self::time(&value))
            .transpose()?;
        if start.zip(end).is_some_and(|(start, end)| start > end) {
            return Err("trace creation time from must not exceed to".into());
        }
        Ok((start, end))
    }

    pub(super) fn parse(&self, raw: &str) -> Result<TracePageQuery, String> {
        let (page, limit) = self.pagination(raw)?;
        let (start_time, end_time) = Self::times(raw)?;
        Ok(TracePageQuery {
            page,
            limit,
            start_time,
            end_time,
            name: optional_query_param(raw, "name")?.filter(|name| !name.is_empty()),
            trace_id: optional_query_param(raw, "trace_id")?
                .map(|value| parse_u64(&value).map(TraceId::new))
                .transpose()?,
        })
    }

    pub(super) fn time(raw: &str) -> Result<SystemTime, String> {
        let nanos = raw
            .parse::<u64>()
            .ok()
            .filter(|nanos| *nanos <= i64::MAX as u64)
            .ok_or_else(|| "invalid trace creation time nanoseconds".to_string())?;
        UNIX_EPOCH
            .checked_add(Duration::from_nanos(nanos))
            .ok_or_else(|| "trace timestamp overflow".into())
    }
}
