use super::model::ToolCallOccurrence;
use super::*;

pub(super) fn trace_bottlenecks(
    calls: &[ModelInterval],
    commands: &[CommandInterval],
    segments: &[AttributionSegment],
    provisional: bool,
) -> TraceBottlenecks {
    let model_requests = calls
        .iter()
        .map(|call| {
            let key = call
                .model
                .clone()
                .unwrap_or_else(|| MODEL_UNKNOWN_KEY.to_string());
            let label = call
                .model
                .clone()
                .unwrap_or_else(|| "Unknown model".to_string());
            BottleneckInterval {
                id: String::new(),
                kind: "model_request",
                key,
                description: "Request start to observable response end".to_string(),
                label,
                status: call.status,
                start_unix_nanos: nanos_string(call.interval.start),
                end_unix_nanos: nanos_string(call.interval.end),
                duration_nanos: nanos_string(call.interval.duration()),
                action_ids: vec![call.action_id.clone()],
                interval: call.interval,
            }
        })
        .collect();
    let commands = commands
        .iter()
        .map(|command| BottleneckInterval {
            id: String::new(),
            kind: "command_occurrence",
            key: command.key.clone(),
            label: command.label.clone(),
            description: format!("Actual process interval under {}", command.agent_tool_label),
            status: command.status,
            start_unix_nanos: nanos_string(command.interval.start),
            end_unix_nanos: nanos_string(command.interval.end),
            duration_nanos: nanos_string(command.interval.duration()),
            action_ids: vec![command.action_id.clone()],
            interval: command.interval,
        })
        .collect();
    let unattributed_gaps = segments
        .iter()
        .filter(|segment| segment.category_value == Category::Unattributed)
        .map(|segment| BottleneckInterval {
            id: String::new(),
            kind: "unattributed_gap",
            key: Category::Unattributed.key().to_string(),
            label: "Unattributed gap".to_string(),
            description: "No reliable Agent or model activity was observable".to_string(),
            status: if provisional {
                "provisional"
            } else {
                "complete"
            },
            start_unix_nanos: segment.start_unix_nanos.clone(),
            end_unix_nanos: segment.end_unix_nanos.clone(),
            duration_nanos: segment.duration_nanos.clone(),
            action_ids: Vec::new(),
            interval: segment.interval,
        })
        .collect();
    TraceBottlenecks {
        default_display_limit: BOTTLENECK_DEFAULT_DISPLAY_LIMIT,
        model_requests: bottleneck_collection("model-request", model_requests),
        commands: bottleneck_collection("command", commands),
        unattributed_gaps: bottleneck_collection("unattributed-gap", unattributed_gaps),
    }
}

fn bottleneck_collection(
    id_prefix: &str,
    mut items: Vec<BottleneckInterval>,
) -> BottleneckCollection {
    let observed_count = items.len();
    items.sort_by(|left, right| {
        right
            .interval
            .duration()
            .cmp(&left.interval.duration())
            .then_with(|| left.interval.start.cmp(&right.interval.start))
            .then_with(|| left.action_ids.cmp(&right.action_ids))
    });
    for (index, item) in items.iter_mut().enumerate() {
        item.id = format!("{id_prefix}-{}", index + 1);
    }
    BottleneckCollection {
        observed_count,
        items,
    }
}

pub(super) fn category_shares(segments: &[AttributionSegment], total: u128) -> Vec<DurationShare> {
    let durations = Category::ALL.map(|category| {
        segments
            .iter()
            .filter(|segment| segment.category_value == category)
            .map(|segment| segment.interval.duration())
            .sum::<u128>()
    });
    let percentages = exact_percentages(&durations, total);
    Category::ALL
        .iter()
        .enumerate()
        .map(|(index, category)| DurationShare {
            key: category.key().to_string(),
            label: category.label().to_string(),
            duration_nanos: nanos_string(durations[index]),
            percentage_bps: percentages[index],
            segment_count: segments
                .iter()
                .filter(|segment| segment.category_value == *category)
                .count(),
            target: dominant_segment_target(
                segments
                    .iter()
                    .filter(|segment| segment.category_value == *category),
            ),
        })
        .collect()
}

