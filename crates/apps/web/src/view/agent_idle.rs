//! Explicit, bounded reads of durable agent stall episodes.
use storage_core::IdleQuery;
use storage_factory::StorageConfig;

use super::open_storage;
use crate::json;

pub(crate) struct IdleView;

impl IdleView {
    pub(crate) fn render(config: &StorageConfig, query: &IdleQuery) -> Result<String, String> {
        let storage = open_storage(config)?;
        let page = storage
            .idle_page(query)
            .map_err(|error| format!("read idle intervals: {}: {}", error.stage, error.message))?;
        let intervals = page.intervals.iter().map(|interval| {
            format!("{{\"id\":{},\"session_id\":{},\"task_id\":{},\"kind\":\"hang\",\"start_time_unix_nanos\":{},\"end_time_unix_nanos\":{}}}",
                json::string(&interval.id), json::string(&interval.session_id), json::string(&interval.task_id),
                json::time_nanos(interval.start_time), json::optional_time_nanos(interval.end_time))
        }).collect::<Vec<_>>();
        let cursor = page
            .next_cursor
            .map(|value| json::string(&value.to_string()))
            .unwrap_or_else(|| "null".into());
        Ok(format!(
            "{{\"idle_intervals\":[{}],\"next_cursor\":{},\"has_more\":{},\"scanned\":{},\"axis_end_unix_nanos\":{}}}",
            intervals.join(","),
            cursor,
            page.has_more,
            page.scanned,
            json::time_nanos(page.axis_end)
        ))
    }
}
