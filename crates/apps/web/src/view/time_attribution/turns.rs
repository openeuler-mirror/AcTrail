use super::*;

pub(super) fn user_turns(
    calls: Vec<ModelInterval>,
    tools: &[ToolInterval],
    tracker: &mut StatusTracker,
) -> (Vec<ModelInterval>, Vec<UserTurn>) {
    let mut calls_by_key = BTreeMap::<UserTurnKey, Vec<ModelInterval>>::new();
    for call in calls {
        calls_by_key
            .entry(call.turn_key.clone())
            .or_default()
            .push(call);
    }

    let mut accepted_calls = Vec::new();
    let mut turns = Vec::new();
    for mut keyed_calls in calls_by_key.into_values() {
        keyed_calls.sort_by(|left, right| {
            (left.interval.start, left.interval.end, &left.action_id).cmp(&(
                right.interval.start,
                right.interval.end,
                &right.action_id,
            ))
        });
        let mut current_interval = None::<Interval>;
        let mut current_calls = Vec::<ModelInterval>::new();
        for call in keyed_calls {
            let call_start = call.user_input_start.unwrap_or(call.interval.start);
            let call_scope = Interval {
                start: call_start,
                end: call.interval.end,
            };
            let Some(active) = current_interval else {
                current_interval = Some(call_scope);
                current_calls.push(call);
                continue;
            };
            let tool_bridge = tools.iter().any(|tool| {
                tool.interval.start >= active.end && tool.interval.start < call.interval.start
            });
            if call.interval.start <= active.end || tool_bridge {
                current_interval = Some(Interval {
                    start: active.start.min(call_scope.start),
                    end: active.end.max(call_scope.end),
                });
                current_calls.push(call);
                continue;
            }
            if call.user_input_start.is_none() {
                tracker.action_info(
                    "detached_llm_call_excluded",
                    "A repeated user-message request was observed after its turn completed without new input or tool activity; it is treated as background activity.",
                    &call.action_id,
                    Some(call.interval),
                );
                continue;
            }
            finish_user_turn(
                active,
                std::mem::take(&mut current_calls),
                &mut accepted_calls,
                &mut turns,
            );
            current_interval = Some(call_scope);
            current_calls.push(call);
        }
        if let Some(interval) = current_interval {
            finish_user_turn(interval, current_calls, &mut accepted_calls, &mut turns);
        }
    }
    accepted_calls.sort_by(|left, right| {
        (left.interval.start, left.interval.end, &left.action_id).cmp(&(
            right.interval.start,
            right.interval.end,
            &right.action_id,
        ))
    });
    turns.sort_by(|left, right| {
        (
            left.interval.start,
            left.interval.end,
            &left.call_action_ids,
        )
            .cmp(&(
                right.interval.start,
                right.interval.end,
                &right.call_action_ids,
            ))
    });
    (accepted_calls, turns)
}

fn finish_user_turn(
    interval: Interval,
    calls: Vec<ModelInterval>,
    accepted_calls: &mut Vec<ModelInterval>,
    turns: &mut Vec<UserTurn>,
) {
    let call_action_ids = calls
        .iter()
        .map(|call| call.action_id.clone())
        .collect::<Vec<_>>();
    accepted_calls.extend(calls);
    turns.push(UserTurn {
        interval,
        call_action_ids,
    });
}

pub(super) fn merge_intervals(intervals: impl IntoIterator<Item = Interval>) -> Vec<Interval> {
    let mut intervals = intervals.into_iter().collect::<Vec<_>>();
    intervals.sort_by_key(|interval| (interval.start, interval.end));
    let mut merged = Vec::<Interval>::new();
    for interval in intervals {
        if let Some(previous) = merged.last_mut()
            && interval.start <= previous.end
        {
            previous.end = previous.end.max(interval.end);
            continue;
        }
        merged.push(interval);
    }
    merged
}

pub(super) fn clip_intervals(intervals: &[Interval], clip: Option<Interval>) -> Vec<Interval> {
    intervals
        .iter()
        .filter_map(|interval| match clip {
            Some(clip) => interval.intersect(clip),
            None => Some(*interval),
        })
        .collect()
}

pub(super) fn clip_model_intervals(
    calls: &[ModelInterval],
    windows: &[Interval],
) -> Vec<ModelInterval> {
    calls
        .iter()
        .flat_map(|call| {
            windows.iter().filter_map(|window| {
                let interval = call.interval.intersect(*window)?;
                let mut clipped = call.clone();
                clipped.interval = interval;
                Some(clipped)
            })
        })
        .collect()
}

pub(super) fn clip_tool_intervals(
    tools: &[ToolInterval],
    windows: &[Interval],
) -> Vec<ToolInterval> {
    tools
        .iter()
        .flat_map(|tool| {
            windows.iter().filter_map(|window| {
                let interval = tool.interval.intersect(*window)?;
                let mut clipped = tool.clone();
                clipped.interval = interval;
                Some(clipped)
            })
        })
        .collect()
}

pub(super) fn clip_command_intervals(
    commands: &[CommandInterval],
    windows: &[Interval],
) -> Vec<CommandInterval> {
    commands
        .iter()
        .flat_map(|command| {
            windows.iter().filter_map(|window| {
                let interval = command.interval.intersect(*window)?;
                let mut clipped = command.clone();
                clipped.interval = interval;
                Some(clipped)
            })
        })
        .collect()
}

pub(super) fn clip_user_turns(
    turns: &[UserTurn],
    calls: &[ModelInterval],
    clip: Option<Interval>,
) -> Vec<UserTurn> {
    let retained_call_ids = calls
        .iter()
        .map(|call| call.action_id.as_str())
        .collect::<BTreeSet<_>>();
    turns
        .iter()
        .filter_map(|turn| {
            let interval = match clip {
                Some(clip) => turn.interval.intersect(clip)?,
                None => turn.interval,
            };
            let call_action_ids = turn
                .call_action_ids
                .iter()
                .filter(|action_id| retained_call_ids.contains(action_id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            (!call_action_ids.is_empty()).then_some(UserTurn {
                interval,
                call_action_ids,
            })
        })
        .collect()
}

pub(super) fn attribution_scope(
    windows: &[Interval],
    fallback_start: u128,
    provisional: bool,
) -> AttributionScope {
    let start = windows
        .first()
        .map(|window| window.start)
        .unwrap_or(fallback_start);
    let end = windows
        .last()
        .map(|window| window.end)
        .unwrap_or(fallback_start);
    AttributionScope {
        start_unix_nanos: nanos_string(start),
        end_unix_nanos: nanos_string(end),
        duration_nanos: nanos_string(windows.iter().map(|window| window.duration()).sum()),
        provisional,
        semantics: "user_turn_union_exclusive_wall_clock",
        windows: windows
            .iter()
            .enumerate()
            .map(|(index, window)| AttributionScopeWindow {
                id: format!("user-turn-window-{}", index + 1),
                start_unix_nanos: nanos_string(window.start),
                end_unix_nanos: nanos_string(window.end),
                duration_nanos: nanos_string(window.duration()),
            })
            .collect(),
    }
}
