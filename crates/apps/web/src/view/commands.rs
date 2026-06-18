//! Command semantic action JSON rendering.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use storage_core::StorageBackend;

use super::actions;
use crate::json;

const COMMAND_ACTION_KINDS: &[&str] = &["command.invocation", "process.exec"];

pub(super) fn commands_json(
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let actions = storage
        .semantic_actions_matching_kinds(trace_id, COMMAND_ACTION_KINDS)
        .map_err(|error| {
            format!(
                "list command actions failed: {}: {}",
                error.stage, error.message
            )
        })?;
    let action_ids = actions
        .iter()
        .map(|action| action.action_id.as_str())
        .collect::<BTreeSet<_>>();
    let links = storage
        .list_semantic_action_links(trace_id)
        .map_err(|error| {
            format!(
                "list command links failed: {}: {}",
                error.stage, error.message
            )
        })?;
    let link_rows = links
        .iter()
        .filter(|link| {
            link.valid
                && action_ids.contains(link.parent_action_id.as_str())
                && action_ids.contains(link.child_action_id.as_str())
        })
        .map(|link| {
            let mut output = String::from("{");
            json::field(&mut output, "parent", &json::string(&link.parent_action_id));
            output.push(',');
            json::field(&mut output, "child", &json::string(&link.child_action_id));
            output.push('}');
            output
        })
        .collect::<Vec<_>>();
    let rows = actions
        .iter()
        .map(actions::action_json_lite)
        .collect::<Vec<_>>();

    let mut output = String::from("{");
    json::field(&mut output, "actions", &format!("[{}]", rows.join(",")));
    output.push(',');
    json::field(&mut output, "links", &format!("[{}]", link_rows.join(",")));
    output.push('}');
    Ok(output)
}
