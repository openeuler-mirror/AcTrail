use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use model_core::ids::TraceId;
use model_core::process::{ExitStatus, NamespaceIdentity, ProcessMembership};
use model_core::trace::{TraceLifecycleState, TraceRecord};
use plugin_system::{
    PluginRuntimeError, TraceActivityContext, TraceAnalysisAction, TraceAnalysisContext,
    TraceAnalysisFileChange, TraceCommandExecution, TraceFileState, TraceFileStateStatus,
    TraceLlmExchange,
};
use semantic_action::{
    SemanticAction, SemanticActionCompleteness, SemanticActionKind, SemanticActionLink,
    SemanticActionLinkRole, SemanticActionStatus, attr_keys,
};
use storage_core::StorageBackend;

pub(super) fn analysis_context(
    trace: &TraceRecord,
) -> Result<TraceAnalysisContext, PluginRuntimeError> {
    let terminal_at = match trace.lifecycle_state {
        TraceLifecycleState::Completed => trace.timings.completed_at,
        TraceLifecycleState::Exited => trace.timings.exited_at,
        TraceLifecycleState::Failed => trace.timings.failed_at,
        _ => None,
    }
    .ok_or_else(|| {
        PluginRuntimeError::new(
            "trace_analysis",
            format!("trace {} is not durably terminal", trace.trace_id),
        )
    })?;
    Ok(TraceAnalysisContext {
        terminal_at,
        working_directory: trace.root_working_directory.clone(),
    })
}

pub(super) fn activity_context(trace: &TraceRecord) -> TraceActivityContext {
    TraceActivityContext {
        root_container_id: trace.root_container_id.clone(),
        root_process_id: trace.root_process_identity.to_string(),
        display_name: trace.display_name.to_string(),
        profile_name: trace.profile_name.to_string(),
    }
}

pub(super) fn project_llm_exchanges(
    actions: Vec<SemanticAction>,
) -> Result<Vec<TraceLlmExchange>, PluginRuntimeError> {
    let by_id = actions
        .iter()
        .map(|action| (action.action_id.as_str(), action))
        .collect::<BTreeMap<_, _>>();
    let mut exchanges = Vec::new();
    for call in actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmCall)
    {
        let request_action_id = required_attribute(
            call,
            attr_keys::llm_call::REQUEST_ACTION_ID,
            "LLM call request action id",
        )?;
        let request = by_id.get(request_action_id).copied().ok_or_else(|| {
            PluginRuntimeError::new(
                "trace_activity",
                format!(
                    "LLM call {} references missing request action {request_action_id}",
                    call.action_id
                ),
            )
        })?;
        if request.kind != SemanticActionKind::LlmRequest {
            return Err(PluginRuntimeError::new(
                "trace_activity",
                format!(
                    "LLM call {} request {} has kind {}",
                    call.action_id,
                    request.action_id,
                    request.kind.as_str()
                ),
            ));
        }
        let response_action_id = call
            .attributes
            .get(attr_keys::llm_call::RESPONSE_ACTION_ID)
            .cloned();
        let response = response_action_id
            .as_deref()
            .map(|action_id| {
                by_id.get(action_id).copied().ok_or_else(|| {
                    PluginRuntimeError::new(
                        "trace_activity",
                        format!(
                            "LLM call {} references missing response action {action_id}",
                            call.action_id
                        ),
                    )
                })
            })
            .transpose()?;
        if let Some(response) = response
            && response.kind != SemanticActionKind::LlmResponse
        {
            return Err(PluginRuntimeError::new(
                "trace_activity",
                format!(
                    "LLM call {} response {} has kind {}",
                    call.action_id,
                    response.action_id,
                    response.kind.as_str()
                ),
            ));
        }
        exchanges.push(TraceLlmExchange {
            call_action_id: call.action_id.clone(),
            request_action_id: request.action_id.clone(),
            response_action_id,
            process_id: call.process.to_string(),
            model: call
                .attributes
                .get(attr_keys::llm_call::MODEL)
                .or_else(|| request.attributes.get(attr_keys::llm_request::MODEL))
                .cloned(),
            server_address: request.attributes.get(attr_keys::server::ADDRESS).cloned(),
            url_path: request.attributes.get(attr_keys::url::PATH).cloned(),
            started_at: call.start_time,
            completed_at: call.end_time,
            request_body_bytes: required_u64_attribute(
                request,
                attr_keys::llm_request::PAYLOAD_BYTES,
            )?,
            request_raw_bytes: optional_u64_attribute(
                request,
                attr_keys::llm_request::RAW_PAYLOAD_BYTES,
            )?,
            request_complete: action_complete(request),
            response_body_bytes: response
                .map(|action| {
                    required_u64_attribute(action, attr_keys::llm_response::PAYLOAD_BYTES)
                })
                .transpose()?,
            response_raw_bytes: response
                .map(|action| {
                    optional_u64_attribute(action, attr_keys::llm_response::RAW_PAYLOAD_BYTES)
                })
                .transpose()?
                .flatten(),
            response_complete: response.is_some_and(action_complete),
        });
    }
    Ok(exchanges)
}

