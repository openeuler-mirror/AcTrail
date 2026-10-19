//! Numbered trace-list rendering with only the selected page's root processes.
use super::ProcessDisplayIndex;
use crate::view::open_storage;
use storage_core::TracePageQuery;
use storage_factory::StorageConfig;

pub(crate) fn trace_page_json(
    config: &StorageConfig,
    query: &TracePageQuery,
) -> Result<String, String> {
    let storage = open_storage(config)?;
    let page = storage
        .trace_page(query)
        .map_err(|error| format!("read trace page: {}: {}", error.stage, error.message))?;
    let rows = page
        .traces
        .iter()
        .map(|trace| {
            ProcessDisplayIndex::for_trace(storage.as_ref(), trace)
                .map(|display| display.render_trace(trace))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!(
        "{{\"traces\":[{}],\"page\":{},\"total\":{},\"total_pages\":{}}}",
        rows.join(","),
        page.page,
        page.total,
        page.total_pages
    ))
}
