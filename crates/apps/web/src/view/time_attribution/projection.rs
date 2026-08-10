use super::agents::{AgentProcessScope, agent_process_scope, include_nested_agent_calls};
use super::partition::{sweep_command_segments, sweep_segments};
use super::summary::{
    breakdown_shares, category_shares, command_breakdown_shares, round_attributions,
    trace_bottlenecks,
};
use super::turns::{
    attribution_scope, clip_command_intervals, clip_intervals, clip_model_intervals,
    clip_tool_intervals, clip_user_turns, merge_intervals, user_turns,
};
use super::*;

pub(super) fn project_trace_data(
    trace: &TraceRecord,
    actions: &[SemanticAction],
    links: &[SemanticActionLink],
    memberships: &[ProcessMembership],
    clip_window: Option<Interval>,
) -> Result<TraceAttribution, String> {
    let mut tracker = StatusTracker::default();
    let degraded = trace.health == TraceHealth::Degraded;
    if degraded {
        tracker.warning(
            "trace_degraded",
            "Trace health is degraded; uncertain time remains unattributed.",
        );
    }
    let provisional = !trace.lifecycle_state.is_terminal();
    let full_scope = trace_scope(trace, actions, memberships, &mut tracker)?;
    let observed_calls = model_intervals(actions, full_scope, provisional, &mut tracker);
    let strong_turn_keys = observed_calls
        .iter()
        .filter(|call| call.user_input_start.is_some())
        .map(|call| call.turn_key.clone())
        .collect::<BTreeSet<_>>();
    let candidate_calls = if strong_turn_keys.is_empty() {
        if !observed_calls.is_empty() {
            tracker.info(
                "user_input_boundary_unobserved",
                "User-message evidence was observed, but terminal input timing was unavailable; each turn begins at its first model request.",
            );
        }
        observed_calls.clone()
    } else {
        observed_calls
            .iter()
            .filter(|call| strong_turn_keys.contains(&call.turn_key))
            .cloned()
            .collect()
    };
    let candidate_call_ids = candidate_calls
        .iter()
        .map(|call| call.action_id.clone())
        .collect::<BTreeSet<_>>();
    let agent_scope = agent_process_scope(memberships, &observed_calls, &mut tracker);
    let raw_tools = tool_intervals(
        actions,
        links,
        memberships,
        &agent_scope,
        full_scope,
        provisional,
        &mut tracker,
    );
    let raw_local_tools = raw_tools
        .iter()
        .filter(|tool| !tool.agent_invocation)
        .cloned()
        .collect::<Vec<_>>();
    let (direct_calls, mut full_turns) = user_turns(candidate_calls, &raw_tools, &mut tracker);
    let calls = include_nested_agent_calls(
        &observed_calls,
        direct_calls,
        &mut full_turns,
        &raw_tools,
        &agent_scope,
        &mut tracker,
    );
    let attributed_call_ids = calls
        .iter()
        .map(|call| call.action_id.clone())
        .collect::<BTreeSet<_>>();
    for call in &observed_calls {
        if !candidate_call_ids.contains(&call.action_id)
            && !attributed_call_ids.contains(&call.action_id)
        {
            tracker.action_info(
                "background_llm_call_excluded",
                "LLM call is not linked to an observed user input or nested Agent invocation and is excluded from user-turn attribution.",
                &call.action_id,
                Some(call.interval),
            );
        }
    }
    let full_windows = merge_intervals(full_turns.iter().map(|turn| turn.interval));
    let windows = clip_intervals(&full_windows, clip_window);
    let calls = clip_model_intervals(&calls, &windows);
    let turns = clip_user_turns(&full_turns, &calls, clip_window);
    if full_windows.is_empty() {
        tracker.info(
            "user_turn_not_observed",
            "No completed model exchange with reliable user-message evidence was observed; startup, idle, and background activity are excluded.",
        );
    }
    let tools = clip_tool_intervals(&raw_local_tools, &windows);
    let mut agent_intervals = tools.iter().map(|tool| tool.interval).collect::<Vec<_>>();
    if degraded {
        agent_intervals.clear();
    }
    let raw_command_intervals = command_intervals(
        actions,
        memberships,
        &raw_local_tools,
        &agent_scope,
        full_scope,
        provisional,
    );
    let command_intervals = clip_command_intervals(&raw_command_intervals, &windows);
    let mut segments = windows
        .iter()
        .flat_map(|window| sweep_segments(*window, &agent_intervals, &calls, &tools))
        .collect::<Vec<_>>();
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.id = format!("segment-{}", index + 1);
    }
    let mut command_segments = sweep_command_segments(&segments, &command_intervals);
    for (index, segment) in command_segments.iter_mut().enumerate() {
        segment.id = format!("command-segment-{}", index + 1);
    }
    let total_duration = windows.iter().map(|window| window.duration()).sum::<u128>();
    let categories = category_shares(&segments, total_duration);
    let models = breakdown_shares(&segments, total_duration, Category::ModelSide, "model");
    let tools_breakdown = breakdown_shares(&segments, total_duration, Category::AgentSide, "agent");
    let commands = command_breakdown_shares(&command_segments, total_duration);
    let rounds = round_attributions(&turns, &calls, &segments, provisional);
    let bottlenecks = trace_bottlenecks(&calls, &command_intervals, &segments, provisional);
    let status = tracker.status(provisional);
    let llm_request_count = actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmRequest)
        .count();
    let observed_llm_call_count = actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmCall)
        .count();
    let coverage = TraceCoverage {
        llm_request_count,
        observed_llm_call_count,
        llm_call_count: calls.len(),
        excluded_llm_call_count: observed_llm_call_count.saturating_sub(calls.len()),
        user_turn_count: turns.len(),
        strong_user_input_count: calls
            .iter()
            .filter(|call| call.user_input_start.is_some())
            .count(),
        agent_process_count: agent_scope.subjects.len(),
        tool_interval_count: tools.len(),
        command_interval_count: command_intervals.len(),
        segment_count: segments.len(),
        command_segment_count: command_segments.len(),
    };
    Ok(TraceAttribution {
        schema_version: SCHEMA_VERSION,
        trace: TraceReference {
            id: trace.trace_id.get(),
            name: trace.display_name.as_str().to_string(),
            state: trace.lifecycle_state.as_storage_str().to_string(),
        },
        scope: attribution_scope(&windows, full_scope.start, provisional),
        status,
        categories,
        rounds,
        models,
        tools: tools_breakdown,
        commands,
        bottlenecks,
        segments,
        command_segments,
        coverage,
        issues: tracker.issues,
    })
}