pub(super) fn project_command_executions(
    actions: Vec<SemanticAction>,
    links: &[SemanticActionLink],
    memberships: &[ProcessMembership],
) -> Result<Vec<TraceCommandExecution>, PluginRuntimeError> {
    let command_by_id = actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::CommandInvocation)
        .map(|action| (action.action_id.as_str(), action))
        .collect::<BTreeMap<_, _>>();
    let membership_by_process = memberships
        .iter()
        .map(|membership| (&membership.identity, membership))
        .collect::<BTreeMap<_, _>>();
    let mut commands_by_process = BTreeMap::<_, Vec<&SemanticAction>>::new();
    for command in command_by_id.values().copied() {
        commands_by_process
            .entry(&command.process)
            .or_default()
            .push(command);
    }
    let mut next_command_by_id = BTreeMap::<&str, &SemanticAction>::new();
    for commands in commands_by_process.values_mut() {
        commands.sort_by(|left, right| {
            (left.start_time, left.action_id.as_str())
                .cmp(&(right.start_time, right.action_id.as_str()))
        });
        for pair in commands.windows(2) {
            next_command_by_id.insert(pair[0].action_id.as_str(), pair[1]);
        }
    }
    let mut agent_by_child = BTreeMap::<&str, &str>::new();
    let mut command_parent_by_child = BTreeMap::<&str, &str>::new();
    for link in links.iter().filter(|link| valid_link(link)) {
        match link.role {
            SemanticActionLinkRole::AgentPerformedAction => {
                agent_by_child
                    .entry(link.child_action_id.as_str())
                    .or_insert(link.parent_action_id.as_str());
            }
            SemanticActionLinkRole::CommandContainsCommandInvocation => {
                command_parent_by_child
                    .entry(link.child_action_id.as_str())
                    .or_insert(link.parent_action_id.as_str());
            }
            _ => {}
        }
    }

    let mut commands = command_by_id.values().copied().collect::<Vec<_>>();
    commands.sort_by(|left, right| {
        (left.start_time, left.action_id.as_str())
            .cmp(&(right.start_time, right.action_id.as_str()))
    });
    commands
        .into_iter()
        .map(|action| {
            let parent_command_action_id = command_parent_by_child
                .get(action.action_id.as_str())
                .copied()
                .map(str::to_string);
            let agent_action_id = agent_by_child
                .get(action.action_id.as_str())
                .copied()
                .map(str::to_string);
            let top_level_agent_child = agent_action_id.is_some();
            let (ended_at, status, exit_code) = if let Some(next) =
                next_command_by_id.get(action.action_id.as_str())
            {
                (
                    next.end_time.or(Some(next.start_time)),
                    SemanticActionStatus::Success,
                    None,
                )
            } else {
                command_terminal_state(action, membership_by_process.get(&action.process).copied())
            };
            Ok(TraceCommandExecution {
                action_id: action.action_id.clone(),
                process_id: action.process.to_string(),
                executable: action
                    .attributes
                    .get(attr_keys::process::EXECUTABLE)
                    .cloned(),
                command_line: action.attributes.get(attr_keys::command::LINE).cloned(),
                started_at: action.start_time,
                ended_at,
                status,
                exit_code,
                agent_action_id,
                parent_command_action_id,
                top_level_agent_child,
            })
        })
        .collect()
}

