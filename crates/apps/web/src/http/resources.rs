use super::query::{optional_query_param, parse_u64};
use super::{Response, STATUS_BAD_REQUEST, STATUS_NOT_FOUND, WebContext};
use model_core::ids::TraceId;
use resource_profiling::{
    model::ResourceSample,
    storage::{
        ResourceStorageBackendKind, ResourceStorageConfig, ResourceStorageFactory,
        database::{DatabaseStorageBackend, ResourceKind, ResourceQuery},
    },
    timeline::{
        DEFAULT_MAX_POINTS, DEFAULT_ROW_LIMIT, MAX_POINTS_LIMIT, ResourceTimelineReader,
        ResourceTimelineRequest, TrackReason,
    },
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage_core::StorageOpenMode;
use storage_factory::open_storage_backend;

pub(super) struct ResourceHttp<'a> {
    context: &'a WebContext,
}

/// The local resource database, or why the timeline cannot read one.
enum ResourceDatabase {
    Open(Box<dyn DatabaseStorageBackend>),
    Unavailable(TrackReason),
}

impl<'a> ResourceHttp<'a> {
    pub(super) fn new(context: &'a WebContext) -> Self {
        Self { context }
    }
    pub(super) fn route(&self, path: &str, query: &str) -> Option<Result<Response, String>> {
        let suffix = path.strip_prefix("/api/traces/")?;
        let (trace, part) = suffix.split_once('/')?;
        match part {
            "resources" => Some(self.read(trace, query)),
            "resource-timeline" => Some(self.timeline(trace, query)),
            _ => None,
        }
    }
    fn read(&self, trace: &str, query: &str) -> Result<Response, String> {
        if let Some(response) = self.unsupported() {
            return Ok(response);
        }
        let id = match parse_u64(trace) {
            Ok(id) => TraceId::new(id),
            Err(e) => return Ok(Response::text(STATUS_BAD_REQUEST, e)),
        };
        let limit = match optional_query_param(query, "limit")?
            .unwrap_or_else(|| "100".into())
            .parse::<u32>()
        {
            Ok(n) if n > 0 => n,
            _ => {
                return Ok(Response::text(
                    STATUS_BAD_REQUEST,
                    "limit must be a positive u32",
                ));
            }
        };
        let backend = ResourceStorageFactory::open_database(self.storage_config()?)
            .map_err(|e| e.to_string())?;
        let mut query = ResourceQuery::new(ResourceKind::Workload, limit);
        query.trace_id = Some(id);
        let samples = backend.query(&query).map_err(|e| e.to_string())?;
        let resources: Vec<_> = samples
            .into_iter()
            .filter_map(|s| {
                if let ResourceSample::Workload(s) = s {
                    Some(s)
                } else {
                    None
                }
            })
            .collect();
        Ok(Response::json(
            serde_json::json!({"resources":resources}).to_string(),
        ))
    }