fn trace_scope(
    trace: &TraceRecord,
    actions: &[SemanticAction],
    memberships: &[ProcessMembership],
    tracker: &mut StatusTracker,
) -> Result<Interval, String> {
    let start_time = match trace.timings.started_at {
        Some(started_at) => started_at,
        None => {
            tracker.warning(
                "trace_start_missing",
                "Trace started_at is missing; created_at is used as the scope start.",
            );
            trace.timings.created_at
        }
    };
    let start = system_time_nanos(start_time)
        .map_err(|error| format!("invalid trace {} start time: {error}", trace.trace_id))?;
    let end_time = if trace.lifecycle_state.is_terminal() {
        match terminal_time(trace) {
            Some(time) => time,
            None => {
                tracker.invalid(
                    "trace_terminal_time_missing",
                    format!(
                        "Trace state {} has no matching terminal timestamp.",
                        trace.lifecycle_state.as_storage_str()
                    ),
                );
                latest_observation(start_time, actions, memberships)
            }
        }
    } else {
        latest_observation(start_time, actions, memberships)
    };
    let end = system_time_nanos(end_time)
        .map_err(|error| format!("invalid trace {} end time: {error}", trace.trace_id))?;
    if end < start {
        tracker.invalid(
            "trace_clock_reversed",
            "Trace end precedes trace start; a zero-length scope is returned.",
        );
        return Ok(Interval { start, end: start });
    }
    Ok(Interval { start, end })
}

pub(super) fn terminal_time(trace: &TraceRecord) -> Option<SystemTime> {
    match trace.lifecycle_state {
        TraceLifecycleState::Completed => trace.timings.completed_at,
        TraceLifecycleState::Exited => trace.timings.exited_at,
        TraceLifecycleState::Failed => trace.timings.failed_at,
        _ => None,
    }
}

fn latest_observation(
    fallback: SystemTime,
    actions: &[SemanticAction],
    memberships: &[ProcessMembership],
) -> SystemTime {
    let action_times = actions
        .iter()
        .flat_map(|action| [Some(action.start_time), action.end_time])
        .flatten();
    let membership_times = memberships.iter().flat_map(|membership| {
        [
            membership.observed_at,
            membership.exit_status.as_ref().map(|exit| exit.observed_at),
        ]
    });
    action_times
        .chain(membership_times.flatten())
        .fold(fallback, SystemTime::max)
}

