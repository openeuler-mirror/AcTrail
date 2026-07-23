use control_contract::reply::{PluginCommandReply, PluginConfigReply, PluginConfigValidationReply};
use plugin_system::{PluginHostcallMetrics, PluginInstanceStatus};
use serde_json::{Value, json};

use super::catalog::{CatalogPackage, CatalogSnapshot};

pub(crate) fn catalog_json(snapshot: &CatalogSnapshot) -> Result<String, String> {
    let packages = snapshot
        .packages
        .iter()
        .map(package_value)
        .collect::<Vec<_>>();
    let runtime_plugins = snapshot
        .runtime_plugins
        .iter()
        .map(plugin_status_value)
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({
        "available": true,
        "source": {
            "mode": "operator_config",
            "path": snapshot.config_path.as_ref().map(|path| path.display().to_string()),
        },
        "directory": snapshot.directory.display().to_string(),
        "package_count": packages.len(),
        "runtime_available": snapshot.runtime_error.is_none(),
        "runtime_error": snapshot.runtime_error.as_deref(),
        "runtime_plugin_count": runtime_plugins.len(),
        "packages": packages,
        "runtime_plugins": runtime_plugins,
    }))
    .map_err(|error| format!("serialize installed plugin catalog failed: {error}"))
}

pub(crate) fn plugin_status_json(status: &PluginInstanceStatus) -> Result<String, String> {
    serde_json::to_string(&json!({
        "available": true,
        "plugin": plugin_status_value(status),
    }))
    .map_err(|error| format!("serialize plugin status failed: {error}"))
}

pub(crate) fn plugin_command_json(reply: &PluginCommandReply) -> Result<String, String> {
    serde_json::to_string(&json!({
        "available": true,
        "command": {
            "instance_id": &reply.instance_id,
            "exit_code": reply.exit_code,
            "stdout": &reply.stdout,
            "stderr": &reply.stderr,
        },
    }))
    .map_err(|error| format!("serialize plugin command reply failed: {error}"))
}

pub(crate) fn plugin_config_json(reply: &PluginConfigReply) -> Result<String, String> {
    let config = serde_json::from_str::<Value>(&reply.config_json)
        .map_err(|error| format!("parse daemon plugin config JSON failed: {error}"))?;
    let schema = serde_json::from_str::<Value>(&reply.schema_json)
        .map_err(|error| format!("parse daemon plugin schema JSON failed: {error}"))?;
    serde_json::to_string(&json!({
        "available": true,
        "instance_id": &reply.instance_id,
        "plugin_id": &reply.plugin_id,
        "editable": reply.editable,
        "config": config,
        "schema": schema,
    }))
    .map_err(|error| format!("serialize plugin config failed: {error}"))
}

pub(crate) fn plugin_config_validation_json(
    reply: &PluginConfigValidationReply,
) -> Result<String, String> {
    serde_json::to_string(&json!({
        "available": true,
        "instance_id": &reply.instance_id,
        "valid": reply.valid,
        "errors": &reply.errors,
    }))
    .map_err(|error| format!("serialize plugin config validation failed: {error}"))
}

pub(crate) fn unavailable_catalog_json() -> String {
    json!({
        "available": false,
        "reason": "operator config was not loaded; plugin discovery is unavailable in storage-only mode",
        "packages": [],
        "runtime_plugins": [],
    })
    .to_string()
}

fn package_value(item: &CatalogPackage) -> Value {
    json!({
        "package_key": &item.package.key,
        "package_path": item.package.package_path.display().to_string(),
        "manifest_path": item.package.manifest_path.as_ref().map(|path| path.display().to_string()),
        "plugin_config_path": item.package.plugin_config_path.as_ref().map(|path| path.display().to_string()),
        "plugin_id": item.package.plugin_id.as_deref(),
        "purpose": item.package.purpose.as_ref().map(|purpose| purpose.as_str()),
        "runtime": item.package.runtime.as_ref().map(|runtime| runtime.as_str()),
        "requested_capabilities": &item.package.requested_capabilities,
        "automatic_host_grants": &item.package.automatic_host_grants,
        "parameterized_host_grants": &item.package.parameterized_host_grants,
        "warnings": &item.package.warnings,
        "issue": item.package.issue.as_deref(),
        "activation_ready": item.package.activation_ready(),
        "loaded_instances": item.loaded_instances.as_ref(),
    })
}

fn plugin_status_value(item: &PluginInstanceStatus) -> Value {
    json!({
        "instance_id": &item.instance_id,
        "plugin_id": &item.plugin_id,
        "purpose": item.purpose.as_str(),
        "runtime": item.runtime.as_str(),
        "state": item.state.as_str(),
        "host_grants": &item.host_grants,
        "queue_depth": item.queue_depth,
        "queue_capacity": item.queue_capacity,
        "observed_records": item.observed_records,
        "dropped_records": item.dropped_records,
        "hostcall_metrics": hostcall_metrics_value(&item.hostcall_metrics),
        "last_error": item.last_error.as_deref(),
        "warnings": &item.warnings,
    })
}

fn hostcall_metrics_value(metrics: &PluginHostcallMetrics) -> Value {
    let payload = metrics.payload_read;
    json!({
        "payload_read": {
            "calls": payload.calls,
            "bytes": payload.bytes,
            "denied": payload.denied,
            "not_found": payload.not_found,
            "invalid": payload.invalid,
            "too_large": payload.too_large,
            "truncated": payload.truncated,
            "latency_total_ns": payload.latency_total_ns,
            "latency_max_ns": payload.latency_max_ns,
        }
    })
}
