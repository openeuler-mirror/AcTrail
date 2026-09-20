//! Command semantic action JSON rendering.

use std::collections::BTreeSet;

use model_core::ids::TraceId;
use model_core::process::ProcessIdentity;
use semantic_action::SemanticAction;
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
    let processes = CommandProcessDisplay::load(storage, &actions);
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
    output.push(',');
    json::field(&mut output, "processes", &processes.json());
    output.push('}');
    Ok(output)
}

struct CommandProcessDisplay {
    host_pids: Vec<(ProcessIdentity, u32)>,
}

impl CommandProcessDisplay {
    fn load(storage: &mut dyn StorageBackend, actions: &[SemanticAction]) -> Self {
        let identities = actions
            .iter()
            .map(|action| action.process)
            .collect::<BTreeSet<_>>();
        let mut host_pids = Vec::with_capacity(identities.len());
        for identity in identities {
            let record = match storage.get_process_record(identity) {
                Ok(record) => record,
                Err(error) => {
                    eprintln!(
                        "actrailweb: host PID unavailable for process {}: {}: {}",
                        identity.get(),
                        error.stage,
                        error.message,
                    );
                    continue;
                }
            };
            if let Some(host) = record.and_then(|record| record.host) {
                host_pids.push((identity, host.pid));
            }
        }
        Self { host_pids }
    }

    fn json(&self) -> String {
        let rows = self
            .host_pids
            .iter()
            .map(|(identity, pid)| {
                format!(
                    "{{\"process_id\":{},\"pid\":{}}}",
                    json::number(identity.get()),
                    json::number(*pid),
                )
            })
            .collect::<Vec<_>>();
        format!("[{}]", rows.join(","))
    }
}
