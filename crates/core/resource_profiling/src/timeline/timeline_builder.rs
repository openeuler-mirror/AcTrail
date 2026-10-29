//! Streaming bucket aggregation for resource timelines; memory is O(buckets + samples' timestamps).

use super::resource_timeline::{
    Gap, HostPoint, HostTrack, WorkloadPoint, WorkloadSource, WorkloadSummary, WorkloadTrack,
};
use crate::model::host_resource::HostResourceSample;
use crate::model::{
    ResourceAccountingCoverage, ResourceAccountingMethod, ResourceMetrics, ResourceSampleKind,
};
use std::collections::BTreeMap;

/// A gap is a sample interval longer than this many times the median interval.
const GAP_FACTOR: u64 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimelineWindow {
    pub from_ms: u64,
    pub to_ms: u64,
    pub bucket_ms: u64,
    /// The part of the window in which the trace was alive; gaps never extend past it.
    pub active_from_ms: u64,
    pub active_to_ms: u64,
    pub includes_trace_start: bool,
}

impl TimelineWindow {
    pub(crate) fn new(
        from_ms: u64,
        to_ms: u64,
        max_points: u32,
        trace_from_ms: u64,
        trace_to_ms: u64,
    ) -> Self {
        let span = to_ms.saturating_sub(from_ms).max(1);
        let bucket_ms = span.div_ceil(u64::from(max_points.max(1))).max(1);
        let active_from_ms = trace_from_ms.clamp(from_ms, to_ms);
        Self {
            from_ms,
            to_ms,
            bucket_ms,
            active_from_ms,
            active_to_ms: trace_to_ms.clamp(active_from_ms, to_ms),
            includes_trace_start: from_ms <= trace_from_ms,
        }
    }

    fn bucket(&self, t_ms: u64) -> u64 {
        t_ms.saturating_sub(self.from_ms) / self.bucket_ms
    }

    fn bucket_start(&self, index: u64) -> u64 {
        self.from_ms + index * self.bucket_ms
    }