pub(super) fn breakdown_shares(
    segments: &[AttributionSegment],
    total: u128,
    category: Category,
    kind: &str,
) -> Vec<BreakdownShare> {
    let mut groups = BTreeMap::<String, BreakdownAccumulator>::new();
    for segment in segments
        .iter()
        .filter(|segment| segment.category_value == category)
    {
        let group = groups
            .entry(segment.key.clone())
            .or_insert_with(|| BreakdownAccumulator {
                label: segment.label.clone(),
                ..Default::default()
            });
        let segment_duration = segment.interval.duration();
        group.duration += segment_duration;
        group.segment_count += 1;
        group.action_ids.extend(segment.action_ids.iter().cloned());
        if segment_duration > group.target_duration {
            group.target_duration = segment_duration;
            group.target = Some(target_for_segment(segment));
        }
    }
    let mut rows = groups
        .into_iter()
        .map(|(key, group)| BreakdownShare {
            key,
            label: group.label,
            kind: kind.to_string(),
            agent_tools: Vec::new(),
            duration_nanos: nanos_string(group.duration),
            percentage_bps: single_percentage(group.duration, total),
            segment_count: group.segment_count,
            action_count: group.action_count.max(group.action_ids.len()),
            target: group.target,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        parse_nanos(&right.duration_nanos)
            .cmp(&parse_nanos(&left.duration_nanos))
            .then_with(|| left.label.cmp(&right.label))
    });
    rows
}

pub(super) fn tool_breakdown_shares(
    tools: &[ToolInterval],
    calls: &[ToolCallOccurrence],
    total: u128,
) -> Vec<BreakdownShare> {
    let mut groups = BTreeMap::<String, ToolBreakdownAccumulator>::new();
    for call in calls {
        groups.entry(call.name.clone()).or_default().add_call(call);
    }
    for tool in tools {
        let Some(key) = tool.tool_name.as_deref() else {
            continue;
        };
        groups.entry(key.to_string()).or_default().add_tool(tool);
    }
    let mut rows = groups
        .into_iter()
        .map(|(key, group)| BreakdownShare {
            label: key.clone(),
            key,
            kind: "agent".to_string(),
            agent_tools: Vec::new(),
            duration_nanos: nanos_string(group.measured.duration),
            percentage_bps: single_percentage(group.measured.duration, total),
            segment_count: group.measured.action_ids.len(),
            action_count: group.call_action_ids.len(),
            target: group.measured.target,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        parse_nanos(&right.duration_nanos)
            .cmp(&parse_nanos(&left.duration_nanos))
            .then_with(|| left.label.cmp(&right.label))
    });
    rows
}

pub(super) fn command_breakdown_shares(
    segments: &[CommandSegment],
    total: u128,
) -> Vec<BreakdownShare> {
    let mut groups = BTreeMap::<String, BreakdownAccumulator>::new();
    let mut kinds = BTreeMap::<String, String>::new();
    for segment in segments {
        kinds
            .entry(segment.key.clone())
            .or_insert_with(|| segment.kind.as_str().to_string());
        let group = groups
            .entry(segment.key.clone())
            .or_insert_with(|| BreakdownAccumulator {
                label: segment.label.clone(),
                ..Default::default()
            });
        let segment_duration = segment.interval.duration();
        group.duration += segment_duration;
        group.segment_count += 1;
        group.action_ids.extend(segment.action_ids.iter().cloned());
        group
            .agent_tools
            .extend(segment.agent_tools.iter().cloned());
        if segment_duration > group.target_duration {
            group.target_duration = segment_duration;
            group.target = Some(target_for_command_segment(segment));
        }
    }
    let mut rows = groups
        .into_iter()
        .map(|(key, group)| BreakdownShare {
            kind: kinds.remove(&key).unwrap_or_else(|| "command".to_string()),
            key,
            label: group.label,
            agent_tools: group.agent_tools.into_iter().collect(),
            duration_nanos: nanos_string(group.duration),
            percentage_bps: single_percentage(group.duration, total),
            segment_count: group.segment_count,
            action_count: group.action_count.max(group.action_ids.len()),
            target: group.target,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        parse_nanos(&right.duration_nanos)
            .cmp(&parse_nanos(&left.duration_nanos))
            .then_with(|| left.label.cmp(&right.label))
    });
    rows
}

