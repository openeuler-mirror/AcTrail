//! User-facing output projection for control responses.

use control_contract::reply::ControlReply;

pub fn format_reply(reply: &ControlReply) -> String {
    match reply {
        ControlReply::TrackAdded(reply) => format!(
            "trace {} entered {:?}",
            reply.trace_id, reply.lifecycle_state
        ),
        ControlReply::SeccompListenerRegistered => "seccomp listener registered".to_string(),
        ControlReply::TrackRemoved => "root capture removed".to_string(),
        ControlReply::TraceList(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{} {} pid={} {:?}/{:?}",
                    item.trace_id,
                    item.display_name,
                    item.root_pid,
                    item.lifecycle_state,
                    item.health
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ControlReply::Doctor(reply) => format!(
            "collectors={} plugins={} storage_ready={}",
            reply.available_collectors.join(","),
            reply.loaded_policy_plugins.join(","),
            reply.storage_ready
        ),
        ControlReply::PluginList(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{} {} {} {} {} queue={}/{} observed={} dropped={} warnings={}",
                    item.instance_id,
                    item.plugin_id,
                    item.purpose.as_str(),
                    item.runtime.as_str(),
                    item.state.as_str(),
                    item.queue_depth
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    item.queue_capacity
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    item.observed_records,
                    item.dropped_records,
                    printable_warnings(&item.warnings)
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        ControlReply::PluginStatus(item) => {
            let payload_read = item.hostcall_metrics.payload_read;
            format!(
                "instance={} plugin_id={} purpose={} runtime={} state={} queue={}/{} observed={} dropped={} payload_read_calls={} payload_read_bytes={} payload_read_denied={} payload_read_not_found={} payload_read_invalid={} payload_read_too_large={} payload_read_truncated={} payload_read_latency_total_ns={} payload_read_latency_max_ns={} warnings={}",
                item.instance_id,
                item.plugin_id,
                item.purpose.as_str(),
                item.runtime.as_str(),
                item.state.as_str(),
                item.queue_depth
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                item.queue_capacity
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                item.observed_records,
                item.dropped_records,
                payload_read.calls,
                payload_read.bytes,
                payload_read.denied,
                payload_read.not_found,
                payload_read.invalid,
                payload_read.too_large,
                payload_read.truncated,
                payload_read.latency_total_ns,
                payload_read.latency_max_ns,
                printable_warnings(&item.warnings)
            )
        }
    }
}

fn printable_warnings(warnings: &[String]) -> String {
    if warnings.is_empty() {
        "none".to_string()
    } else {
        warnings.join(";")
    }
}
