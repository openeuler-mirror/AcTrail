use super::partition::{sweep_command_segments, sweep_segments};
use super::summary::{
    breakdown_shares, category_shares, command_breakdown_shares, round_attributions,
    trace_bottlenecks,
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
    let scope = match clip_window {
        Some(window) => full_scope.intersect(window).unwrap_or_else(|| {
            let boundary = full_scope.start.max(window.start);
            Interval {
                start: boundary,
                end: boundary,
            }
        }),
        None => full_scope,
    };

    let calls = model_intervals(actions, scope, provisional, &mut tracker);
    let (mut agent_intervals, agent_processes) =
        agent_intervals(actions, memberships, &calls, scope, &mut tracker);
    if degraded {
        agent_intervals.clear();
    }
    let tools = tool_intervals(
        actions,
        links,
        memberships,
        &agent_processes,
        scope,
        provisional,
        &mut tracker,
    );
    let command_intervals = command_intervals(actions, memberships, &tools, scope, provisional);
    let mut segments = sweep_segments(scope, &agent_intervals, &calls, &tools);
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.id = format!("segment-{}", index + 1);
    }
    let mut command_segments = sweep_command_segments(&segments, &command_intervals);
    for (index, segment) in command_segments.iter_mut().enumerate() {
        segment.id = format!("command-segment-{}", index + 1);
    }
    let categories = category_shares(&segments, scope.duration());
    let models = breakdown_shares(&segments, scope.duration(), Category::ModelSide, "model");
    let tools_breakdown =
        breakdown_shares(&segments, scope.duration(), Category::AgentSide, "agent");
    let commands = command_breakdown_shares(&command_segments, scope.duration());
    let rounds = round_attributions(scope, &calls, &segments, provisional);
    let bottlenecks = trace_bottlenecks(&calls, &command_intervals, &segments, provisional);
    let status = tracker.status(provisional);
    let coverage = TraceCoverage {
        llm_call_count: calls.len(),
        agent_process_count: agent_processes.len(),
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
        scope: AttributionScope {
            start_unix_nanos: nanos_string(scope.start),
            end_unix_nanos: nanos_string(scope.end),
            duration_nanos: nanos_string(scope.duration()),
            provisional,
            semantics: "exclusive_wall_clock",
        },
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
    let mut intervals = Vec::new();
    for action in actions
        .iter()
        .filter(|action| action.kind == SemanticActionKind::LlmCall)
    {
        let Ok(start) = system_time_nanos(action.start_time) else {
            tracker.action_error(
                "llm_call_clock_invalid",
                "LLM call start time cannot be represented.",
                &action.action_id,
            );
            continue;
        };
        let unfinished = action.end_time.is_none();
        let end = match action.end_time {
            Some(end) => match system_time_nanos(end) {
                Ok(end) => end,
                Err(_) => {
                    tracker.action_error(
                        "llm_call_clock_invalid",
                        "LLM call end time cannot be represented.",
                        &action.action_id,
                    );
                    continue;
                }
            },
            None => {
                if provisional {
                    tracker.action_info(
                        "llm_call_in_progress",
                        "LLM call is unfinished; the current observation watermark is used.",
                        &action.action_id,
                        Interval::new(start.min(scope.end), scope.end),
                    );
                } else {
                    tracker.action_warning(
                        "llm_call_end_missing",
                        "Terminal trace contains an LLM call without an end time.",
                        &action.action_id,
                        Interval::new(start.min(scope.end), scope.end),
                    );
                }
                scope.end
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
        let Some(interval) = Interval::new(start, end).and_then(|value| value.intersect(scope))
        else {
            continue;
        };
        let finalized_on_trace_close = action
            .attributes
            .get(attr_keys::actrail::ACTION_FINALIZED_ON_TRACE_CLOSE)
            .is_some_and(|value| value == "true");
        let partial_observation = action.completeness != SemanticActionCompleteness::Complete
            || action.status == SemanticActionStatus::InProgress;
        if finalized_on_trace_close {
            tracker.action_warning(
                "llm_call_closed_on_trace_end",
                "LLM call was finalized at trace close; its end is a capture boundary.",
                &action.action_id,
                Some(interval),
            );
        } else if !(provisional && unfinished) && partial_observation {
            tracker.action_warning(
                "llm_call_partial",
                "LLM call is incomplete; observable boundaries may be partial.",
                &action.action_id,
                Some(interval),
            );
        }
        intervals.push(ModelInterval {
            interval,
            action_id: action.action_id.clone(),
            model: non_empty_attr(action, attr_keys::llm_call::MODEL),
            status: if provisional && unfinished {
                "in_progress"
            } else if unfinished || finalized_on_trace_close || partial_observation {
                "partial"
            } else if action.status == SemanticActionStatus::Error {
                "error"
            } else {
                "complete"
            },
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

fn agent_intervals(
    actions: &[SemanticAction],
    memberships: &[ProcessMembership],
    calls: &[ModelInterval],
    scope: Interval,
    tracker: &mut StatusTracker,
) -> (Vec<Interval>, BTreeSet<ProcessIdentity>) {
    let mut evidence_start = BTreeMap::<ProcessIdentity, u128>::new();
    for action in actions.iter().filter(|action| {
        matches!(
            action.kind,
            SemanticActionKind::AgentIdentity | SemanticActionKind::LlmCall
        )
    }) {
        if let Ok(start) = system_time_nanos(action.start_time) {
            evidence_start
                .entry(action.process.clone())
                .and_modify(|current| *current = (*current).min(start))
                .or_insert(start);
        }
    }
    if evidence_start.is_empty() && !calls.is_empty() {
        tracker.warning(
            "agent_identity_missing",
            "Model calls exist but no reliable Agent process identity was found.",
        );
    }

    let mut agent_processes = evidence_start.keys().cloned().collect::<BTreeSet<_>>();
    loop {
        let before = agent_processes.len();
        for membership in memberships {
            if membership
                .inherited_from
                .as_ref()
                .is_some_and(|parent| agent_processes.contains(parent))
            {
                agent_processes.insert(membership.identity.clone());
            }
        }
        if before == agent_processes.len() {
            break;
        }
    }

    let membership_by_process = memberships
        .iter()
        .map(|membership| (membership.identity.clone(), membership))
        .collect::<BTreeMap<_, _>>();
    let mut intervals = Vec::new();
    for process in &agent_processes {
        let Some(membership) = membership_by_process.get(process).copied() else {
            tracker.warning(
                "agent_membership_missing",
                format!(
                    "Agent process {process} has no membership boundary; its local time is left unattributed."
                ),
            );
            continue;
        };
        if membership.state == MembershipState::IdentityStale {
            tracker.warning(
                "agent_identity_stale",
                format!("Agent subtree process {process} has stale identity coordinates."),
            );
        }
        let start = membership
            .observed_at
            .and_then(|time| system_time_nanos(time).ok())
            .or_else(|| {
                tracker.warning(
                    "agent_start_missing",
                    format!(
                        "Agent subtree process {process} has no observed start; identity evidence is used."
                    ),
                );
                evidence_start.get(process).copied()
            })
            .unwrap_or(scope.start);
        let end = membership
            .exit_status
            .as_ref()
            .and_then(|exit| system_time_nanos(exit.observed_at).ok())
            .unwrap_or(scope.end);
        if end < start {
            tracker.warning(
                "agent_clock_reversed",
                format!(
                    "Agent subtree process {process} ends before it starts; its interval is unattributed."
                ),
            );
            continue;
        }
        if let Some(interval) = Interval::new(start, end).and_then(|value| value.intersect(scope)) {
            intervals.push(interval);
        }
    }
    if agent_processes.is_empty() {
        tracker.warning(
            "agent_identity_missing",
            "No confirmed Agent process was observed; non-model time is left unattributed.",
        );
    }
    (intervals, agent_processes)
}

fn tool_intervals(
    actions: &[SemanticAction],
    links: &[SemanticActionLink],
    memberships: &[ProcessMembership],
    agent_processes: &BTreeSet<ProcessIdentity>,
    scope: Interval,
    provisional: bool,
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
        if agent_processes.contains(&command.process) && (explicitly_linked || named_tool) {
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
            let end = next_start
                .or(membership_end)
                .or_else(|| provisional.then_some(scope.end));
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
            if provisional && next_start.is_none() && membership_end.is_none() {
                tracker.info(
                    "tool_in_progress",
                    format!(
                        "Command {} is still running at the observation watermark.",
                        action.action_id
                    ),
                );
            }
            output.push(ToolInterval {
                interval,
                action_id: action.action_id.clone(),
                tool_name: non_empty_attr(action, attr_keys::command::TOOL_NAME)
                    .or_else(|| inferred_tool_names.get(&action.action_id).cloned()),
                process,
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
    scope: Interval,
    provisional: bool,
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
        let Some(end) = observed_end.or_else(|| provisional.then_some(scope.end)) else {
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
            command_key_label(action).is_some_and(|(key, _)| !is_shell_command(&key))
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
        let Some(end) = observed_end.or_else(|| provisional.then_some(scope.end)) else {
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