#[derive(Default)]
struct BreakdownAccumulator {
    label: String,
    duration: u128,
    segment_count: usize,
    action_ids: BTreeSet<String>,
    agent_tools: BTreeSet<String>,
    action_count: usize,
    target_duration: u128,
    target: Option<AttributionTarget>,
}

#[derive(Default)]
struct ToolBreakdownAccumulator {
    measured: BreakdownAccumulator,
    call_action_ids: BTreeSet<String>,
}

impl ToolBreakdownAccumulator {
    fn add_call(&mut self, call: &ToolCallOccurrence) {
        self.call_action_ids.insert(call.action_id.clone());
    }

    fn add_tool(&mut self, tool: &ToolInterval) {
        self.measured.add_tool(tool);
    }
}

impl BreakdownAccumulator {
    fn add_tool(&mut self, tool: &ToolInterval) {
        let duration = tool.interval.duration();
        self.duration = self.duration.saturating_add(duration);
        self.segment_count += 1;
        self.action_ids.insert(tool.action_id.clone());
        if duration > self.target_duration {
            self.target_duration = duration;
            self.target = Some(AttributionTarget {
                start_unix_nanos: nanos_string(tool.interval.start),
                end_unix_nanos: nanos_string(tool.interval.end),
                action_ids: vec![tool.action_id.clone()],
            });
        }
    }
}

pub(super) fn round_attributions(
    turns: &[UserTurn],
    calls: &[ModelInterval],
    segments: &[AttributionSegment],
    tools: &[ToolInterval],
    provisional: bool,
) -> Vec<RoundAttribution> {
    turns
        .iter()
        .enumerate()
        .map(|(index, turn)| {
            let turn_call_ids = turn
                .call_action_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            let turn_calls = calls
                .iter()
                .filter(|call| turn_call_ids.contains(call.action_id.as_str()))
                .collect::<Vec<_>>();
            let has_input_boundary = turn_calls
                .iter()
                .any(|call| call.user_input_start.is_some());
            let in_progress = provisional
                && turn_calls
                    .iter()
                    .any(|call| call.status == "in_progress" || call.status == "partial");
            let label = if in_progress {
                format!("Current user request {}", index + 1)
            } else {
                format!("User request {}", index + 1)
            };
            let start_boundary = if has_input_boundary {
                "Observed user input"
            } else {
                "First attributable model request"
            };
            let description = format!(
                "{start_boundary} → final observable response boundary · {} model {}",
                turn_calls.len(),
                if turn_calls.len() == 1 {
                    "call"
                } else {
                    "calls"
                }
            );
            round_for_interval(
                format!("user-request-{}", index + 1),
                label,
                description,
                "user_turn".to_string(),
                turn.interval,
                &turn_calls,
                segments,
                tools,
            )
        })
        .collect()
}

