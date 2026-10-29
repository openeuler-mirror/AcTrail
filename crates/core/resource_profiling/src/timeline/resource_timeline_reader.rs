//! Reads one trace's resource window from the local database and downsamples it.

use super::resource_timeline::{ResourceTimeline, TrackReason};
use super::timeline_builder::{HostTimelineBuilder, TimelineWindow, WorkloadTimelineBuilder};
use crate::model::{ResourceSample, ResourceSampleKind};
use crate::storage::ResourceStorageError;
use crate::storage::database::{DatabaseStorageBackend, ResourceKind, ResourceQuery};
use model_core::ids::TraceId;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DEFAULT_MAX_POINTS: u32 = 600;
pub const MAX_POINTS_LIMIT: u32 = 2000;
/// Rows read per track and request; about 55 hours of 1 Hz samples.
pub const DEFAULT_ROW_LIMIT: u32 = 200_000;
/// How long after a trace's recorded end its final sample is looked for.
const FINAL_SAMPLE_GRACE: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct ResourceTimelineRequest {
    pub trace_id: TraceId,
    pub from: SystemTime,
    pub to: SystemTime,
    pub max_points: u32,
    pub trace_started: SystemTime,
    /// `None` while the trace is still running.
    pub trace_ended: Option<SystemTime>,
    pub now: SystemTime,
    pub row_limit: u32,
}

pub struct ResourceTimelineReader;

impl ResourceTimelineReader {
    pub fn read(
        backend: &dyn DatabaseStorageBackend,
        request: &ResourceTimelineRequest,
    ) -> Result<ResourceTimeline, ResourceStorageError> {
        let window = Self::window(request);
        let mut timeline = Self::empty(request, window);

        let mut workload = WorkloadTimelineBuilder::new(window);
        let mut query = Self::query(request, ResourceKind::Workload);
        if let Some(sample) = Self::baseline(backend, &query)? {
            if let ResourceSample::Workload(sample) = sample {
                workload.baseline(millis(sample.observed_at), &sample.payload);
            }
        }
        query.limit = request.row_limit;
        let visited = backend.scan_ascending(&query, &mut |sample| {
            if let ResourceSample::Workload(sample) = sample {
                workload.push(millis(sample.observed_at), &sample.payload);
            }
        })?;
        let samples = workload.sample_count();
        let ended_in_window = request.trace_ended.is_some_and(|ended| ended <= request.to);
        if samples > 0 && ended_in_window && !workload.has_final_sample() {
            if let Some(ResourceSample::Workload(sample)) = Self::after(backend, &query)? {
                workload.tail(&sample.payload);
            }
        }
        timeline.workload = workload.finish();
        timeline.workload.reasons = Self::reasons(backend, request, samples, visited);
        if samples > 0 && !timeline.workload.summary.final_sample && ended_in_window {
            timeline
                .workload
                .reasons
                .push(TrackReason::FinalSampleMissing);
        }

        let mut host = HostTimelineBuilder::new(window);
        let mut query = Self::query(request, ResourceKind::Host);
        query.trace_id = None;
        if let Some(ResourceSample::Host(sample)) = Self::baseline(backend, &query)? {
            host.baseline(millis(sample.observed_at), &sample);
        }
        query.limit = request.row_limit;
        let visited = backend.scan_ascending(&query, &mut |sample| {
            if let ResourceSample::Host(sample) = sample {
                host.push(millis(sample.observed_at), &sample);
            }
        })?;
        let samples = host.sample_count();
        timeline.host = host.finish();
        timeline.host.reasons = Self::reasons(backend, request, samples, visited);
        Ok(timeline)
    }

    /// A timeline whose tracks carry only `reason`, such as remote storage.
    pub fn unavailable(request: &ResourceTimelineRequest, reason: TrackReason) -> ResourceTimeline {
        let mut timeline = Self::empty(request, Self::window(request));
        timeline.workload.reasons = vec![reason];
        timeline.host.reasons = vec![reason];
        timeline
    }

    fn window(request: &ResourceTimelineRequest) -> TimelineWindow {
        TimelineWindow::new(
            millis(request.from),
            millis(request.to),
            request.max_points.clamp(1, MAX_POINTS_LIMIT),
            millis(request.trace_started),
            millis(request.trace_ended.unwrap_or(request.now)),
        )
    }

    fn empty(request: &ResourceTimelineRequest, window: TimelineWindow) -> ResourceTimeline {
        ResourceTimeline {
            trace_id: request.trace_id.get(),
            from_ms: window.from_ms,
            to_ms: window.to_ms,
            bucket_ms: window.bucket_ms,
            ..ResourceTimeline::default()
        }
    }

    fn query(request: &ResourceTimelineRequest, kind: ResourceKind) -> ResourceQuery {
        let mut query = ResourceQuery::new(kind, 1);
        query.trace_id = Some(request.trace_id);
        query.since = Some(request.from);
        query.until = Some(request.to);
        query
    }

    /// The newest sample before the window, so the first bucket has a rate and totals a start.
    fn baseline(
        backend: &dyn DatabaseStorageBackend,
        window: &ResourceQuery,
    ) -> Result<Option<ResourceSample>, ResourceStorageError> {
        let Some(until) = window
            .since
            .and_then(|since| since.checked_sub(Duration::from_nanos(1)))
            .filter(|until| *until > UNIX_EPOCH)
        else {
            return Ok(None);
        };
        let mut query = window.clone();
        query.since = None;
        query.until = Some(until);
        query.limit = 1;
        Ok(backend.query(&query)?.into_iter().next())
    }

    /// Find the final sample shortly after the window, skipping periodic samples.
    fn after(
        backend: &dyn DatabaseStorageBackend,
        window: &ResourceQuery,
    ) -> Result<Option<ResourceSample>, ResourceStorageError> {
        let Some(until) = window.until else {
            return Ok(None);
        };
        let mut query = window.clone();
        query.since = until.checked_add(Duration::from_nanos(1));
        query.until = until.checked_add(FINAL_SAMPLE_GRACE);
        let mut final_sample = None;
        backend.scan_ascending(&query, &mut |sample| {
            if final_sample.is_none()
                && matches!(&sample, ResourceSample::Workload(workload)
                    if workload.payload.sample_kind == ResourceSampleKind::Final)
            {
                final_sample = Some(sample);
            }
        })?;
        Ok(final_sample)
    }

    fn reasons(
        backend: &dyn DatabaseStorageBackend,
        request: &ResourceTimelineRequest,
        samples: usize,
        visited: u32,
    ) -> Vec<TrackReason> {
        let mut reasons = Vec::new();
        if samples == 0 {
            let horizon = backend
                .retention_max_age()
                .and_then(|age| request.now.checked_sub(age));
            reasons.push(if horizon.is_some_and(|horizon| request.to < horizon) {
                TrackReason::PastRetention
            } else {
                TrackReason::NoSamples
            });
        }
        if visited >= request.row_limit {
            reasons.push(TrackReason::Truncated);
        }
        reasons
    }
}

fn millis(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
