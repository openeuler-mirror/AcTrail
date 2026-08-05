use export_core::ExportError;
use export_otel_http::{
    build_otel_http_observation_consumer_instance_with_subscriptions, parse_otel_http_plugin_config,
};
use export_otel_jsonl::{
    build_otel_jsonl_observation_consumer_instance_with_subscriptions,
    parse_otel_jsonl_plugin_config,
};
use std::sync::Arc;

use plugin_system::{AlertHost, PluginHostGrants, PostTraceHost};
use plugin_system::{ObservationConsumer, PluginManifest, PluginPurpose, PluginRuntimeKind};

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

    match manifest.id() {
        "otel-jsonl" => {
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
        "otel-http" => {
            let raw_config = plugin_config.ok_or_else(|| {
                ExportError::new("plugin_factory", "otel-http plugin config is required")
            })?;
            let config = parse_otel_http_plugin_config(raw_config)
                .map_err(|message| ExportError::new("plugin_factory", message))?;
            Ok(Box::new(
                build_otel_http_observation_consumer_instance_with_subscriptions(
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

#[cfg(test)]
mod tests {
    use super::build_observation_consumer_from_manifest;
    use plugin_system::{PluginHostGrants, PluginManifest};

    #[test]
    fn builtin_otel_http_loads_through_the_plugin_factory() {
        let manifest = toml::from_str::<PluginManifest>(include_str!(
            "../../../../examples/plugins/builtin/otel-http/otel-http.plugin.toml"
        ))
        .expect("parse otel-http manifest");
        manifest
            .validate_loadable()
            .expect("validate otel-http manifest");

        let consumer = build_observation_consumer_from_manifest(
            "builtin.otel-http",
            &manifest,
            Some(include_str!(
                "../../../../examples/plugins/builtin/otel-http/otel-http.config.toml"
            )),
            PluginHostGrants::default(),
            None,
            None,
        )
        .expect("build otel-http consumer");

        assert_eq!(consumer.instance_id(), "builtin.otel-http");
        assert_eq!(consumer.plugin_id(), "otel-http");
        assert!(!consumer.subscribed_event_families().is_empty());
    }
}
