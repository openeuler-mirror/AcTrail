use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimeAttributionRangeQuery {
    pub from_ms: u64,
    pub to_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeAttributionDimension {
    Category,
    Model,
    Tool,
    Command,
}

impl TimeAttributionDimension {
    pub(crate) fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "category" => Ok(Self::Category),
            "model" => Ok(Self::Model),
            "tool" => Ok(Self::Tool),
            "command" => Ok(Self::Command),
            _ => Err(format!(
                "unsupported time attribution dimension {raw}; expected category, model, tool, or command"
            )),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Category => "category",
            Self::Model => "model",
            Self::Tool => "tool",
            Self::Command => "command",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TimeAttributionRowsQuery {
    pub range: TimeAttributionRangeQuery,
    pub offset: usize,
    pub limit: usize,
    pub dimension: Option<TimeAttributionDimension>,
    pub key: Option<String>,
}

pub(in crate::view) fn trace_time_attribution_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace_id: TraceId,
) -> Result<String, String> {
    let trace = storage
        .get_trace(trace_id)
        .map_err(|error| storage_error("read trace for time attribution", error))?
        .ok_or_else(|| format!("trace {trace_id} not found"))?;
    let attribution = project_trace(storage_path, storage, &trace, None)?;
    serde_json::to_string(&attribution)
        .map_err(|error| format!("serialize trace time attribution failed: {error}"))
}

pub(in crate::view) fn aggregate_time_attribution_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    query: TimeAttributionRangeQuery,
) -> Result<String, String> {
    let window = range_interval(query)?;
    let projections = project_range(storage_path, storage, window)?;
    let total_duration = projections.iter().map(trace_duration).sum::<u128>();

    let category_totals = sum_category_totals(&projections);
    let category_percentages = exact_percentages(
        &Category::ALL.map(|category| {
            category_totals
                .get(category.key())
                .copied()
                .unwrap_or_default()
        }),
        total_duration,
    );
    let categories = Category::ALL
        .iter()
        .enumerate()
        .map(|(index, category)| {
            let duration = category_totals
                .get(category.key())
                .copied()
                .unwrap_or_default();
            DurationShare {
                key: category.key().to_string(),
                label: category.label().to_string(),
                duration_nanos: nanos_string(duration),
                percentage_bps: category_percentages[index],
                segment_count: projections
                    .iter()
                    .map(|projection| {
                        projection
                            .segments
                            .iter()
                            .filter(|segment| segment.category_value == *category)
                            .count()
                    })
                    .sum(),
                target: dominant_category_target(&projections, *category),
            }
        })
        .collect::<Vec<_>>();
    let models = aggregate_breakdowns(&projections, total_duration, |projection| {
        &projection.models
    });
    let tools = aggregate_breakdowns(&projections, total_duration, |projection| &projection.tools);
    let tool_workloads = aggregate_tool_workloads(&tools);
    let commands = aggregate_breakdowns(&projections, total_duration, |projection| {
        &projection.commands
    });
    let coverage = aggregate_coverage(&projections);
    let status = aggregate_status(&coverage);
    let issues = aggregate_issues(&projections);
    let response = AggregateAttribution {
        schema_version: SCHEMA_VERSION,
        range: AggregateRange {
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            semantics: "trace_overlap_clipped",
        },
        status,
        total_duration_nanos: nanos_string(total_duration),
        categories,
        models,
        tools,
        tool_workloads,
        commands,
        coverage,
        issues,
    };
    serde_json::to_string(&response)
        .map_err(|error| format!("serialize aggregate time attribution failed: {error}"))
}

pub(in crate::view) fn time_attribution_rows_json(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    query: TimeAttributionRowsQuery,
) -> Result<String, String> {
    let window = range_interval(query.range)?;
    let projections = project_range(storage_path, storage, window)?;
    let mut rows = projections
        .iter()
        .filter_map(|projection| attribution_row(projection, &query))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        parse_nanos(&right.contribution_duration_nanos)
            .cmp(&parse_nanos(&left.contribution_duration_nanos))
            .then_with(|| right.trace.id.cmp(&left.trace.id))
    });
    let total = rows.len();
    let page_rows = rows
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect::<Vec<_>>();
    let next_offset = query
        .offset
        .checked_add(page_rows.len())
        .filter(|next| *next < total);
    let response = AttributionRows {
        schema_version: SCHEMA_VERSION,
        range: AggregateRange {
            from_ms: query.range.from_ms,
            to_ms: query.range.to_ms,
            semantics: "trace_overlap_clipped",
        },
        filter: RowsFilter {
            dimension: query.dimension.map(TimeAttributionDimension::as_str),
            key: query.key,
        },
        page: Page {
            offset: query.offset,
            limit: query.limit,
            total,
            next_offset,
        },
        rows: page_rows,
    };
    serde_json::to_string(&response)
        .map_err(|error| format!("serialize time attribution rows failed: {error}"))
}

pub(in crate::view) fn clear_time_attribution_cache() -> usize {
    cache::clear_time_attribution_cache()
}

fn range_interval(query: TimeAttributionRangeQuery) -> Result<Interval, String> {
    let start = u128::from(query.from_ms) * NANOS_PER_MILLI;
    let end = u128::from(query.to_ms) * NANOS_PER_MILLI;
    Interval::new(start, end)
        .ok_or_else(|| "time attribution range must have from_ms less than to_ms".to_string())
}