fn model_intervals(
    actions: &[SemanticAction],
    scope: Interval,
    provisional: bool,
    tracker: &mut StatusTracker,
) -> Vec<ModelInterval> {
    let actions_by_id = actions
        .iter()
        .map(|action| (action.action_id.as_str(), action))
        .collect::<BTreeMap<_, _>>();
    let mut intervals = Vec::new();
    for action in actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmCall)
    {
        let request = action
            .attributes
            .get(attr_keys::llm_call::REQUEST_ACTION_ID)
            .and_then(|request_id| actions_by_id.get(request_id.as_str()).copied())
            .filter(|request| request.kind == SemanticActionKind::LlmRequest);
        let Some(request) = request else {
            tracker.action_warning(
                "llm_call_request_missing",
                "LLM call has no matching request action and is left unattributed.",
                &action.action_id,
                None,
            );
            continue;
        };
        if let Some(background_kind) = request
            .attributes
            .get(attr_keys::llm_request::BACKGROUND_KIND)
        {
            tracker.action_info(
                "background_llm_call_excluded",
                format!(
                    "LLM call is classified as background activity ({background_kind}) and is excluded from user-turn attribution."
                ),
                &action.action_id,
                None,
            );
            continue;
        }
        let response = action
            .attributes
            .get(attr_keys::llm_call::RESPONSE_ACTION_ID)
            .and_then(|response_id| actions_by_id.get(response_id.as_str()).copied())
            .filter(|response| response.kind == SemanticActionKind::LlmResponse);
        let model = resolved_call_model(action, request, response, tracker);
        let Some(response) = response else {
            report_unpaired_call(action, provisional, tracker);
            continue;
        };
        let Ok(start) = system_time_nanos(request.start_time) else {
            tracker.action_error(
                "llm_call_clock_invalid",
                "LLM request start time cannot be represented.",
                &action.action_id,
            );
            continue;
        };
        let end = match response.end_time {
            Some(end) => match system_time_nanos(end) {
                Ok(end) => end,
                Err(_) => {
                    tracker.action_error(
                        "llm_call_clock_invalid",
                        "LLM response end time cannot be represented.",
                        &action.action_id,
                    );
                    continue;
                }
            },
            None => {
                if provisional {
                    tracker.action_info(
                        "llm_response_pending",
                        "LLM response has no observable data boundary yet; pending time remains unattributed.",
                        &action.action_id,
                        None,
                    );
                } else {
                    tracker.action_warning(
                        "llm_response_end_missing",
                        "Terminal Trace contains an LLM response without an observable end; the call is left unattributed.",
                        &action.action_id,
                        None,
                    );
                }
                continue;
            }
        };
        if end < start {
            tracker.action_error(
                "llm_call_clock_reversed",
                "LLM call end precedes its start and is left unattributed.",
                &action.action_id,
            );
            continue;
        }
        let Some(turn_key) = user_turn_key(request) else {
            tracker.action_info(
                "llm_call_without_user_message",
                "LLM call has no retained user-message evidence and is excluded from user-turn attribution.",
                &action.action_id,
                Interval::new(start, end),
            );
            continue;
        };
        let user_input_start = request
            .attributes
            .get(attr_keys::agent_turn::USER_INPUT_OBSERVED_AT_UNIX_NANOS)
            .and_then(|value| value.parse::<u128>().ok())
            .filter(|input_start| *input_start <= start && *input_start >= scope.start);
        let finalized_on_trace_close =
            finalized_on_trace_close(action) || finalized_on_trace_close(response);
        if finalized_on_trace_close && end == scope.end {
            tracker.action_warning(
                "llm_call_trace_close_only",
                "LLM call has no response boundary distinct from Trace close and is left unattributed.",
                &action.action_id,
                Interval::new(start.min(scope.end), scope.end),
            );
            continue;
        }
        let Some(interval) = Interval::new(start, end).and_then(|value| value.intersect(scope))
        else {
            continue;
        };
        let partial_observation = response.completeness != SemanticActionCompleteness::Complete
            || response.status == SemanticActionStatus::InProgress;
        if finalized_on_trace_close {
            tracker.action_warning(
                "llm_call_closed_on_trace_end",
                "LLM call was finalized at Trace close; only its last observed response boundary is attributed.",
                &action.action_id,
                Some(interval),
            );
        } else if partial_observation {
            if provisional {
                tracker.action_info(
                    "llm_call_in_progress",
                    "LLM response is still open; attribution stops at the latest observed response data.",
                    &action.action_id,
                    Some(interval),
                );
            } else {
                tracker.action_warning(
                    "llm_call_partial",
                    "LLM call is incomplete; attribution stops at the last reliable response boundary.",
                    &action.action_id,
                    Some(interval),
                );
            }
        }
        intervals.push(ModelInterval {
            interval,
            action_id: action.action_id.clone(),
            model,
            process: request.process.clone(),
            status: if provisional && response.status == SemanticActionStatus::InProgress {
                "in_progress"
            } else if finalized_on_trace_close || partial_observation {
                "partial"
            } else if response.status == SemanticActionStatus::Error {
                "error"
            } else {
                "complete"
            },
            turn_key,
            user_input_start,
        });
    }
    intervals.sort_by(|left, right| {
        (left.interval.start, left.interval.end, &left.action_id).cmp(&(
            right.interval.start,
            right.interval.end,
            &right.action_id,
        ))
    });
    intervals
}

