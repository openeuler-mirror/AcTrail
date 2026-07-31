use super::*;

pub(super) fn sweep_segments(
    scope: Interval,
    agent_intervals: &[Interval],
    calls: &[ModelInterval],
    tools: &[ToolInterval],
) -> Vec<AttributionSegment> {
    if scope.start >= scope.end {
        return Vec::new();
    }
    let mut boundaries = BTreeMap::<u128, Boundary>::new();
    boundaries.entry(scope.start).or_default();
    boundaries.entry(scope.end).or_default();
    for interval in agent_intervals {
        boundaries.entry(interval.start).or_default().agent_delta += 1;
        boundaries.entry(interval.end).or_default().agent_delta -= 1;
    }
    for (index, call) in calls.iter().enumerate() {
        boundaries
            .entry(call.interval.start)
            .or_default()
            .call_starts
            .push(index);
        boundaries
            .entry(call.interval.end)
            .or_default()
            .call_ends
            .push(index);
    }
    for (index, tool) in tools.iter().enumerate() {
        boundaries
            .entry(tool.interval.start)
            .or_default()
            .tool_starts
            .push(index);
        boundaries
            .entry(tool.interval.end)
            .or_default()
            .tool_ends
            .push(index);
    }
    let times = boundaries.keys().copied().collect::<Vec<_>>();
    let mut agent_count = 0i32;
    let mut active_calls = BTreeSet::<usize>::new();
    let mut active_tools = BTreeSet::<usize>::new();
    let mut output = Vec::new();
    for pair in times.windows(2) {
        let current = pair[0];
        let next = pair[1];
        let boundary = boundaries
            .get(&current)
            .expect("boundary timestamp came from the boundary map");
        for index in &boundary.call_ends {
            active_calls.remove(index);
        }
        for index in &boundary.tool_ends {
            active_tools.remove(index);
        }
        agent_count += boundary.agent_delta;
        for index in &boundary.call_starts {
            active_calls.insert(*index);
        }
        for index in &boundary.tool_starts {
            active_tools.insert(*index);
        }
        let Some(interval) = Interval::new(current, next) else {
            continue;
        };
        let segment = classify_segment(
            interval,
            agent_count > 0,
            &active_calls,
            &active_tools,
            calls,
            tools,
        );
        if let Some(previous) = output.last_mut()
            && mergeable(previous, &segment)
        {
            previous.interval.end = segment.interval.end;
            previous.end_unix_nanos = nanos_string(segment.interval.end);
            previous.duration_nanos = nanos_string(previous.interval.duration());
            continue;
        }
        output.push(segment);
    }
    output
}

fn classify_segment(
    interval: Interval,
    agent_active: bool,
    active_calls: &BTreeSet<usize>,
    active_tools: &BTreeSet<usize>,
    calls: &[ModelInterval],
    tools: &[ToolInterval],
) -> AttributionSegment {
    if !active_calls.is_empty() {
        let models = active_calls
            .iter()
            .map(|index| calls[*index].model.as_deref().unwrap_or(MODEL_UNKNOWN_KEY))
            .collect::<BTreeSet<_>>();
        let action_ids = active_calls
            .iter()
            .map(|index| calls[*index].action_id.clone())
            .collect::<Vec<_>>();
        let (subcategory, key, label) = if models.len() > 1 {
            (
                "concurrent_models",
                MODEL_CONCURRENT_KEY.to_string(),
                "Concurrent mixed models".to_string(),
            )
        } else {
            let model = models.iter().next().copied().unwrap_or(MODEL_UNKNOWN_KEY);
            if model == MODEL_UNKNOWN_KEY {
                (
                    "model",
                    MODEL_UNKNOWN_KEY.to_string(),
                    "Unknown model".to_string(),
                )
            } else {
                ("model", model.to_string(), model.to_string())
            }
        };
        return new_segment(
            Category::ModelSide,
            subcategory,
            key,
            label,
            interval,
            action_ids,
        );
    }
    if agent_active {
        if active_tools.is_empty() {
            return new_segment(
                Category::AgentSide,
                "orchestration",
                TOOL_ORCHESTRATION_KEY.to_string(),
                "Local orchestration / response handling".to_string(),
                interval,
                Vec::new(),
            );
        }
        let names = active_tools
            .iter()
            .map(|index| {
                tools[*index]
                    .tool_name
                    .as_deref()
                    .unwrap_or(TOOL_UNKNOWN_KEY)
            })
            .collect::<BTreeSet<_>>();
        let action_ids = active_tools
            .iter()
            .map(|index| tools[*index].action_id.clone())
            .collect::<Vec<_>>();
        let (subcategory, key, label) = if names.len() > 1 {
            (
                "concurrent_tools",
                TOOL_CONCURRENT_KEY.to_string(),
                "Concurrent tools".to_string(),
            )
        } else {
            let name = names.iter().next().copied().unwrap_or(TOOL_UNKNOWN_KEY);
            if name == TOOL_UNKNOWN_KEY {
                (
                    "tool",
                    TOOL_UNKNOWN_KEY.to_string(),
                    "Unidentified local command".to_string(),
                )
            } else {
                ("tool", name.to_string(), name.to_string())
            }
        };
        return new_segment(
            Category::AgentSide,
            subcategory,
            key,
            label,
            interval,
            action_ids,
        );
    }
    new_segment(
        Category::Unattributed,
        "unattributed",
        Category::Unattributed.key().to_string(),
        Category::Unattributed.label().to_string(),
        interval,
        Vec::new(),
    )
}

