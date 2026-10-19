use crate::command::{StorageCommand, ViewInvocation};
use resource_profiling::{
    model::ResourceSample,
    storage::database::{DatabaseStorageConfig, DatabaseStorageFactory},
};
pub(crate) struct ResourceSource;
impl ResourceSource {
    pub(crate) fn render(invocation: &ViewInvocation) -> Result<String, String> {
        if invocation.storage_path.is_some() && invocation.storage_config_path.is_some() {
            return Err("--storage-path cannot be combined with --storage-config".into());
        }
        let backend = if let Some(path) = &invocation.storage_path {
            let mut config = DatabaseStorageConfig::default();
            config.sqlite.path = path.clone();
            DatabaseStorageFactory::open_reader(config).map_err(|e| e.to_string())?
        } else {
            let path = invocation
                .storage_config_path
                .as_ref()
                .unwrap_or(&invocation.config_path);
            let table: toml::Table =
                toml::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let mut storage = resource_profiling::storage::ResourceStorageConfig::parse(
                table
                    .get("resource_profiling")
                    .and_then(|resource| resource.get("storage"))
                    .and_then(toml::Value::as_table)
                    .unwrap_or(&toml::Table::new()),
            )?;
            storage.resolve_paths(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
            resource_profiling::storage::ResourceStorageFactory::open_database(storage)
                .map_err(|e| e.to_string())?
        };
        let query = invocation
            .resource_query
            .as_ref()
            .ok_or("missing resource query")?;
        let samples = backend.query(query).map_err(|e| e.to_string())?;
        match invocation.command {
            StorageCommand::HostResources => crate::host_resources::HostResourceView::render(
                samples
                    .into_iter()
                    .filter_map(|s| {
                        if let ResourceSample::Host(s) = s {
                            Some(s)
                        } else {
                            None
                        }
                    })
                    .collect(),
                invocation.output_format,
            ),
            _ => crate::workload_resources::WorkloadResourceView::render(
                samples
                    .into_iter()
                    .filter_map(|s| {
                        if let ResourceSample::Workload(s) = s {
                            Some(s)
                        } else {
                            None
                        }
                    })
                    .collect(),
                invocation.output_format,
            ),
        }
    }
}