fn user_turn_key(request: &SemanticAction) -> Option<UserTurnKey> {
    let user_message_count = request
        .attributes
        .get(attr_keys::llm_request::USER_MESSAGE_COUNT)
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|count| *count > 0);
    let latest_user_message_hash = request
        .attributes
        .get(attr_keys::llm_request::LATEST_USER_MESSAGE_HASH)
        .filter(|value| valid_user_message_hash(value))
        .cloned();
    let (user_message_count, latest_user_message_hash) =
        match (user_message_count, latest_user_message_hash) {
            (Some(count), Some(hash)) => (count, hash),
            _ => {
                let preview = request
                    .attributes
                    .get(attr_keys::llm_request::MESSAGE_PREVIEW)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())?;
                (0, format!("legacy-preview:{preview}"))
            }
        };
    Some(UserTurnKey {
        process: request.process.clone(),
        user_message_count,
        latest_user_message_hash,
    })
}

fn valid_user_message_hash(value: &str) -> bool {
    value.len() == "sha256:".len() + 64
        && value.starts_with("sha256:")
        && value["sha256:".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn report_unpaired_call(action: &SemanticAction, provisional: bool, tracker: &mut StatusTracker) {
    if provisional && action.status == SemanticActionStatus::InProgress {
        tracker.action_info(
            "llm_call_unpaired",
            "LLM request has no matching response yet; it is excluded from model-side time until response evidence arrives.",
            &action.action_id,
            None,
        );
    } else {
        tracker.action_warning(
            "llm_call_unpaired",
            "LLM call has no matching response and is excluded from model-side time.",
            &action.action_id,
            None,
        );
    }
}

fn resolved_call_model(
    call: &SemanticAction,
    request: &SemanticAction,
    response: Option<&SemanticAction>,
    tracker: &mut StatusTracker,
) -> Option<String> {
    let request_model = validated_action_model(request, attr_keys::llm_request::MODEL);
    let response_model =
        response.and_then(|action| validated_action_model(action, attr_keys::llm_response::MODEL));
    let call_model = validated_action_model(call, attr_keys::llm_call::MODEL);
    let has_invalid_model = [
        action_model_is_invalid(request, attr_keys::llm_request::MODEL),
        response
            .is_some_and(|action| action_model_is_invalid(action, attr_keys::llm_response::MODEL)),
        action_model_is_invalid(call, attr_keys::llm_call::MODEL),
    ]
    .into_iter()
    .any(|invalid| invalid);
    if has_invalid_model {
        tracker.action_info(
            "llm_model_invalid",
            "A malformed model identifier was ignored; JSON fragments are not used as model keys.",
            &call.action_id,
            None,
        );
    }
    if let (Some(request_model), Some(response_model)) = (&request_model, &response_model)
        && request_model != response_model
    {
        tracker.action_info(
            "llm_model_conflict",
            "Request and response report different model identifiers; the response model is used.",
            &call.action_id,
            None,
        );
    }
    response_model.or(request_model).or(call_model)
}

fn validated_action_model(action: &SemanticAction, key: &str) -> Option<String> {
    action
        .attributes
        .get(key)
        .and_then(|value| validated_model_identifier(value))
        .map(ToOwned::to_owned)
}

fn action_model_is_invalid(action: &SemanticAction, key: &str) -> bool {
    action
        .attributes
        .get(key)
        .is_some_and(|value| validated_model_identifier(value).is_none())
}

fn finalized_on_trace_close(action: &SemanticAction) -> bool {
    action
        .attributes
        .get(attr_keys::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE)
        .is_some_and(|value| value == "true")
}

fn tool_intervals(
    actions: &[SemanticAction],
    links: &[SemanticActionLink],
    memberships: &[ProcessMembership],
    agent_scope: &AgentProcessScope,
    scope: Interval,
    _provisional: bool,
    tracker: &mut StatusTracker,
) -> Vec<ToolInterval> {
    let inferred_tool_names = infer_tool_names(actions, links);
    let commands = actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::CommandInvocation)
        .collect::<Vec<_>>();
    let command_ids = commands
        .iter()
        .map(|action| action.action_id.as_str())
        .collect::<BTreeSet<_>>();
    let linked_agent_commands = links
        .iter()
        .filter(|link| {
            valid_link(link)
                && link.role == SemanticActionLinkRole::AgentPerformedAction
                && command_ids.contains(link.child_action_id.as_str())
        })
        .map(|link| link.child_action_id.as_str())
        .collect::<BTreeSet<_>>();
    let membership_by_process = memberships
        .iter()
        .map(|membership| (membership.identity.clone(), membership))
        .collect::<BTreeMap<_, _>>();
    let mut by_process = BTreeMap::<ProcessIdentity, Vec<&SemanticAction>>::new();
    for command in commands {
        let explicitly_linked = linked_agent_commands.contains(command.action_id.as_str());
        let named_tool = non_empty_attr(command, attr_keys::command::TOOL_NAME).is_some()
            || inferred_tool_names.contains_key(&command.action_id);
        if agent_scope.subtree.contains(&command.process) && (explicitly_linked || named_tool) {
            by_process
                .entry(command.process.clone())
                .or_default()
                .push(command);
        }
    }

    let mut output = Vec::new();
    for (process, mut process_commands) in by_process {
        process_commands.sort_by(|left, right| {
            (left.start_time, left.action_id.as_str())
                .cmp(&(right.start_time, right.action_id.as_str()))
        });
        for (index, action) in process_commands.iter().enumerate() {
            let Ok(start) = system_time_nanos(action.start_time) else {
                tracker.action_error(
                    "tool_clock_invalid",
                    "Command start time cannot be represented.",
                    &action.action_id,
                );
                continue;
            };
            let next_start = process_commands
                .get(index + 1)
                .and_then(|next| next.end_time.or(Some(next.start_time)))
                .and_then(|time| system_time_nanos(time).ok());
            let membership_end = membership_by_process
                .get(&process)
                .and_then(|membership| membership.exit_status.as_ref())
                .and_then(|exit| system_time_nanos(exit.observed_at).ok());
            let end = next_start.or(membership_end);
            let Some(end) = end else {
                tracker.action_warning(
                    "tool_exit_missing",
                    "Command has no next exec or process exit; tool duration is not inferred.",
                    &action.action_id,
                    None,
                );
                continue;
            };
            if end < start {
                tracker.action_error(
                    "tool_clock_reversed",
                    "Command end precedes its start; tool duration is not attributed.",
                    &action.action_id,
                );
                continue;
            }
            let Some(interval) = Interval::new(start, end).and_then(|value| value.intersect(scope))
            else {
                continue;
            };
            let agent_invocation = agent_scope.is_agent_runtime_command(action);
            if agent_invocation {
                tracker.action_info(
                    "agent_runtime_command_excluded",
                    "Command execution was promoted to an Agent runtime or invocation wrapper; it is retained for causal turn linking but excluded from Agent-side command duration.",
                    &action.action_id,
                    Some(interval),
                );
            }
            output.push(ToolInterval {
                interval,
                action_id: action.action_id.clone(),
                tool_name: non_empty_attr(action, attr_keys::command::TOOL_NAME)
                    .or_else(|| inferred_tool_names.get(&action.action_id).cloned()),
                process,
                agent_invocation,
            });
        }
    }
    output.sort_by(|left, right| {
        (left.interval.start, left.interval.end, &left.action_id).cmp(&(
            right.interval.start,
            right.interval.end,
            &right.action_id,
        ))
    });
    output
}

fn command_intervals(
    actions: &[SemanticAction],
    memberships: &[ProcessMembership],
    tools: &[ToolInterval],
    agent_scope: &AgentProcessScope,
    scope: Interval,
    _provisional: bool,
) -> Vec<CommandInterval> {
    let mut commands_by_process = BTreeMap::<ProcessIdentity, Vec<&SemanticAction>>::new();
    for action in actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::CommandInvocation)
    {
        commands_by_process
            .entry(action.process)
            .or_default()
            .push(action);
    }
    for process_commands in commands_by_process.values_mut() {
        process_commands.sort_by(|left, right| {
            (left.start_time, left.action_id.as_str())
                .cmp(&(right.start_time, right.action_id.as_str()))
        });
    }

    let tool_processes = tools
        .iter()
        .map(|tool| tool.process)
        .collect::<BTreeSet<_>>();
    let memberships_by_process = memberships
        .iter()
        .map(|membership| (membership.identity, membership))
        .collect::<BTreeMap<_, _>>();
    let mut output = Vec::new();

    for membership in memberships {
        if agent_scope.is_agent_runtime_process(membership.identity) {
            continue;
        }
        let Some(parent) = membership
            .inherited_from
            .filter(|parent| tool_processes.contains(parent))
        else {
            continue;
        };
        let Some(process_commands) = commands_by_process.get(&membership.identity) else {
            continue;
        };
        let Some(first) = process_commands.first().copied() else {
            continue;
        };
        let Ok(start) = system_time_nanos(first.start_time) else {
            continue;
        };
        let observed_end = membership
            .exit_status
            .as_ref()
            .and_then(|exit| system_time_nanos(exit.observed_at).ok());
        let Some(end) = observed_end else {
            continue;
        };
        let Some(interval) = Interval::new(start, end).and_then(|value| value.intersect(scope))
        else {
            continue;
        };
        let Some((key, label)) = command_key_label(first) else {
            continue;
        };
        let (agent_tool_key, agent_tool_label) = tool_identity_at(tools, parent, interval.start);
        output.push(CommandInterval {
            interval,
            action_id: first.action_id.clone(),
            key,
            label,
            agent_tool_key,
            agent_tool_label,
            status: if observed_end.is_some() {
                "complete"
            } else {
                "in_progress"
            },
        });
    }

    // Some tool runners replace their own process with the requested executable
    // instead of forking it. Keep that executable visible as an actual command,
    // while ignoring shell setup execs such as /usr/bin/bash -> /bin/bash.
    for process in tool_processes {
        let Some(process_commands) = commands_by_process.get(&process) else {
            continue;
        };
        let Some(first_actual) = process_commands.iter().copied().find(|action| {
            !agent_scope.is_agent_runtime_command(action)
                && command_key_label(action).is_some_and(|(key, _)| !is_shell_command(&key))
        }) else {
            continue;
        };
        let Ok(start) = system_time_nanos(first_actual.start_time) else {
            continue;
        };
        let observed_end = memberships_by_process
            .get(&process)
            .and_then(|membership| membership.exit_status.as_ref())
            .and_then(|exit| system_time_nanos(exit.observed_at).ok());
        let Some(end) = observed_end else {
            continue;
        };
        let Some(interval) = Interval::new(start, end).and_then(|value| value.intersect(scope))
        else {
            continue;
        };
        let Some((key, label)) = command_key_label(first_actual) else {
            continue;
        };
        let (agent_tool_key, agent_tool_label) = tool_identity_at(tools, process, interval.start);
        output.push(CommandInterval {
            interval,
            action_id: first_actual.action_id.clone(),
            key,
            label,
            agent_tool_key,
            agent_tool_label,
            status: if observed_end.is_some() {
                "complete"
            } else {
                "in_progress"
            },
        });
    }

    output.sort_by(|left, right| {
        (
            left.interval.start,
            left.interval.end,
            &left.key,
            &left.action_id,
        )
            .cmp(&(
                right.interval.start,
                right.interval.end,
                &right.key,
                &right.action_id,
            ))
    });
    output
}

