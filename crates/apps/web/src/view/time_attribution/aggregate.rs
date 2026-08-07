use super::cache::project_trace;
use super::projection::terminal_time;
use super::summary::{attribution_target_duration, single_percentage, target_for_segment};
use super::*;

#[derive(Default)]
struct AggregateBreakdownAccumulator {
    label: String,
    duration: u128,
    segment_count: usize,
    action_ids: BTreeSet<String>,
    agent_tools: BTreeSet<String>,
    action_count: usize,
    target_duration: u128,
    target: Option<AttributionTarget>,
}

pub(super) fn project_range(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    window: Interval,
) -> Result<Vec<TraceAttribution>, String> {
    let traces = storage
        .list_traces(&TraceFilter::default())
        .map_err(|error| storage_error("list traces for time attribution", error))?;
    let mut projections = Vec::new();
    for trace in traces {
        if !trace_may_overlap(&trace, window) {
            continue;
        }
        let projection = project_trace(storage_path, storage, &trace, Some(window))?;
        if trace_duration(&projection) > 0
            && (projection.coverage.llm_call_count > 0
                || projection.coverage.agent_process_count > 0)
        {
            projections.push(projection);
        }
    }
    Ok(projections)
}

fn trace_may_overlap(trace: &TraceRecord, window: Interval) -> bool {
    let start = trace.timings.started_at.unwrap_or(trace.timings.created_at);
    let Ok(start) = system_time_nanos(start) else {
        return true;
    };
    if start >= window.end {
        return false;
    }
    terminal_time(trace)
        .and_then(|time| system_time_nanos(time).ok())
        .is_none_or(|end| end > window.start)
}

pub(super) fn dominant_category_target(
    projections: &[TraceAttribution],
    category: Category,
) -> Option<AttributionTarget> {
    projections
        .iter()
        .flat_map(|projection| &projection.segments)
        .filter(|segment| segment.category_value == category)
        .max_by_key(|segment| segment.interval.duration())
        .map(target_for_segment)
}

pub(super) fn sum_category_totals(projections: &[TraceAttribution]) -> BTreeMap<String, u128> {
    let mut totals = BTreeMap::new();
    for projection in projections {
        for category in &projection.categories {
            *totals.entry(category.key.clone()).or_default() +=
                parse_nanos(&category.duration_nanos);
        }
    }
    totals
}

pub(super) fn aggregate_breakdowns(
    projections: &[TraceAttribution],
    total_duration: u128,
    select: impl Fn(&TraceAttribution) -> &[BreakdownShare],
) -> Vec<BreakdownShare> {
    let mut groups = BTreeMap::<String, AggregateBreakdownAccumulator>::new();
    let mut kinds = BTreeMap::<String, String>::new();
    for projection in projections {
        for row in select(projection) {
            kinds
                .entry(row.key.clone())
                .or_insert_with(|| row.kind.clone());
            let group =
                groups
                    .entry(row.key.clone())
                    .or_insert_with(|| AggregateBreakdownAccumulator {
                        label: row.label.clone(),
                        ..Default::default()
                    });
            group.duration += parse_nanos(&row.duration_nanos);
            group.segment_count += row.segment_count;
            group.action_count += row.action_count;
            group.agent_tools.extend(row.agent_tools.iter().cloned());
            group.action_ids.extend(
                row.target
                    .iter()
                    .flat_map(|target| target.action_ids.iter().cloned()),
            );
            let row_target_duration = row
                .target
                .as_ref()
                .map(attribution_target_duration)
                .unwrap_or_default();
            if row_target_duration > group.target_duration {
                group.target_duration = row_target_duration;
                group.target = row.target.clone();
            }
        }
    }
    let mut output = groups
        .into_iter()
        .map(|(key, group)| BreakdownShare {
            kind: kinds.remove(&key).unwrap_or_else(|| "unknown".to_string()),
            key,
            label: group.label,
            agent_tools: group.agent_tools.into_iter().collect(),
            duration_nanos: nanos_string(group.duration),
            percentage_bps: single_percentage(group.duration, total_duration),
            segment_count: group.segment_count,
            action_count: group.action_count.max(group.action_ids.len()),
            target: group.target,
        })
        .collect::<Vec<_>>();
    output.sort_by(|left, right| {
        parse_nanos(&right.duration_nanos)
            .cmp(&parse_nanos(&left.duration_nanos))
            .then_with(|| left.label.cmp(&right.label))
    });
    output
}