    /// Gaps between consecutive sample times, plus the uncovered head and tail of the active range.
    fn gaps(
        &self,
        times: &[u64],
        baseline: bool,
        extra_breaks: &[(u64, u64)],
    ) -> (Vec<Gap>, Option<u64>) {
        let mut intervals = times
            .windows(2)
            .map(|pair| pair[1].saturating_sub(pair[0]))
            .filter(|interval| *interval > 0)
            .collect::<Vec<_>>();
        if intervals.is_empty() {
            return (Vec::new(), None);
        }
        intervals.sort_unstable();
        let median = intervals[intervals.len() / 2];
        let threshold = median.saturating_mul(GAP_FACTOR);
        let mut gaps = Vec::new();
        let mut push = |from: u64, to: u64| {
            let (from, to) = (from.max(self.from_ms), to.min(self.to_ms));
            if to > from {
                gaps.push(Gap {
                    from_ms: from,
                    to_ms: to,
                });
            }
        };
        let (first, last) = (times[0], times[times.len() - 1]);
        if !baseline && first.saturating_sub(self.active_from_ms) > threshold {
            push(self.active_from_ms, first);
        }
        for pair in times.windows(2) {
            if pair[1].saturating_sub(pair[0]) > threshold
                || extra_breaks.contains(&(pair[0], pair[1]))
            {
                push(pair[0], pair[1]);
            }
        }
        if self.active_to_ms.saturating_sub(last) > threshold {
            push(last, self.active_to_ms);
        }
        (gaps, Some(median))
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn max_option<T: PartialOrd + Copy>(current: Option<T>, value: Option<T>) -> Option<T> {
    match (current, value) {
        (Some(a), Some(b)) => Some(if b > a { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// Weighted CPU accumulation shared by both tracks.
#[derive(Default)]
struct CpuAccumulator {
    weighted: f64,
    weight: f64,
    max: Option<f64>,
}

impl CpuAccumulator {
    fn add(&mut self, percent: f64, weight_seconds: f64) {
        self.weighted += percent * weight_seconds;
        self.weight += weight_seconds;
        self.max = max_option(self.max, Some(percent));
    }
    /// Samples with no known interval only count toward the maximum.
    fn avg(&self) -> Option<f64> {
        if self.weight > 0.0 {
            Some(round2(self.weighted / self.weight))
        } else {
            self.max
        }
    }
}

#[derive(Clone, Copy)]
struct Counters {
    t_ms: u64,
    cpu_usage_usec: Option<u64>,
    io_read: Option<u64>,
    io_write: Option<u64>,
}

impl Counters {
    fn of(t_ms: u64, metrics: &ResourceMetrics) -> Self {
        Self {
            t_ms,
            cpu_usage_usec: metrics.cpu_usage_usec,
            io_read: metrics.io_read_bytes,
            io_write: metrics.io_write_bytes,
        }
    }
}

#[derive(Default)]
struct WorkloadBucket {
    samples: u32,
    cpu: CpuAccumulator,
    memory_max: Option<u64>,
    anon_max: Option<u64>,
    io_read: u64,
    io_write: u64,
    io_seconds: f64,
    pids_max: Option<u64>,
}

pub(crate) struct WorkloadTimelineBuilder {
    window: TimelineWindow,
    baseline: Option<Counters>,
    previous: Option<Counters>,
    first: Option<Counters>,
    buckets: BTreeMap<u64, WorkloadBucket>,
    times: Vec<u64>,
    cpu_integrated_seconds: f64,
    cpu_integrated: bool,
    memory_peak: Option<u64>,
    memory_max: Option<u64>,
    pids_peak: Option<u64>,
    final_sample: bool,
    describe: Option<ResourceMetrics>,
}

impl WorkloadTimelineBuilder {
    pub(crate) fn new(window: TimelineWindow) -> Self {
        Self {
            window,
            baseline: None,
            previous: None,
            first: None,
            buckets: BTreeMap::new(),
            times: Vec::new(),
            cpu_integrated_seconds: 0.0,
            cpu_integrated: false,
            memory_peak: None,
            memory_max: None,
            pids_peak: None,
            final_sample: false,
            describe: None,
        }
    }

    /// The last sample before the window: a starting point for rates and totals.
    pub(crate) fn baseline(&mut self, t_ms: u64, metrics: &ResourceMetrics) {
        let counters = Counters::of(t_ms, metrics);
        self.baseline = Some(counters);
        self.previous = Some(counters);
        self.times.push(t_ms);
    }

    pub(crate) fn push(&mut self, t_ms: u64, metrics: &ResourceMetrics) {
        let counters = Counters::of(t_ms, metrics);
        let elapsed = self
            .previous
            .map(|previous| t_ms.saturating_sub(previous.t_ms) as f64 / 1000.0)
            .filter(|seconds| *seconds > 0.0);
        let bucket = self.buckets.entry(self.window.bucket(t_ms)).or_default();
        bucket.samples += 1;
        if let Some(millis) = metrics.cpu_percent_millis {
            let percent = millis as f64 / 1000.0;
            let weight = elapsed
                .or_else(|| {
                    metrics
                        .metadata
                        .get("cpu_rate_interval_usec")
                        .and_then(|raw| raw.parse::<f64>().ok())
                        .map(|usec| usec / 1_000_000.0)
                })
                .unwrap_or(0.0);
            bucket.cpu.add(percent, weight);
            if let Some(seconds) = elapsed {
                self.cpu_integrated_seconds += percent / 100.0 * seconds;
                self.cpu_integrated = true;
            }
        }
        let memory = metrics
            .memory_current_bytes
            .or(metrics.process_rss_sum_kb.map(|kb| kb * 1024))
            .or(metrics.rss_kb.map(|kb| kb * 1024));
        bucket.memory_max = max_option(bucket.memory_max, memory);
        bucket.anon_max = max_option(bucket.anon_max, metrics.memory_anon_bytes);
        bucket.pids_max = max_option(bucket.pids_max, metrics.pids_current);
        if let (Some(previous), Some(seconds)) = (self.previous, elapsed) {
            let read = Self::delta(previous.io_read, counters.io_read);
            let write = Self::delta(previous.io_write, counters.io_write);
            if read.is_some() || write.is_some() {
                bucket.io_read += read.unwrap_or(0);
                bucket.io_write += write.unwrap_or(0);
                bucket.io_seconds += seconds;
            }
        }
        self.memory_max = max_option(self.memory_max, memory);
        self.memory_peak = max_option(self.memory_peak, metrics.memory_peak_bytes);
        self.pids_peak = max_option(
            max_option(self.pids_peak, metrics.pids_peak),
            metrics.pids_current,
        );
        if metrics.sample_kind == ResourceSampleKind::Final {
            self.final_sample = true;
        } else if self.describe.is_none() || metrics.cpu_percent_millis.is_some() {
            self.describe = Some(metrics.clone());
        }
        if self.describe.is_none() {
            self.describe = Some(metrics.clone());
        }
        self.first.get_or_insert(counters);
        self.previous = Some(counters);
        self.times.push(t_ms);
    }

    /// A decrease means the counter was reset, which is not a rate.
    fn delta(previous: Option<u64>, current: Option<u64>) -> Option<u64> {
        current?.checked_sub(previous?)
    }

    pub(crate) fn sample_count(&self) -> usize {
        self.times.len() - usize::from(self.baseline.is_some())
    }

    pub(crate) fn has_final_sample(&self) -> bool {
        self.final_sample
    }

    /// The final sample, stored just after the trace's recorded end: it completes
    /// the totals but is not drawn, since it lies outside the window.
    pub(crate) fn tail(&mut self, metrics: &ResourceMetrics) {
        let Some(previous) = self.previous else {
            return;
        };
        self.final_sample |= metrics.sample_kind == ResourceSampleKind::Final;
        self.previous = Some(Counters::of(previous.t_ms, metrics));
        self.memory_peak = max_option(self.memory_peak, metrics.memory_peak_bytes);
        self.pids_peak = max_option(self.pids_peak, metrics.pids_peak);
    }

    pub(crate) fn finish(self) -> WorkloadTrack {
        let (gaps, interval) = self.window.gaps(&self.times, self.baseline.is_some(), &[]);
        let Some(describe) = self.describe.as_ref() else {
            return WorkloadTrack::default();
        };
        let exact_scope = describe.accounting_coverage == ResourceAccountingCoverage::Exact
            && describe.scope == "trace";
        // A trace-owned cgroup counts from zero only when the window includes its start.
        let start = |pick: fn(&Counters) -> Option<u64>| -> Option<u64> {
            match self.baseline {
                Some(baseline) => pick(&baseline),
                None if exact_scope && self.window.includes_trace_start => Some(0),
                None => self.first.as_ref().and_then(pick),
            }
        };
        let end = self.previous;
        let total = |pick: fn(&Counters) -> Option<u64>| {
            end.as_ref()
                .and_then(pick)
                .zip(start(pick))
                .map(|(last, first)| last.saturating_sub(first))
        };
        let cpu_seconds = total(|c| c.cpu_usage_usec)
            .map(|usec| round2(usec as f64 / 1_000_000.0))
            .or(self
                .cpu_integrated
                .then(|| round2(self.cpu_integrated_seconds)));
        let window = self.window;
        let points = self
            .buckets
            .into_iter()
            .map(|(index, bucket)| WorkloadPoint {
                t_ms: window.bucket_start(index),
                samples: bucket.samples,
                cpu_percent_avg: bucket.cpu.avg(),
                cpu_percent_max: bucket.cpu.max,
                memory_bytes_max: bucket.memory_max,
                memory_anon_bytes_max: bucket.anon_max,
                io_read_bytes_per_sec: (bucket.io_seconds > 0.0)
                    .then(|| round2(bucket.io_read as f64 / bucket.io_seconds)),
                io_write_bytes_per_sec: (bucket.io_seconds > 0.0)
                    .then(|| round2(bucket.io_write as f64 / bucket.io_seconds)),
                pids_max: bucket.pids_max,
            })
            .collect();
        let (source, source_path) = classify_source(describe);
        WorkloadTrack {
            available: true,
            reasons: Vec::new(),
            accounting_method: Some(describe.accounting_method.as_str()),
            accounting_coverage: Some(describe.accounting_coverage.as_str()),
            source: Some(source),
            source_path,
            sample_interval_ms: interval,
            points,
            gaps,
            summary: WorkloadSummary {
                cpu_seconds,
                // The kernel peak spans the cgroup's whole life, so it only bounds a
                // window that starts with the trace (no earlier sample).
                memory_peak_bytes: if exact_scope
                    && self.window.includes_trace_start
                    && self.baseline.is_none()
                {
                    max_option(self.memory_peak, self.memory_max)
                } else {
                    self.memory_max
                },
                io_read_bytes: total(|c| c.io_read),
                io_write_bytes: total(|c| c.io_write),
                pids_peak: self.pids_peak,
                final_sample: self.final_sample,
            },
        }
    }
}

/// Labels the cgroup track: a trace-owned scope, or the shared group it was bound to.
pub(crate) fn classify_source(metrics: &ResourceMetrics) -> (WorkloadSource, Option<String>) {
    let path = ["cgroup_relative_path", "cpuacct_path", "relative_path"]
        .iter()
        .find_map(|key| metrics.metadata.get(*key).cloned());
    if metrics.accounting_method == ResourceAccountingMethod::ProcfsRssSum {
        return (WorkloadSource::ProcessTree, None);
    }
    if metrics.scope == "trace" {
        return (WorkloadSource::Trace, path);
    }
    if metrics.scope == "container" {
        return (WorkloadSource::Container, path);
    }
    let leaf = path
        .as_deref()
        .and_then(|path| path.rsplit('/').find(|part| !part.is_empty()))
        .unwrap_or_default();
    let source = if leaf.starts_with("session-") && leaf.ends_with(".scope") {
        WorkloadSource::Session
    } else if leaf.ends_with(".service") {
        WorkloadSource::Service
    } else if ["docker", "containerd", "kubepods", "libpod", "crio"]
        .iter()
        .any(|runtime| path.as_deref().is_some_and(|path| path.contains(runtime)))
    {
        WorkloadSource::Container
    } else {
        WorkloadSource::Cgroup
    };
    (source, path)
}

#[derive(Default)]
struct HostBucket {
    samples: u32,
    cpu: CpuAccumulator,
    memory_used_max: Option<u64>,
    load1_max: Option<f64>,
}

pub(crate) struct HostTimelineBuilder {
    window: TimelineWindow,
    previous: Option<(u64, String)>,
    has_baseline: bool,
    buckets: BTreeMap<u64, HostBucket>,
    times: Vec<u64>,
    boot_changes: Vec<(u64, u64)>,
    memory_total: Option<u64>,
    cpu_count: Option<u32>,
}

impl HostTimelineBuilder {
    pub(crate) fn new(window: TimelineWindow) -> Self {
        Self {
            window,
            previous: None,
            has_baseline: false,
            buckets: BTreeMap::new(),
            times: Vec::new(),
            boot_changes: Vec::new(),
            memory_total: None,
            cpu_count: None,
        }
    }

    pub(crate) fn baseline(&mut self, t_ms: u64, sample: &HostResourceSample) {
        self.previous = Some((t_ms, sample.host_boot_id.to_string()));
        self.has_baseline = true;
        self.times.push(t_ms);
    }

    pub(crate) fn push(&mut self, t_ms: u64, sample: &HostResourceSample) {
        let boot = sample.host_boot_id.to_string();
        // A reboot resets counters and makes the interval meaningless.
        let elapsed = match &self.previous {
            Some((previous, previous_boot)) if *previous_boot == boot => {
                Some(t_ms.saturating_sub(*previous) as f64 / 1000.0).filter(|s| *s > 0.0)
            }
            Some((previous, _)) => {
                self.boot_changes.push((*previous, t_ms));
                None
            }
            None => None,
        };
        let bucket = self.buckets.entry(self.window.bucket(t_ms)).or_default();
        bucket.samples += 1;
        if let Some(millis) = sample.cpu_percent_millis {
            bucket
                .cpu
                .add(millis as f64 / 1000.0, elapsed.unwrap_or(0.0));
        }
        bucket.memory_used_max = max_option(
            bucket.memory_used_max,
            Some(
                sample
                    .memory_total_bytes
                    .saturating_sub(sample.memory_available_bytes),
            ),
        );
        bucket.load1_max = max_option(bucket.load1_max, Some(sample.load_average[0]));
        self.memory_total = Some(sample.memory_total_bytes);
        self.cpu_count = sample.cpu_count.or(self.cpu_count);
        self.previous = Some((t_ms, boot));
        self.times.push(t_ms);
    }

    pub(crate) fn sample_count(&self) -> usize {
        self.times.len() - usize::from(self.has_baseline)
    }

    pub(crate) fn finish(self) -> HostTrack {
        if self.sample_count() == 0 {
            return HostTrack::default();
        }
        let (gaps, interval) = self
            .window
            .gaps(&self.times, self.has_baseline, &self.boot_changes);
        let window = self.window;
        HostTrack {
            available: true,
            reasons: Vec::new(),
            sample_interval_ms: interval,
            memory_total_bytes: self.memory_total,
            cpu_count: self.cpu_count,
            points: self
                .buckets
                .into_iter()
                .map(|(index, bucket)| HostPoint {
                    t_ms: window.bucket_start(index),
                    samples: bucket.samples,
                    cpu_percent_avg: bucket.cpu.avg(),
                    cpu_percent_max: bucket.cpu.max,
                    memory_used_bytes_max: bucket.memory_used_max,
                    load1_max: bucket.load1_max,
                })
                .collect(),
            gaps,
        }
    }
}
