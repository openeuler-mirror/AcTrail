use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use config_core::daemon::{OperatorConfig, StartupPluginLoadConfig};
use control_contract::command::{ControlCommand, PluginListCommand, PluginUnloadCommand};
use control_contract::reply::{ControlError, ControlReply};
use model_core::ids::RequestId;
use uds_control_client::{UdsControlClient, UdsSocketTransport};

use crate::json;

pub(crate) fn current_config_json(
    config_path: Option<&Path>,
    operator_config: Option<&OperatorConfig>,
) -> Result<String, String> {
    let Some(config) = operator_config else {
        return Ok(unavailable_json());
    };
    let text = config.to_hierarchical_toml()?;
    let plugin_count = config.startup_plugins.load.len();
    let plugin_enabled_count = config
        .startup_plugins
        .load
        .iter()
        .filter(|plugin| config.startup_plugins.enabled && plugin.enabled)
        .count();

    let mut summary = String::from("{");
    json::field(
        &mut summary,
        "socket_path",
        &json::string(&config.socket_path.display().to_string()),
    );
    summary.push(',');
    json::field(
        &mut summary,
        "storage_path",
        &json::string(&config.storage.path().display().to_string()),
    );
    summary.push(',');
    json::field(
        &mut summary,
        "listen_addr",
        &json::string(&config.web.listen_addr.to_string()),
    );
    summary.push(',');
    json::field(
        &mut summary,
        "startup_plugins_enabled",
        &json::boolean(config.startup_plugins.enabled),
    );
    summary.push(',');
    json::field(&mut summary, "plugin_count", &json::number(plugin_count));
    summary.push(',');
    json::field(
        &mut summary,
        "plugin_enabled_count",
        &json::number(plugin_enabled_count),
    );
    summary.push('}');

    let mut output = String::from("{");
    json::field(&mut output, "available", &json::boolean(true));
    output.push(',');
    json::field(&mut output, "source", &source_json(config_path));
    output.push(',');
    json::field(&mut output, "format", &json::string("toml"));
    output.push(',');
    json::field(&mut output, "summary", &summary);
    output.push(',');
    json::field(&mut output, "text", &json::string(&text));
    output.push('}');
    Ok(output)
}

pub(crate) fn plugin_enablement_json(
    config_path: Option<&Path>,
    operator_config: Option<&OperatorConfig>,
) -> Result<String, String> {
    let Some(config) = operator_config else {
        return Ok(unavailable_json());
    };

    let plugins = config
        .startup_plugins
        .load
        .iter()
        .map(|plugin| plugin_json(config, plugin))
        .collect::<Vec<_>>();
    let enabled_count = config
        .startup_plugins
        .load
        .iter()
        .filter(|plugin| config.startup_plugins.enabled && plugin.enabled)
        .count();

    let mut output = String::from("{");
    json::field(&mut output, "available", &json::boolean(true));
    output.push(',');
    json::field(&mut output, "source", &source_json(config_path));
    output.push(',');
    json::field(
        &mut output,
        "global_enabled",
        &json::boolean(config.startup_plugins.enabled),
    );
    output.push(',');
    json::field(
        &mut output,
        "global_failure_policy",
        &json::string(config.startup_plugins.failure_policy.as_str()),
    );
    output.push(',');
    json::field(
        &mut output,
        "configured_count",
        &json::number(config.startup_plugins.load.len()),
    );
    output.push(',');
    json::field(&mut output, "enabled_count", &json::number(enabled_count));
    output.push(',');
    json::field(&mut output, "plugins", &format!("[{}]", plugins.join(",")));
    output.push('}');
    Ok(output)
}

