use export_core::{ExportError, ExportRuntime};
use export_otel_jsonl::{
    build_otel_jsonl_observation_consumer,
    build_otel_jsonl_observation_consumer_instance_with_subscriptions,
    parse_otel_jsonl_plugin_config,
};
use std::sync::Arc;

use plugin_system::{AlertHost, PluginHostGrants, PostTraceHost};
use plugin_system::{ObservationConsumer, PluginManifest, PluginPurpose, PluginRuntimeKind};

use crate::{ExportConfig, ExportDeliveryConfig, ExportRouteTargetConfig};

pub fn build_export_runtime(config: &ExportConfig) -> Result<ExportRuntime, ExportError> {
    if !config.enabled {
        return Ok(ExportRuntime::new(Vec::new()));
    }
    if config.routes().iter().all(|route| !route.enabled) {
        return Err(ExportError::new(
            "export_factory",
            "export enabled but no enabled export routes configured",
        ));
    }
    let mut consumers = Vec::new();
    for route in config.routes().iter().filter(|route| route.enabled) {
        route
            .target
            .validate_enabled_route()
            .map_err(|message| ExportError::new("export_factory", message))?;
        match route.delivery {
            ExportDeliveryConfig::BestEffort => match &route.target {
                ExportRouteTargetConfig::OtelJsonl(otel_jsonl) => {
                    let consumer = build_otel_jsonl_observation_consumer(otel_jsonl.clone())?;
                    consumers.push(Box::new(consumer) as Box<dyn ObservationConsumer>);
                }
            },
        }
    }
    Ok(export_core::ExportRuntime::new(consumers))
}

pub fn build_observation_consumer_from_manifest(
    instance_id: &str,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
    host_grants: PluginHostGrants,
    post_trace_host: Option<Arc<dyn PostTraceHost>>,
    alert_host: Option<Arc<dyn AlertHost>>,
) -> Result<Box<dyn ObservationConsumer>, ExportError> {
    if manifest.role() != PluginPurpose::ObservationConsumer {
        return Err(ExportError::new(
            "plugin_factory",
            format!(
                "plugin {} role {} is not supported by observation runtime",
                manifest.id(),
                manifest.role().as_str()
            ),
        ));
    }
    match manifest.runtime_kind() {
        PluginRuntimeKind::Builtin => build_builtin_consumer(instance_id, manifest, plugin_config),
        PluginRuntimeKind::Wasm => {
            let consumer = plugin_wasm_runtime::build_wasm_observation_consumer(
                instance_id,
                manifest,
                plugin_config,
                host_grants,
                post_trace_host,
                alert_host,
            )
            .map_err(|error| ExportError::new(error.code, error.message))?;
            Ok(Box::new(consumer))
        }
        PluginRuntimeKind::NativeDylib => Err(ExportError::new(
            "plugin_factory",
            "native dynamic plugins are not enabled",
        )),
    }
}

fn build_builtin_consumer(
    instance_id: &str,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
) -> Result<Box<dyn ObservationConsumer>, ExportError> {
    match manifest.id() {
        "otel-jsonl" => {
            if !manifest.capabilities().is_empty() {
                let capabilities = manifest
                    .capabilities()
                    .iter()
                    .map(|capability| capability.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(ExportError::new(
                    "plugin_factory",
                    format!(
                        "builtin plugin {} does not support host capabilities: {capabilities}",
                        manifest.id()
                    ),
                ));
            }
            let raw_config = plugin_config.ok_or_else(|| {
                ExportError::new("plugin_factory", "otel-jsonl plugin config is required")
            })?;
            let config = parse_otel_jsonl_plugin_config(raw_config)
                .map_err(|message| ExportError::new("plugin_factory", message))?;
            Ok(Box::new(
                build_otel_jsonl_observation_consumer_instance_with_subscriptions(
                    instance_id,
                    config,
                    manifest.observation_event_families(),
                )?,
            ))
        }
        _ => Err(ExportError::new(
            "plugin_factory",
            format!("unknown builtin plugin {}", manifest.id()),
        )),
    }
}
