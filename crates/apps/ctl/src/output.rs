//! User-facing output projection for control responses.

use control_contract::reply::{ControlReply, LaunchTlsPlanStatus};

pub fn format_reply(reply: &ControlReply) -> String {
    match reply {
        ControlReply::LaunchPermissions(reply) => format!(
            "launch permissions profile={} host_ebpf={} seccomp_notify={} degraded={}",
            reply.selected_profile_name,
            reply.selected_host_ebpf,
            reply.selected_seccomp_notify,
            reply.degraded
        ),
        ControlReply::LaunchTlsPlan(reply) => match &reply.status {
            LaunchTlsPlanStatus::Found(plan) => format!(
                "tls plan cache={} daemon_elapsed_us={} target={} binary={} provider={} source={} point_count={}",
                if reply.cache_hit { "hit" } else { "miss" },
                reply.resolve_elapsed_micros,
                plan.target.display(),
                plan.binary.display(),
                plan.provider,
                plan.source,
                point_count(&plan.points)
            ),
            LaunchTlsPlanStatus::Unsupported { reason } => format!(
                "tls plan cache={} daemon_elapsed_us={} unsupported={}",
                if reply.cache_hit { "hit" } else { "miss" },
                reply.resolve_elapsed_micros,
                reason
            ),
        },
        ControlReply::TrackAdded(reply) => {
            format!("trace {} entered {}", reply.trace_id, reply.lifecycle_state)
        }
        ControlReply::SeccompListenerRegistered => "seccomp listener registered".to_string(),
        ControlReply::TrackRemoved => "root capture removed".to_string(),
        ControlReply::TraceList(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{} {} pid={} {}/{:?}",
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
        ControlReply::PluginCommand(reply) => {
            let mut output = String::new();
            output.push_str(&reply.stdout);
            output.push_str(&reply.stderr);
            output
        }
    }
}

fn point_count(points: &str) -> usize {
    if points.is_empty() {
        0
    } else {
        points.split(';').count()
    }
}

fn printable_warnings(warnings: &[String]) -> String {
    if warnings.is_empty() {
        "none".to_string()
    } else {
        warnings.join(";")
    }
}