pub(crate) fn runtime_plugin_status_json(
    config_path: Option<&Path>,
    operator_config: Option<&OperatorConfig>,
) -> Result<String, String> {
    let Some(config) = operator_config else {
        return Ok(unavailable_json());
    };

    let mut client = UdsControlClient::new(UdsSocketTransport::new(config.socket_path.clone()));
    let reply = client.send(ControlCommand::PluginList(PluginListCommand {
        request_id: web_request_id()?,
    }));
    match reply {
        Ok(ControlReply::PluginList(items)) => {
            let plugins = items.iter().map(runtime_plugin_json).collect::<Vec<_>>();
            let active_count = items
                .iter()
                .filter(|item| item.state.as_str() == "active")
                .count();

            let mut output = String::from("{");
            json::field(&mut output, "available", &json::boolean(true));
            output.push(',');
            json::field(&mut output, "source", &source_json(config_path));
            output.push(',');
            json::field(
                &mut output,
                "socket_path",
                &json::string(&config.socket_path.display().to_string()),
            );
            output.push(',');
            json::field(&mut output, "plugin_count", &json::number(items.len()));
            output.push(',');
            json::field(&mut output, "active_count", &json::number(active_count));
            output.push(',');
            json::field(&mut output, "plugins", &format!("[{}]", plugins.join(",")));
            output.push('}');
            Ok(output)
        }
        Ok(_) => Err("daemon returned unexpected reply for plugin list".to_string()),
        Err(error) => Ok(runtime_unavailable_json(config_path, config, &error)),
    }
}

pub(crate) fn runtime_plugin_unload_json(
    config_path: Option<&Path>,
    operator_config: Option<&OperatorConfig>,
    instance_id: &str,
) -> Result<String, String> {
    let Some(config) = operator_config else {
        return Ok(unavailable_json());
    };
    if instance_id.trim().is_empty() {
        return Err("plugin instance id must not be empty".to_string());
    }

    let mut client = UdsControlClient::new(UdsSocketTransport::new(config.socket_path.clone()));
    let reply = client
        .send(ControlCommand::PluginUnload(PluginUnloadCommand {
            request_id: web_request_id()?,
            instance_id: instance_id.to_string(),
        }))
        .map_err(|error| {
            format!(
                "daemon plugin unload failed: {}: {}",
                error.code, error.message
            )
        })?;

    let ControlReply::PluginStatus(status) = reply else {
        return Err("daemon returned unexpected reply for plugin unload".to_string());
    };

    let mut output = String::from("{");
    json::field(&mut output, "available", &json::boolean(true));
    output.push(',');
    json::field(&mut output, "source", &source_json(config_path));
    output.push(',');
    json::field(
        &mut output,
        "socket_path",
        &json::string(&config.socket_path.display().to_string()),
    );
    output.push(',');
    json::field(&mut output, "unloaded", &json::boolean(true));
    output.push(',');
    json::field(&mut output, "plugin", &runtime_plugin_json(&status));
    output.push('}');
    Ok(output)
}

fn plugin_json(config: &OperatorConfig, plugin: &StartupPluginLoadConfig) -> String {
    let effective_enabled = config.startup_plugins.enabled && plugin.enabled;
    let effective_failure_policy = plugin
        .failure_policy
        .unwrap_or(config.startup_plugins.failure_policy)
        .as_str();
    let host_grants = plugin.host_grants.iter().cloned().collect::<Vec<_>>();

    let mut output = String::from("{");
    json::field(
        &mut output,
        "instance_id",
        &json::string(&plugin.instance_id),
    );
    output.push(',');
    json::field(
        &mut output,
        "effective_enabled",
        &json::boolean(effective_enabled),
    );
    output.push(',');
    json::field(
        &mut output,
        "configured_enabled",
        &json::boolean(plugin.enabled),
    );
    output.push(',');
    json::field(
        &mut output,
        "effective_failure_policy",
        &json::string(effective_failure_policy),
    );
    output.push(',');
    json::field(
        &mut output,
        "manifest_path",
        &json::string(&plugin.manifest_path.display().to_string()),
    );
    output.push(',');
    json::field(
        &mut output,
        "plugin_config_path",
        &json::optional_string(
            plugin
                .plugin_config_path
                .as_ref()
                .map(|path| path.display().to_string())
                .as_deref(),
        ),
    );
    output.push(',');
    json::field(&mut output, "host_grants", &json::string_array(host_grants));
    output.push('}');
    output
}

