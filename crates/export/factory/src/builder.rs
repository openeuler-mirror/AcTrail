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

/// Check a candidate config without building anything.
///
/// A builtin plugin's own parser enforces rules its JSON schema cannot express,
/// so a caller that stops at schema validation learns about them only while
/// constructing the instance — by which point the previous instance is already
/// gone. This runs exactly the parse `build_observation_consumer_from_manifest`
/// would run, and nothing else: no registration, no sockets, no instance id.
pub fn validate_observation_consumer_config(
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
) -> Result<(), ExportError> {
    // A WASM plugin owns its configuration and validates it over a hostcall;
    // there is nothing for the factory to parse here.
    if manifest.runtime_kind() != PluginRuntimeKind::Builtin {
        return Ok(());
    }
    parse_builtin_consumer_config(manifest, plugin_config).map(drop)
}

/// A builtin observation consumer's parsed configuration.
enum BuiltinConsumerConfig {
    OtelJsonl(Box<export_otel_jsonl::OtelJsonlExporterConfig>),
    OtelHttp(Box<export_otel_http::OtelHttpExporterConfig>),
}

fn parse_builtin_consumer_config(
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
) -> Result<BuiltinConsumerConfig, ExportError> {
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

    let plugin_id = manifest.id();
    match plugin_id {
        "otel-jsonl" => Ok(BuiltinConsumerConfig::OtelJsonl(Box::new(
            parse_otel_jsonl_plugin_config(required_plugin_config(plugin_id, plugin_config)?)
                .map_err(|message| ExportError::new("plugin_factory", message))?,
        ))),
        "otel-http" => Ok(BuiltinConsumerConfig::OtelHttp(Box::new(
            parse_otel_http_plugin_config(required_plugin_config(plugin_id, plugin_config)?)
                .map_err(|message| ExportError::new("plugin_factory", message))?,
        ))),
        other => Err(ExportError::new(
            "plugin_factory",
            format!("unknown builtin plugin {other}"),
        )),
    }
}

fn required_plugin_config<'a>(
    plugin_id: &str,
    plugin_config: Option<&'a str>,
) -> Result<&'a str, ExportError> {
    plugin_config.ok_or_else(|| {
        ExportError::new(
            "plugin_factory",
            format!("{plugin_id} plugin config is required"),
        )
    })
}

fn build_builtin_consumer(
    instance_id: &str,
    manifest: &PluginManifest,
    plugin_config: Option<&str>,
) -> Result<Box<dyn ObservationConsumer>, ExportError> {
    match parse_builtin_consumer_config(manifest, plugin_config)? {
        BuiltinConsumerConfig::OtelJsonl(config) => Ok(Box::new(
            build_otel_jsonl_observation_consumer_instance_with_subscriptions(
                instance_id,
                *config,
                manifest.observation_event_families(),
            )?,
        )),
        BuiltinConsumerConfig::OtelHttp(config) => Ok(Box::new(
            build_otel_http_observation_consumer_instance_with_subscriptions(
                instance_id,
                *config,
                manifest.observation_event_families(),
            )?,
        )),
    }
}