fn command_key_label(action: &SemanticAction) -> Option<(String, String)> {
    let executable = non_empty_attr(action, attr_keys::process::EXECUTABLE).or_else(|| {
        action
            .title
            .split_whitespace()
            .next()
            .map(ToOwned::to_owned)
    })?;
    let label = Path::new(&executable)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(executable.as_str())
        .to_string();
    Some((label.to_ascii_lowercase(), label))
}

fn is_shell_command(key: &str) -> bool {
    matches!(
        key,
        "bash" | "sh" | "dash" | "zsh" | "ksh" | "fish" | "csh" | "tcsh"
    )
}

fn tool_identity_at(
    tools: &[ToolInterval],
    process: ProcessIdentity,
    timestamp: u128,
) -> (String, String) {
    let candidate = tools
        .iter()
        .filter(|tool| tool.process == process)
        .filter(|tool| tool.interval.start <= timestamp)
        .max_by_key(|tool| tool.interval.start)
        .or_else(|| tools.iter().find(|tool| tool.process == process));
    let name = candidate.and_then(|tool| tool.tool_name.as_deref());
    match name {
        Some(name) => (name.to_string(), name.to_string()),
        None => (
            TOOL_UNKNOWN_KEY.to_string(),
            "Unidentified local command".to_string(),
        ),
    }
}