fn command_terminal_state(
    action: &SemanticAction,
    membership: Option<&ProcessMembership>,
) -> (Option<SystemTime>, SemanticActionStatus, Option<i32>) {
    let Some(exit) = membership
        .and_then(|membership| membership.exit_status.as_ref())
        .filter(|exit| exit.observed_at >= action.start_time)
    else {
        return (None, SemanticActionStatus::Unknown, None);
    };
    (Some(exit.observed_at), command_exit_status(exit), exit.code)
}

fn command_exit_status(exit: &ExitStatus) -> SemanticActionStatus {
    match exit.code {
        Some(0) => SemanticActionStatus::Success,
        Some(_) => SemanticActionStatus::Error,
        None => SemanticActionStatus::Unknown,
    }
}

fn action_complete(action: &SemanticAction) -> bool {
    action.completeness == SemanticActionCompleteness::Complete
        && action.status != SemanticActionStatus::InProgress
}

fn required_attribute<'a>(
    action: &'a SemanticAction,
    key: &str,
    label: &str,
) -> Result<&'a str, PluginRuntimeError> {
    action
        .attributes
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| {
            PluginRuntimeError::new(
                "trace_activity",
                format!("{label} is missing from action {}", action.action_id),
            )
        })
}

fn required_u64_attribute(action: &SemanticAction, key: &str) -> Result<u64, PluginRuntimeError> {
    let raw = required_attribute(action, key, key)?;
    raw.parse::<u64>().map_err(|error| {
        PluginRuntimeError::new(
            "trace_activity",
            format!(
                "action {} attribute {key} is not u64: {error}",
                action.action_id
            ),
        )
    })
}

fn optional_u64_attribute(
    action: &SemanticAction,
    key: &str,
) -> Result<Option<u64>, PluginRuntimeError> {
    action
        .attributes
        .get(key)
        .map(|raw| {
            raw.parse::<u64>().map_err(|error| {
                PluginRuntimeError::new(
                    "trace_activity",
                    format!(
                        "action {} attribute {key} is not u64: {error}",
                        action.action_id
                    ),
                )
            })
        })
        .transpose()
}

fn valid_link(link: &SemanticActionLink) -> bool {
    link.valid
        && !link
            .attributes
            .get(attr_keys::actrail::LINK_VALID)
            .is_some_and(|value| value == "false")
}

pub(super) fn project_analysis_action(
    action: SemanticAction,
    paths: Vec<semantic_action::FileObservationPath>,
) -> TraceAnalysisAction {
    let path_complete = paths.len() == 1;
    let path = path_complete.then(|| paths[0].path.clone());
    let file_change = matches!(
        action.kind,
        SemanticActionKind::FileModify | SemanticActionKind::FileWrite
    )
    .then(|| TraceAnalysisFileChange {
        operation: action
            .attributes
            .get(attr_keys::file::OPERATION)
            .cloned()
            .unwrap_or_else(|| match action.kind {
                SemanticActionKind::FileWrite => "write".to_string(),
                _ => action.kind.as_str().to_string(),
            }),
        change_kind: action.file_change_kind().unwrap_or(match action.kind {
            SemanticActionKind::FileWrite => semantic_action::FileChangeKind::Modified,
            _ => semantic_action::FileChangeKind::Unknown,
        }),
        successful: action.status == SemanticActionStatus::Success,
        path: path.or_else(|| action.attributes.get(attr_keys::file::PATH).cloned()),
        path_complete: path_complete || action.attributes.contains_key(attr_keys::file::PATH),
    });
    TraceAnalysisAction {
        action_id: action.action_id,
        kind: action.kind.as_str().to_string(),
        status: action.status,
        completeness: action.completeness,
        file_change,
    }
}

