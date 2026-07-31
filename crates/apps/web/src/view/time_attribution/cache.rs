use super::projection::{project_trace_data, terminal_time};
use super::summary::{breakdown_shares, category_shares, command_breakdown_shares};
use super::*;

pub(super) fn project_trace(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace: &TraceRecord,
    clip_window: Option<Interval>,
) -> Result<TraceAttribution, String> {
    if trace.lifecycle_state.is_terminal() {
        let projection = cached_terminal_projection(storage_path, storage, trace)?;
        return Ok(match clip_window {
            Some(window) => clip_terminal_projection(projection.as_ref(), window),
            None => projection.as_ref().clone(),
        });
    }
    project_trace_uncached(storage, trace, clip_window)
}

fn project_trace_uncached(
    storage: &mut dyn StorageBackend,
    trace: &TraceRecord,
    clip_window: Option<Interval>,
) -> Result<TraceAttribution, String> {
    let actions = storage
        .list_semantic_actions(trace.trace_id)
        .map_err(|error| storage_error("list actions for time attribution", error))?;
    let links = storage
        .list_semantic_action_links(trace.trace_id)
        .map_err(|error| storage_error("list links for time attribution", error))?;
    let memberships = storage
        .trace_memberships(trace.trace_id)
        .map_err(|error| storage_error("list process memberships for time attribution", error))?;
    project_trace_data(trace, &actions, &links, &memberships, clip_window)
}

fn cached_terminal_projection(
    storage_path: &Path,
    storage: &mut dyn StorageBackend,
    trace: &TraceRecord,
) -> Result<Arc<TraceAttribution>, String> {
    let terminal_nanos = terminal_time(trace)
        .and_then(|time| system_time_nanos(time).ok())
        .unwrap_or_default();
    let key = TerminalCacheKey {
        storage_path: storage_key(storage_path),
        trace_id: trace.trace_id.get(),
        terminal_nanos,
        revision: storage_revision(storage_path),
    };
    if let Ok(mut cache) = terminal_cache().lock()
        && let Some(projection) = cache.get(&key)
    {
        return Ok(projection);
    }
    let projection = Arc::new(project_trace_uncached(storage, trace, None)?);
    if let Ok(mut cache) = terminal_cache().lock() {
        cache.insert(key, Arc::clone(&projection));
    }
    Ok(projection)
}

fn clip_terminal_projection(projection: &TraceAttribution, window: Interval) -> TraceAttribution {
    let full_scope = Interval {
        start: parse_nanos(&projection.scope.start_unix_nanos),
        end: parse_nanos(&projection.scope.end_unix_nanos),
    };
    let scope = full_scope.intersect(window).unwrap_or_else(|| {
        let boundary = full_scope.start.max(window.start);
        Interval {
            start: boundary,
            end: boundary,
        }
    });
    let mut segments = projection
        .segments
        .iter()
        .filter_map(|segment| {
            let interval = segment.interval.intersect(scope)?;
            let mut clipped = segment.clone();
            clipped.interval = interval;
            clipped.start_unix_nanos = nanos_string(interval.start);
            clipped.end_unix_nanos = nanos_string(interval.end);
            clipped.duration_nanos = nanos_string(interval.duration());
            Some(clipped)
        })
        .collect::<Vec<_>>();
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.id = format!("segment-{}", index + 1);
    }
    let mut command_segments = projection
        .command_segments
        .iter()
        .filter_map(|segment| {
            let interval = segment.interval.intersect(scope)?;
            let mut clipped = segment.clone();
            clipped.interval = interval;
            clipped.start_unix_nanos = nanos_string(interval.start);
            clipped.end_unix_nanos = nanos_string(interval.end);
            clipped.duration_nanos = nanos_string(interval.duration());
            Some(clipped)
        })
        .collect::<Vec<_>>();
    for (index, segment) in command_segments.iter_mut().enumerate() {
        segment.id = format!("command-segment-{}", index + 1);
    }
    let categories = category_shares(&segments, scope.duration());
    let models = breakdown_shares(&segments, scope.duration(), Category::ModelSide, "model");
    let tools = breakdown_shares(&segments, scope.duration(), Category::AgentSide, "agent");
    let commands = command_breakdown_shares(&command_segments, scope.duration());
    let llm_call_count = segments
        .iter()
        .filter(|segment| segment.category_value == Category::ModelSide)
        .flat_map(|segment| segment.action_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .len();
    let tool_interval_count = segments
        .iter()
        .filter(|segment| {
            segment.category_value == Category::AgentSide && segment.key != TOOL_ORCHESTRATION_KEY
        })
        .flat_map(|segment| segment.action_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .len();
    let command_interval_count = command_segments
        .iter()
        .filter(|segment| segment.kind != "tool_overhead")
        .flat_map(|segment| segment.action_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .len();
    TraceAttribution {
        schema_version: projection.schema_version,
        trace: projection.trace.clone(),
        scope: AttributionScope {
            start_unix_nanos: nanos_string(scope.start),
            end_unix_nanos: nanos_string(scope.end),
            duration_nanos: nanos_string(scope.duration()),
            provisional: false,
            semantics: "exclusive_wall_clock",
        },
        status: projection.status,
        categories,
        rounds: Vec::new(),
        models,
        tools,
        commands,
        bottlenecks: TraceBottlenecks::default(),
        coverage: TraceCoverage {
            llm_call_count,
            agent_process_count: projection.coverage.agent_process_count,
            tool_interval_count,
            command_interval_count,
            segment_count: segments.len(),
            command_segment_count: command_segments.len(),
        },
        segments,
        command_segments,
        issues: projection.issues.clone(),
    }
}

pub(super) fn clear_time_attribution_cache() -> usize {
    terminal_cache()
        .lock()
        .map(|mut cache| cache.clear())
        .unwrap_or_default()
}

fn terminal_cache() -> &'static Mutex<TerminalProjectionCache> {
    static CACHE: OnceLock<Mutex<TerminalProjectionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(TerminalProjectionCache::new()))
}

fn storage_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn storage_revision(path: &Path) -> StorageRevision {
    let mut wal_name = path.as_os_str().to_os_string();
    wal_name.push("-wal");
    StorageRevision {
        database: file_revision(path),
        write_ahead_log: file_revision(&PathBuf::from(wal_name)),
    }
}

fn file_revision(path: &Path) -> Option<FileRevision> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified_nanos = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(FileRevision {
        modified_nanos,
        len: metadata.len(),
    })
}
