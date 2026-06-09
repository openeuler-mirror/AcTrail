//! Command semantic action JSON rendering.

use model_core::ids::TraceId;
use sqlite_storage::SqliteStorage;

use super::actions;
use crate::json;

const COMMAND_ACTION_KINDS: &[&str] = &["command.invocation", "process.exec"];

pub(super) fn commands_json(
    storage: &mut SqliteStorage,
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
    let rows = actions.iter().map(actions::action_json).collect::<Vec<_>>();

    let mut output = String::from("{");
    json::field(&mut output, "actions", &format!("[{}]", rows.join(",")));
    output.push('}');
    Ok(output)
}