fn infer_tool_names(
    actions: &[SemanticAction],
    links: &[SemanticActionLink],
) -> BTreeMap<String, String> {
    let actions_by_id = actions
        .iter()
        .map(|action| (action.action_id.as_str(), action))
        .collect::<HashMap<_, _>>();
    let mut call_tool_names = HashMap::<&str, VecDeque<String>>::new();
    for call in actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmCall)
    {
        let response_id =
            non_empty_attr(call, attr_keys::llm_call::RESPONSE_ACTION_ID).or_else(|| {
                links
                    .iter()
                    .find(|link| {
                        valid_link(link)
                            && link.role == SemanticActionLinkRole::LlmCallResponse
                            && link.parent_action_id == call.action_id
                    })
                    .map(|link| link.child_action_id.clone())
            });
        let Some(response) = response_id
            .as_deref()
            .and_then(|action_id| actions_by_id.get(action_id))
        else {
            continue;
        };
        let names = response_tool_names(response);
        if !names.is_empty() {
            call_tool_names.insert(call.action_id.as_str(), names);
        }
    }

    let mut performed_by_agent = BTreeMap::<&str, Vec<(u64, &str)>>::new();
    for link in links.iter().filter(|link| {
        valid_link(link) && link.role == SemanticActionLinkRole::AgentPerformedAction
    }) {
        let Some(sequence) = link
            .attributes
            .get(attr_keys::agent::PERFORMED_ACTION_SEQUENCE)
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        performed_by_agent
            .entry(link.parent_action_id.as_str())
            .or_default()
            .push((sequence, link.child_action_id.as_str()));
    }

    let mut inferred = BTreeMap::new();
    for performed in performed_by_agent.values_mut() {
        performed.sort_unstable();
        let mut pending_names = VecDeque::new();
        let mut active_tool_by_process = BTreeMap::<ProcessIdentity, String>::new();
        for (_sequence, action_id) in performed.iter() {
            let Some(action) = actions_by_id.get(action_id) else {
                continue;
            };
            match action.kind {
                SemanticActionKind::LlmCall => {
                    pending_names = call_tool_names
                        .get(action.action_id.as_str())
                        .cloned()
                        .unwrap_or_default();
                    active_tool_by_process.clear();
                }
                SemanticActionKind::CommandInvocation => {
                    let direct_name = non_empty_attr(action, attr_keys::command::TOOL_NAME);
                    let pending_name = pending_names.pop_front();
                    let tool_name = direct_name
                        .clone()
                        .or(pending_name)
                        .or_else(|| active_tool_by_process.get(&action.process).cloned());
                    let Some(tool_name) = tool_name else {
                        continue;
                    };
                    active_tool_by_process.insert(action.process.clone(), tool_name.clone());
                    if direct_name.is_none() {
                        inferred.insert(action.action_id.clone(), tool_name);
                    }
                }
                _ => {}
            }
        }
    }
    inferred
}

fn response_tool_names(response: &SemanticAction) -> VecDeque<String> {
    let Some(raw) = response
        .attributes
        .get(attr_keys::llm_response::TOOL_CALLS_JSON)
    else {
        return VecDeque::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return VecDeque::new();
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool_call| {
            tool_call
                .get("function")
                .and_then(|function| function.get("name"))
                .or_else(|| tool_call.get("name"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect()
}