pub(super) fn aggregate_coverage(projections: &[TraceAttribution]) -> AggregateCoverage {
    let mut coverage = AggregateCoverage {
        trace_count: projections.len(),
        ..Default::default()
    };
    for projection in projections {
        match projection.status {
            AttributionStatus::Complete => coverage.complete_trace_count += 1,
            AttributionStatus::Provisional => coverage.provisional_trace_count += 1,
            AttributionStatus::Partial => coverage.partial_trace_count += 1,
            AttributionStatus::Invalid => coverage.invalid_trace_count += 1,
        }
        coverage.llm_request_count += projection.coverage.llm_request_count;
        coverage.observed_llm_call_count += projection.coverage.observed_llm_call_count;
        coverage.llm_call_count += projection.coverage.llm_call_count;
        coverage.excluded_llm_call_count += projection.coverage.excluded_llm_call_count;
        coverage.user_turn_count += projection.coverage.user_turn_count;
        coverage.tool_interval_count += projection.coverage.tool_interval_count;
        coverage.command_interval_count += projection.coverage.command_interval_count;
    }
    coverage
}

pub(super) fn aggregate_status(coverage: &AggregateCoverage) -> AttributionStatus {
    if coverage.invalid_trace_count > 0 {
        AttributionStatus::Invalid
    } else if coverage.partial_trace_count > 0 {
        AttributionStatus::Partial
    } else if coverage.provisional_trace_count > 0 {
        AttributionStatus::Provisional
    } else {
        AttributionStatus::Complete
    }
}

pub(super) fn aggregate_issues(projections: &[TraceAttribution]) -> Vec<AggregateIssue> {
    let mut issues = BTreeMap::<String, (IssueSeverity, usize, String)>::new();
    for issue in projections.iter().flat_map(|projection| &projection.issues) {
        let entry = issues
            .entry(issue.code.clone())
            .or_insert_with(|| (issue.severity, 0, issue.message.clone()));
        entry.0 = entry.0.max(issue.severity);
        entry.1 += 1;
    }
    issues
        .into_iter()
        .map(|(code, (severity, count, message))| AggregateIssue {
            code,
            severity,
            count,
            message,
        })
        .collect()
}

pub(super) fn attribution_row(
    projection: &TraceAttribution,
    query: &TimeAttributionRowsQuery,
) -> Option<AttributionRow> {
    let scope_duration = trace_duration(projection);
    let (contribution, target) = match query.dimension {
        None => (
            scope_duration,
            projection.segments.first().map(target_for_segment),
        ),
        Some(TimeAttributionDimension::Category) => {
            let key = query.key.as_deref()?;
            let row = projection.categories.iter().find(|row| row.key == key)?;
            (parse_nanos(&row.duration_nanos), row.target.clone())
        }
        Some(TimeAttributionDimension::Model) => {
            let key = query.key.as_deref()?;
            let row = projection.models.iter().find(|row| row.key == key)?;
            (parse_nanos(&row.duration_nanos), row.target.clone())
        }
        Some(TimeAttributionDimension::Tool) => {
            let key = query.key.as_deref()?;
            let row = projection.tools.iter().find(|row| row.key == key)?;
            (parse_nanos(&row.duration_nanos), row.target.clone())
        }
        Some(TimeAttributionDimension::Command) => {
            let key = query.key.as_deref()?;
            let row = projection.commands.iter().find(|row| row.key == key)?;
            (parse_nanos(&row.duration_nanos), row.target.clone())
        }
    };
    (contribution > 0).then(|| AttributionRow {
        trace: projection.trace.clone(),
        status: projection.status,
        scope_duration_nanos: nanos_string(scope_duration),
        contribution_duration_nanos: nanos_string(contribution),
        percentage_bps: single_percentage(contribution, scope_duration),
        target,
    })
}

pub(super) fn trace_duration(projection: &TraceAttribution) -> u128 {
    parse_nanos(&projection.scope.duration_nanos)
}