fn round_for_interval(
    id: String,
    label: String,
    description: String,
    kind: String,
    interval: Interval,
    calls: &[&ModelInterval],
    segments: &[AttributionSegment],
    tools: &[ToolInterval],
) -> RoundAttribution {
    let clipped_segments = segments
        .iter()
        .filter_map(|segment| {
            segment
                .interval
                .intersect(interval)
                .map(|clipped| (segment, clipped))
        })
        .collect::<Vec<_>>();
    let durations = Category::ALL.map(|category| {
        clipped_segments
            .iter()
            .filter(|(segment, _)| segment.category_value == category)
            .map(|(_, interval)| interval.duration())
            .sum::<u128>()
    });
    let percentages = exact_percentages(&durations, interval.duration());
    let categories = Category::ALL
        .iter()
        .enumerate()
        .map(|(index, category)| DurationShare {
            key: category.key().to_string(),
            label: category.label().to_string(),
            duration_nanos: nanos_string(durations[index]),
            percentage_bps: percentages[index],
            segment_count: clipped_segments
                .iter()
                .filter(|(segment, _)| segment.category_value == *category)
                .count(),
            target: clipped_segments
                .iter()
                .filter(|(segment, _)| segment.category_value == *category)
                .max_by_key(|(_, clipped)| clipped.duration())
                .map(|(segment, clipped)| AttributionTarget {
                    start_unix_nanos: nanos_string(clipped.start),
                    end_unix_nanos: nanos_string(clipped.end),
                    action_ids: segment.action_ids.clone(),
                }),
        })
        .collect();
    let models = clipped_segments
        .iter()
        .filter(|(segment, _)| segment.category_value == Category::ModelSide)
        .map(|(segment, _)| segment.label.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let tools = tools
        .iter()
        .filter(|tool| tool.interval.intersect(interval).is_some())
        .filter_map(|tool| tool.tool_name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let action_ids = calls
        .iter()
        .map(|call| call.action_id.clone())
        .collect::<Vec<_>>();
    RoundAttribution {
        id,
        label,
        description,
        kind,
        call_count: calls.len(),
        start_unix_nanos: nanos_string(interval.start),
        end_unix_nanos: nanos_string(interval.end),
        duration_nanos: nanos_string(interval.duration()),
        categories,
        models,
        tools,
        action_ids,
    }
}

pub(super) fn exact_percentages(durations: &[u128], total: u128) -> Vec<u32> {
    if durations.is_empty() {
        return Vec::new();
    }
    if total == 0 {
        return vec![0; durations.len()];
    }
    let mut floors = Vec::with_capacity(durations.len());
    let mut remainders = Vec::with_capacity(durations.len());
    for (index, duration) in durations.iter().copied().enumerate() {
        let scaled = duration.saturating_mul(u128::from(PERCENT_SCALE_BPS));
        floors.push(u32::try_from(scaled / total).unwrap_or(PERCENT_SCALE_BPS));
        remainders.push((scaled % total, index));
    }
    let assigned = floors.iter().copied().sum::<u32>();
    let mut remaining = PERCENT_SCALE_BPS.saturating_sub(assigned);
    remainders.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    for (_, index) in remainders {
        if remaining == 0 {
            break;
        }
        floors[index] = floors[index].saturating_add(1);
        remaining -= 1;
    }
    floors
}

pub(super) fn single_percentage(duration: u128, total: u128) -> u32 {
    if total == 0 {
        return 0;
    }
    let scaled = duration.saturating_mul(u128::from(PERCENT_SCALE_BPS));
    u32::try_from((scaled + total / 2) / total).unwrap_or(PERCENT_SCALE_BPS)
}

fn dominant_segment_target<'a>(
    segments: impl Iterator<Item = &'a AttributionSegment>,
) -> Option<AttributionTarget> {
    segments
        .max_by_key(|segment| segment.interval.duration())
        .map(target_for_segment)
}

pub(super) fn target_for_segment(segment: &AttributionSegment) -> AttributionTarget {
    AttributionTarget {
        start_unix_nanos: segment.start_unix_nanos.clone(),
        end_unix_nanos: segment.end_unix_nanos.clone(),
        action_ids: segment.action_ids.clone(),
    }
}

pub(super) fn target_for_command_segment(segment: &CommandSegment) -> AttributionTarget {
    AttributionTarget {
        start_unix_nanos: segment.start_unix_nanos.clone(),
        end_unix_nanos: segment.end_unix_nanos.clone(),
        action_ids: segment.action_ids.clone(),
    }
}

pub(super) fn attribution_target_duration(target: &AttributionTarget) -> u128 {
    parse_nanos(&target.end_unix_nanos).saturating_sub(parse_nanos(&target.start_unix_nanos))
}