fn runtime_plugin_json(item: &plugin_system::PluginInstanceStatus) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "instance_id", &json::string(&item.instance_id));
    output.push(',');
    json::field(&mut output, "plugin_id", &json::string(&item.plugin_id));
    output.push(',');
    json::field(&mut output, "purpose", &json::string(item.purpose.as_str()));
    output.push(',');
    json::field(&mut output, "runtime", &json::string(item.runtime.as_str()));
    output.push(',');
    json::field(&mut output, "state", &json::string(item.state.as_str()));
    output.push(',');
    json::field(
        &mut output,
        "host_grants",
        &json::string_array(item.host_grants.iter().cloned()),
    );
    output.push(',');
    json::field(
        &mut output,
        "queue_depth",
        &json::optional_number(item.queue_depth),
    );
    output.push(',');
    json::field(
        &mut output,
        "queue_capacity",
        &json::optional_number(item.queue_capacity),
    );
    output.push(',');
    json::field(
        &mut output,
        "observed_records",
        &json::number(item.observed_records),
    );
    output.push(',');
    json::field(
        &mut output,
        "dropped_records",
        &json::number(item.dropped_records),
    );
    output.push(',');
    json::field(
        &mut output,
        "hostcall_metrics",
        &hostcall_metrics_json(&item.hostcall_metrics),
    );
    output.push(',');
    json::field(
        &mut output,
        "last_error",
        &json::optional_string(item.last_error.as_deref()),
    );
    output.push(',');
    json::field(
        &mut output,
        "warnings",
        &json::string_array(item.warnings.iter().cloned()),
    );
    output.push('}');
    output
}

fn hostcall_metrics_json(metrics: &plugin_system::PluginHostcallMetrics) -> String {
    let payload_read = metrics.payload_read;
    let mut output = String::from("{");
    json::field(
        &mut output,
        "payload_read",
        &payload_read_metrics_json(&payload_read),
    );
    output.push('}');
    output
}

fn payload_read_metrics_json(metrics: &plugin_system::PluginPayloadReadMetrics) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "calls", &json::number(metrics.calls));
    output.push(',');
    json::field(&mut output, "bytes", &json::number(metrics.bytes));
    output.push(',');
    json::field(&mut output, "denied", &json::number(metrics.denied));
    output.push(',');
    json::field(&mut output, "not_found", &json::number(metrics.not_found));
    output.push(',');
    json::field(&mut output, "invalid", &json::number(metrics.invalid));
    output.push(',');
    json::field(&mut output, "too_large", &json::number(metrics.too_large));
    output.push(',');
    json::field(&mut output, "truncated", &json::number(metrics.truncated));
    output.push(',');
    json::field(
        &mut output,
        "latency_total_ns",
        &json::number(metrics.latency_total_ns),
    );
    output.push(',');
    json::field(
        &mut output,
        "latency_max_ns",
        &json::number(metrics.latency_max_ns),
    );
    output.push('}');
    output
}

fn runtime_unavailable_json(
    config_path: Option<&Path>,
    config: &OperatorConfig,
    error: &ControlError,
) -> String {
    let mut output = String::from("{");
    json::field(&mut output, "available", &json::boolean(false));
    output.push(',');
    json::field(
        &mut output,
        "reason",
        &json::string(&format!(
            "daemon plugin status unavailable: {}: {}",
            error.code, error.message
        )),
    );
    output.push(',');
    json::field(&mut output, "source", &source_json(config_path));
    output.push(',');
    json::field(
        &mut output,
        "socket_path",
        &json::string(&config.socket_path.display().to_string()),
    );
    output.push('}');
    output
}

fn unavailable_json() -> String {
    let mut output = String::from("{");
    json::field(&mut output, "available", &json::boolean(false));
    output.push(',');
    json::field(
        &mut output,
        "reason",
        &json::string("operator config was not loaded; actrailweb is running in storage-only mode"),
    );
    output.push(',');
    json::field(&mut output, "source", &source_json(None));
    output.push('}');
    output
}

fn source_json(config_path: Option<&Path>) -> String {
    let mut output = String::from("{");
    match config_path {
        Some(path) => {
            json::field(&mut output, "mode", &json::string("operator_config"));
            output.push(',');
            json::field(
                &mut output,
                "path",
                &json::string(&path.display().to_string()),
            );
        }
        None => {
            json::field(&mut output, "mode", &json::string("storage_only"));
            output.push(',');
            json::field(&mut output, "path", "null");
        }
    }
    output.push('}');
    output
}

fn web_request_id() -> Result<RequestId, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before unix epoch; cannot build control request id")?;
    let millis = u64::try_from(duration.as_millis())
        .map_err(|_| "system clock millis overflowed request id")?;
    Ok(RequestId::new(millis))
}