    /// Downsampled workload and host tracks for one trace window.
    ///
    /// `from_ms` and `to_ms` are Unix milliseconds and default to the trace's
    /// lifetime; `max_points` bounds the buckets per track.
    fn timeline(&self, trace: &str, query: &str) -> Result<Response, String> {
        if let Some(response) = self.unsupported() {
            return Ok(response);
        }
        let id = match parse_u64(trace) {
            Ok(id) => TraceId::new(id),
            Err(e) => return Ok(Response::text(STATUS_BAD_REQUEST, e)),
        };
        let storage = open_storage_backend(&self.context.storage, StorageOpenMode::ReadOnly)
            .map_err(|e| format!("open storage read-only failed: {}: {}", e.stage, e.message))?;
        let Some(record) = storage
            .get_trace(id)
            .map_err(|e| format!("read trace failed: {}: {}", e.stage, e.message))?
        else {
            return Ok(Response::text(STATUS_NOT_FOUND, "trace not found"));
        };
        let timings = &record.timings;
        let started = timings.started_at.unwrap_or(timings.created_at);
        let ended = timings
            .completed_at
            .or(timings.failed_at)
            .or(timings.exited_at);
        let now = SystemTime::now();
        let millis = |key: &'static str| -> Result<Option<SystemTime>, String> {
            optional_query_param(query, key)?
                .map(|raw| {
                    raw.parse::<u64>()
                        .map(|ms| UNIX_EPOCH + Duration::from_millis(ms))
                        .map_err(|_| format!("{key} must be Unix milliseconds"))
                })
                .transpose()
        };
        let (from, to) = match (millis("from_ms"), millis("to_ms")) {
            (Ok(from), Ok(to)) => (from.unwrap_or(started), to.unwrap_or(ended.unwrap_or(now))),
            (Err(error), _) | (_, Err(error)) => {
                return Ok(Response::text(STATUS_BAD_REQUEST, error));
            }
        };
        if from >= to {
            return Ok(Response::text(
                STATUS_BAD_REQUEST,
                "from_ms must be earlier than to_ms",
            ));
        }
        let max_points = match optional_query_param(query, "max_points")?
            .map(|raw| raw.parse::<u32>())
            .transpose()
        {
            Ok(None) => DEFAULT_MAX_POINTS,
            Ok(Some(points)) if (1..=MAX_POINTS_LIMIT).contains(&points) => points,
            _ => {
                return Ok(Response::text(
                    STATUS_BAD_REQUEST,
                    format!("max_points must be between 1 and {MAX_POINTS_LIMIT}"),
                ));
            }
        };
        let request = ResourceTimelineRequest {
            trace_id: id,
            from,
            to,
            max_points,
            trace_started: started,
            trace_ended: ended,
            now,
            row_limit: DEFAULT_ROW_LIMIT,
        };
        let mut timeline = match self.timeline_database()? {
            ResourceDatabase::Open(backend) => {
                ResourceTimelineReader::read(backend.as_ref(), &request)
                    .map_err(|e| e.to_string())?
            }
            ResourceDatabase::Unavailable(reason) => {
                ResourceTimelineReader::unavailable(&request, reason)
            }
        };
        // Samples older than `cpu_count` lack it. Only local storage is read, so
        // this machine is the one that took them.
        if timeline.host.cpu_count.is_none() {
            timeline.host.cpu_count = local_cpu_count();
        }
        serde_json::to_string(&timeline)
            .map(Response::json)
            .map_err(|e| e.to_string())
    }

    /// Only the local database is read: remote receivers are queried directly.
    fn timeline_database(&self) -> Result<ResourceDatabase, String> {
        let config = self.storage_config()?;
        if config.backend == ResourceStorageBackendKind::Remote {
            return Ok(ResourceDatabase::Unavailable(TrackReason::RemoteStorage));
        }
        let database_paths = config.local_database_paths();
        let missing_file = database_paths.len() == 1 && !database_paths[0].exists();
        let remote_only = database_paths.is_empty();
        match ResourceStorageFactory::open_database(config) {
            Ok(backend) => Ok(ResourceDatabase::Open(backend)),
            // Nothing has been sampled yet on this host.
            Err(_) if missing_file => Ok(ResourceDatabase::Unavailable(TrackReason::NoSamples)),
            Err(_) if remote_only => Ok(ResourceDatabase::Unavailable(TrackReason::RemoteStorage)),
            Err(error) => Err(error.to_string()),
        }
    }

    fn unsupported(&self) -> Option<Response> {
        if self.context.cluster_root.is_some() {
            return Some(Response::text(
                STATUS_NOT_FOUND,
                "resource samples are queried from the originating host",
            ));
        }
        if self.context.operator_config.is_none() {
            return Some(Response::text(
                STATUS_NOT_FOUND,
                "resource storage is not configured",
            ));
        }
        None
    }

    fn storage_config(&self) -> Result<ResourceStorageConfig, String> {
        let config = self
            .context
            .operator_config
            .as_ref()
            .ok_or("resource storage is not configured")?;
        let mut storage = ResourceStorageConfig::parse(
            config
                .resource_profiling
                .get("storage")
                .and_then(toml::Value::as_table)
                .unwrap_or(&toml::Table::new()),
        )?;
        let path = self
            .context
            .operator_config_path
            .as_deref()
            .ok_or("resource configuration path is missing")?;
        storage.resolve_paths(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
        Ok(storage)
    }
}

fn local_cpu_count() -> Option<u32> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let count = stat
        .lines()
        .filter(|line| {
            line.strip_prefix("cpu")
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        })
        .count() as u32;
    (count > 0).then_some(count)
}
