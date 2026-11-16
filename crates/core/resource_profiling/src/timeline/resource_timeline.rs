//! Downsampled resource tracks for one trace over one time window.
//!
//! Times are Unix milliseconds. CPU is in percent of one core for workload
//! tracks and percent of the whole machine for the host track; `host.cpu_count`
//! converts between the two.

use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ResourceTimeline {
    pub trace_id: u64,
    pub from_ms: u64,
    pub to_ms: u64,
    pub bucket_ms: u64,
    pub workload: WorkloadTrack,
    pub host: HostTrack,
}

/// Why a track is missing or incomplete.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackReason {
    /// Samples go to Prometheus or InfluxDB; this API reads only the local database.
    RemoteStorage,
    /// No sample in the window.
    NoSamples,
    /// No sample in the window, which ends before the retention horizon.
    PastRetention,
    /// The trace ended but its final sample was never stored (writes are best-effort).
    FinalSampleMissing,
    /// The window holds more samples than one request reads.
    Truncated,
}

/// Where the workload numbers come from, as shown next to the cgroup track.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadSource {
    /// A cgroup that holds only this trace's processes.
    Trace,
    /// A login session's cgroup, shared with everything else in the session.
    Session,
    /// A systemd service's cgroup, shared with the daemon and its other children.
    Service,
    /// A container's cgroup.
    Container,
    /// Another pre-existing cgroup.
    Cgroup,
    /// A procfs sum over the trace's process tree.
    ProcessTree,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WorkloadTrack {
    pub available: bool,
    pub reasons: Vec<TrackReason>,
    pub accounting_method: Option<&'static str>,
    pub accounting_coverage: Option<&'static str>,
    pub source: Option<WorkloadSource>,
    pub source_path: Option<String>,
    pub sample_interval_ms: Option<u64>,
    pub points: Vec<WorkloadPoint>,
    pub gaps: Vec<Gap>,
    pub summary: WorkloadSummary,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WorkloadPoint {
    pub t_ms: u64,
    pub samples: u32,
    pub cpu_percent_avg: Option<f64>,
    pub cpu_percent_max: Option<f64>,
    pub memory_bytes_max: Option<u64>,
    pub memory_anon_bytes_max: Option<u64>,
    pub io_read_bytes_per_sec: Option<f64>,
    pub io_write_bytes_per_sec: Option<f64>,
    pub pids_max: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct WorkloadSummary {
    pub cpu_seconds: Option<f64>,
    pub memory_peak_bytes: Option<u64>,
    pub io_read_bytes: Option<u64>,
    pub io_write_bytes: Option<u64>,
    pub pids_peak: Option<u64>,
    pub final_sample: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HostTrack {
    pub available: bool,
    pub reasons: Vec<TrackReason>,
    pub sample_interval_ms: Option<u64>,
    pub memory_total_bytes: Option<u64>,
    /// Online CPUs, the denominator of every CPU percentage.
    pub cpu_count: Option<u32>,
    pub points: Vec<HostPoint>,
    pub gaps: Vec<Gap>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HostPoint {
    pub t_ms: u64,
    pub samples: u32,
    pub cpu_percent_avg: Option<f64>,
    pub cpu_percent_max: Option<f64>,
    pub memory_used_bytes_max: Option<u64>,
    pub load1_max: Option<f64>,
}

/// A stretch without samples, to be drawn as a break rather than interpolated.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Gap {
    pub from_ms: u64,
    pub to_ms: u64,
}
