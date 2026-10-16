//! Snapshot-backed metadata and bounded semantic timeline queries.

use std::path::Path;
use storage_core::TimelineQuery;

use super::{row_by_uid, sqlite_storage_for_row, trace_row_json};

pub(crate) fn trace_metadata_json(root: &Path, trace_id: &str) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    if let Some((storage, local_id)) = sqlite_storage_for_row(&row)? {
        return row.project_identity(crate::view::trace_metadata_json(&storage, local_id)?);
    }
    Ok(format!(
        "{{\"trace\":{},\"timeline_bounds\":{{\"start_time_unix_nanos\":null,\"end_time_unix_nanos\":null}}}}",
        trace_row_json(&row)?
    ))
}

pub(crate) fn timeline_page_json(
    root: &Path,
    trace_id: &str,
    query: impl FnOnce(u64) -> Result<TimelineQuery, String>,
) -> Result<String, String> {
    let row = row_by_uid(root, trace_id)?;
    let (storage, local_id) = sqlite_storage_for_row(&row)?.ok_or_else(|| {
        format!(
            "cluster trace {} requires a trace.sqlite snapshot for timeline queries",
            row.trace_uid
        )
    })?;
    let query = query(local_id)?;
    crate::view::TimelineView::render(&storage, &query)
}
