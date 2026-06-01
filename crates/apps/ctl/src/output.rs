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
    }
}