pub(super) fn observed_host_path(
    storage: &dyn StorageBackend,
    trace: &TraceRecord,
    action: &SemanticAction,
    observed_path: &str,
) -> Result<Option<PathBuf>, PluginRuntimeError> {
    let observed_path = PathBuf::from(observed_path);
    let absolute = if observed_path.is_absolute() {
        observed_path
    } else {
        let Some(working_directory) = trace.root_working_directory.as_deref() else {
            return Ok(None);
        };
        PathBuf::from(working_directory).join(observed_path)
    };
    let process = storage
        .get_process_record(action.process.clone())
        .map_err(storage_runtime_error)?
        .ok_or_else(|| {
            PluginRuntimeError::new(
                "trace_file_state",
                format!("process {} is missing from storage", action.process),
            )
        })?;
    let Some(namespace) = process.namespaces.iter().next() else {
        return Ok(None);
    };
    let host_pid_namespace = current_pid_namespace()?;
    if !requires_namespaced_root(&namespace.pid_namespace, &host_pid_namespace) {
        return Ok(Some(absolute));
    }
    Ok(ebpf_collector::procfs::resolve_host_path(
        &namespace.pid_namespace,
        &absolute.to_string_lossy(),
    ))
}

fn current_pid_namespace() -> Result<NamespaceIdentity, PluginRuntimeError> {
    std::fs::read_link("/proc/self/ns/pid")
        .map(|namespace| NamespaceIdentity::new(namespace.display().to_string()))
        .map_err(|error| {
            PluginRuntimeError::new(
                "trace_file_state",
                format!("read daemon PID namespace: {error}"),
            )
        })
}

fn requires_namespaced_root(
    process_pid_namespace: &NamespaceIdentity,
    host_pid_namespace: &NamespaceIdentity,
) -> bool {
    process_pid_namespace != host_pid_namespace
}

pub(super) fn read_file_state(path: &Path) -> TraceFileState {
    let checked_at = SystemTime::now();
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => TraceFileState {
            status: TraceFileStateStatus::Exists,
            checked_at,
            file_kind: Some(
                if metadata.file_type().is_file() {
                    "file"
                } else if metadata.file_type().is_dir() {
                    "directory"
                } else if metadata.file_type().is_symlink() {
                    "symlink"
                } else {
                    "other"
                }
                .to_string(),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => TraceFileState {
            status: TraceFileStateStatus::NotFound,
            checked_at,
            file_kind: None,
        },
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => TraceFileState {
            status: TraceFileStateStatus::Inaccessible,
            checked_at,
            file_kind: None,
        },
        Err(_) => TraceFileState {
            status: TraceFileStateStatus::Unavailable,
            checked_at,
            file_kind: None,
        },
    }
}

pub(super) fn trace_missing(trace_id: TraceId) -> PluginRuntimeError {
    PluginRuntimeError::new(
        "trace_analysis",
        format!("trace {trace_id} was not found in durable storage"),
    )
}

pub(super) fn storage_runtime_error(error: storage_core::StorageError) -> PluginRuntimeError {
    PluginRuntimeError::new(error.stage, error.message)
}

#[cfg(test)]
mod tests {
    use model_core::process::NamespaceIdentity;

    use super::requires_namespaced_root;

    #[test]
    fn path_view_uses_pid_namespace_not_runtime_attribution() {
        let host = NamespaceIdentity::new("pid:[1]");

        assert!(!requires_namespaced_root(
            &NamespaceIdentity::new("pid:[1]"),
            &host,
        ));
        assert!(requires_namespaced_root(
            &NamespaceIdentity::new("pid:[2]"),
            &host,
        ));
    }
}
