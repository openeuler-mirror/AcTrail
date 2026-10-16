use super::query::{optional_query_param, parse_u64};
use super::{Response, STATUS_BAD_REQUEST, STATUS_NOT_FOUND, WebContext};
use model_core::ids::TraceId;
use resource_profiling::{
    model::ResourceSample,
    storage::{
        ResourceStorageConfig, ResourceStorageFactory,
        database::{ResourceKind, ResourceQuery},
    },
};

pub(super) struct ResourceHttp<'a> {
    context: &'a WebContext,
}
impl<'a> ResourceHttp<'a> {
    pub(super) fn new(context: &'a WebContext) -> Self {
        Self { context }
    }
    pub(super) fn route(&self, path: &str, query: &str) -> Option<Result<Response, String>> {
        let suffix = path.strip_prefix("/api/traces/")?;
        let (trace, part) = suffix.split_once('/')?;
        if part != "resources" {
            return None;
        }
        Some(self.read(trace, query))
    }
    fn read(&self, trace: &str, query: &str) -> Result<Response, String> {
        if self.context.cluster_root.is_some() {
            return Ok(Response::text(
                STATUS_NOT_FOUND,
                "resource samples are queried from the originating host",
            ));
        }
        let Some(config) = self.context.operator_config.as_ref() else {
            return Ok(Response::text(
                STATUS_NOT_FOUND,
                "resource storage is not configured",
            ));
        };
        let id = match parse_u64(trace) {
            Ok(id) => TraceId::new(id),
            Err(e) => return Ok(Response::text(STATUS_BAD_REQUEST, e)),
        };
        let limit = match optional_query_param(query, "limit")?
            .unwrap_or_else(|| "100".into())
            .parse::<u32>()
        {
            Ok(n) if n > 0 => n,
            _ => {
                return Ok(Response::text(
                    STATUS_BAD_REQUEST,
                    "limit must be a positive u32",
                ));
            }
        };
        let mut storage = ResourceStorageConfig::parse(
            config
                .resource_profiling
                .get("storage")
                .and_then(toml::Value::as_table)
                .unwrap_or(&toml::Table::new()),
        )?;
        let path = self
            .context
            .operator_config_path
            .as_deref()
            .ok_or("resource configuration path is missing")?;
        storage.resolve_paths(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
        let backend = ResourceStorageFactory::open_database(storage).map_err(|e| e.to_string())?;
        let mut query = ResourceQuery::new(ResourceKind::Workload, limit);
        query.trace_id = Some(id);
        let samples = backend.query(&query).map_err(|e| e.to_string())?;
        let resources: Vec<_> = samples
            .into_iter()
            .filter_map(|s| {
                if let ResourceSample::Workload(s) = s {
                    Some(s)
                } else {
                    None
                }
            })
            .collect();
        Ok(Response::json(
            serde_json::json!({"resources":resources}).to_string(),
        ))
    }
}