fn new_segment(
    category: Category,
    subcategory: &str,
    key: String,
    label: String,
    interval: Interval,
    action_ids: Vec<String>,
) -> AttributionSegment {
    AttributionSegment {
        id: String::new(),
        category: category.key().to_string(),
        subcategory: subcategory.to_string(),
        key,
        label,
        start_unix_nanos: nanos_string(interval.start),
        end_unix_nanos: nanos_string(interval.end),
        duration_nanos: nanos_string(interval.duration()),
        action_ids,
        interval,
        category_value: category,
    }
}

fn mergeable(left: &AttributionSegment, right: &AttributionSegment) -> bool {
    left.interval.end == right.interval.start
        && left.category_value == right.category_value
        && left.subcategory == right.subcategory
        && left.key == right.key
        && left.action_ids == right.action_ids
}

pub(super) fn sweep_command_segments(
    attribution_segments: &[AttributionSegment],
    commands: &[CommandInterval],
) -> Vec<CommandSegment> {
    let mut output = Vec::<CommandSegment>::new();
    for attribution in attribution_segments.iter().filter(|segment| {
        segment.category_value == Category::AgentSide && segment.key != TOOL_ORCHESTRATION_KEY
    }) {
        let relevant = commands
            .iter()
            .filter(|command| {
                (attribution.key == TOOL_CONCURRENT_KEY
                    || command.agent_tool_key == attribution.key)
                    && command.interval.intersect(attribution.interval).is_some()
            })
            .collect::<Vec<_>>();
        let mut boundaries = BTreeSet::from([attribution.interval.start, attribution.interval.end]);
        for command in &relevant {
            if let Some(interval) = command.interval.intersect(attribution.interval) {
                boundaries.insert(interval.start);
                boundaries.insert(interval.end);
            }
        }
        let times = boundaries.into_iter().collect::<Vec<_>>();
        for pair in times.windows(2) {
            let Some(interval) = Interval::new(pair[0], pair[1]) else {
                continue;
            };
            let active = relevant
                .iter()
                .copied()
                .filter(|command| {
                    command.interval.start < interval.end && command.interval.end > interval.start
                })
                .collect::<Vec<_>>();
            let segment = classify_command_segment(attribution, interval, &active);
            if let Some(previous) = output.last_mut()
                && mergeable_command_segment(previous, &segment)
            {
                previous.interval.end = segment.interval.end;
                previous.end_unix_nanos = nanos_string(segment.interval.end);
                previous.duration_nanos = nanos_string(previous.interval.duration());
                continue;
            }
            output.push(segment);
        }
    }
    output
}

fn classify_command_segment(
    attribution: &AttributionSegment,
    interval: Interval,
    active: &[&CommandInterval],
) -> CommandSegment {
    if active.is_empty() {
        return new_command_segment(
            "tool_overhead",
            format!("{COMMAND_OVERHEAD_KEY}:{}", attribution.key),
            format!("{} tool overhead", attribution.label),
            vec![attribution.label.clone()],
            interval,
            attribution.action_ids.clone(),
        );
    }

    let keys = active
        .iter()
        .map(|command| command.key.as_str())
        .collect::<BTreeSet<_>>();
    let agent_tools = active
        .iter()
        .map(|command| command.agent_tool_label.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let action_ids = active
        .iter()
        .map(|command| command.action_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if keys.len() > 1 {
        return new_command_segment(
            "concurrent_commands",
            COMMAND_CONCURRENT_KEY.to_string(),
            "Concurrent commands".to_string(),
            agent_tools,
            interval,
            action_ids,
        );
    }
    let command = active[0];
    new_command_segment(
        "command",
        command.key.clone(),
        command.label.clone(),
        agent_tools,
        interval,
        action_ids,
    )
}

fn new_command_segment(
    kind: &str,
    key: String,
    label: String,
    agent_tools: Vec<String>,
    interval: Interval,
    action_ids: Vec<String>,
) -> CommandSegment {
    CommandSegment {
        id: String::new(),
        kind: kind.to_string(),
        key,
        label,
        agent_tools,
        start_unix_nanos: nanos_string(interval.start),
        end_unix_nanos: nanos_string(interval.end),
        duration_nanos: nanos_string(interval.duration()),
        action_ids,
        interval,
    }
}

fn mergeable_command_segment(left: &CommandSegment, right: &CommandSegment) -> bool {
    left.interval.end == right.interval.start
        && left.kind == right.kind
        && left.key == right.key
        && left.agent_tools == right.agent_tools
        && left.action_ids == right.action_ids
}
