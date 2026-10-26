//! On-demand action content routing and HTTP body pages.
use model_core::ids::TraceId;
use storage_core::{ActionContentSource, HttpContentQuery};
use storage_factory::StorageConfig;

pub(crate) struct ActionContentView;

impl ActionContentView {
    pub(crate) fn source(
        config: &StorageConfig,
        trace: u64,
        action: &str,
    ) -> Result<String, String> {
        let storage = super::open_storage(config)?;
        let source = storage
            .action_content_source(TraceId::new(trace), action)
            .map_err(|e| format!("{}: {}", e.stage, e.message))?;
        let content = match source {
            Some(ActionContentSource::Llm { action_id }) => {
                serde_json::json!({"type":"llm","action_id":action_id})
            }
            Some(ActionContentSource::Http) => serde_json::json!({"type":"http"}),
            None => serde_json::Value::Null,
        };
        Ok(serde_json::json!({"content":content}).to_string())
    }

    pub(crate) fn http(
        config: &StorageConfig,
        trace: u64,
        action: &str,
        query: &HttpContentQuery,
    ) -> Result<String, String> {
        let storage = super::open_storage(config)?;
        let page = storage
            .http_content_page(TraceId::new(trace), action, query)
            .map_err(|e| format!("{}: {}", e.stage, e.message))?;
        let content=page.map(|p|serde_json::json!({"kind":p.kind,"text":p.text,"offset":p.offset,
            "next_offset":p.next_offset,"has_more":p.next_offset.is_some(),"total_bytes":p.total_bytes}));
        Ok(serde_json::json!({"content":content}).to_string())
    }
}
