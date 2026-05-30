//! Config-gated process resource sampling from procfs.

#[path = "resource_metrics/procfs.rs"]
mod procfs;

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

use config_core::daemon::ResourceMetricsConfig;
use control_contract::reply::ControlError;
use model_core::capability::Capability;
use model_core::event::ResourcePayload;
use model_core::ids::TraceId;
use model_core::process::{MembershipState, ProcessIdentity};
use trace_runtime::registry::TraceEntry;

use self::procfs::{
    BYTES_PER_KIB, SystemMetrics, SystemUnits, cpu_cores, read_proc_memory, read_proc_stat,
    read_system_metrics,
};

pub(super) const COLLECTOR_NAME: &str = "resource-sampler";

const NANOS_PER_SECOND: u128 = 1_000_000_000;
const PERCENT_MILLIS_SCALE: u128 = 100_000;

#[derive(Clone, Debug)]
pub(super) struct ResourceSampleDraft {
    pub trace_id: TraceId,
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub payload: ResourcePayload,
}

pub(super) struct ResourceMetricsSampler {
    config: ResourceMetricsConfig,
    next_sample_at: Option<Instant>,
    previous_cpu: BTreeMap<ProcessIdentity, CpuSample>,
    units: Option<SystemUnits>,
}

impl ResourceMetricsSampler {
    pub(super) fn new(config: ResourceMetricsConfig) -> Self {
        let next_sample_at = config
            .enabled
            .then(|| Instant::now() + Duration::from_millis(config.interval_ms));
        Self {
            config,
            next_sample_at,
            previous_cpu: BTreeMap::new(),
            units: None,
        }
    }

    pub(super) fn poll_timeout(&self) -> Option<Duration> {
        let next_sample_at = self.next_sample_at?;
        Some(next_sample_at.saturating_duration_since(Instant::now()))
    }

    pub(super) fn drain_due(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
    ) -> Result<Vec<ResourceSampleDraft>, ControlError> {
        let Some(next_sample_at) = self.next_sample_at else {
            return Ok(Vec::new());
        };
        let now = Instant::now();
        if now < next_sample_at {
            return Ok(Vec::new());
        }
        self.next_sample_at = Some(now + Duration::from_millis(self.config.interval_ms));
        self.collect_samples(trace_runtime, now, SystemTime::now())
    }

    fn collect_samples(
        &mut self,
        trace_runtime: &trace_runtime::TraceRuntime,
        sampled_at: Instant,
        observed_at: SystemTime,
    ) -> Result<Vec<ResourceSampleDraft>, ControlError> {
        let units = self.units()?;
        let mut drafts = Vec::new();
        let mut active_identities = BTreeSet::new();
        for trace in trace_runtime.list_trace_records() {
            let Some(entry) = trace_runtime.get_trace(trace.trace_id) else {
                continue;
            };
            if !trace_requests_resource_metrics(entry) {
                continue;
            }
            let identities = sample_identities(entry, self.config.include_children);
            active_identities.extend(identities.iter().cloned());
            if let Some(draft) =
                self.collect_trace_sample(entry, identities, sampled_at, observed_at, units)?
            {
                drafts.push(draft);
            }
        }
        self.previous_cpu
            .retain(|identity, _| active_identities.contains(identity));
        Ok(drafts)
    }

    fn collect_trace_sample(
        &mut self,
        entry: &TraceEntry,
        identities: Vec<ProcessIdentity>,
        sampled_at: Instant,
        observed_at: SystemTime,
        units: SystemUnits,
    ) -> Result<Option<ResourceSampleDraft>, ControlError> {
        let root = entry.trace.root_process_identity.clone();
        let mut total_cpu_percent_millis = 0_u64;
        let mut total_rss_kb = 0_u64;
        let mut total_virtual_memory_kb = 0_u64;
        let mut sampled_processes = 0_usize;
        let mut root_threads = None;
        let mut root_comm = None;

        for identity in identities {
            let Some(stat) = read_proc_stat(identity.pid)? else {
                continue;
            };
            let Some(memory) = read_proc_memory(identity.pid, units.page_size_kb)? else {
                continue;
            };
            total_cpu_percent_millis = total_cpu_percent_millis.saturating_add(
                self.cpu_percent_millis(&identity, stat.total_cpu_ticks, sampled_at, units),
            );
            total_rss_kb = total_rss_kb.saturating_add(memory.rss_kb);
            total_virtual_memory_kb =
                total_virtual_memory_kb.saturating_add(memory.virtual_memory_kb);
            sampled_processes += 1;
            if identity == root {
                root_threads = Some(stat.threads);
                root_comm = Some(stat.comm);
            }
        }

        if sampled_processes == 0 {
            return Ok(None);
        }

        let mut metadata = BTreeMap::new();
        if let Some(comm) = root_comm {
            metadata.insert("comm".to_string(), comm);
        }
        metadata.insert(
            "sampled_processes".to_string(),
            sampled_processes.to_string(),
        );
        metadata.insert(
            "children".to_string(),
            sampled_processes.saturating_sub(1).to_string(),
        );
        metadata.insert(
            "threads".to_string(),
            root_threads.unwrap_or_default().to_string(),
        );
        metadata.insert(
            "cpu_percent".to_string(),
            format_percent_millis(total_cpu_percent_millis),
        );
        metadata.insert(
            "cpu_percent_millis".to_string(),
            total_cpu_percent_millis.to_string(),
        );
        metadata.insert("cpu_cores".to_string(), cpu_cores()?);
        metadata.insert("rss_kb".to_string(), total_rss_kb.to_string());
        metadata.insert(
            "rss_mb".to_string(),
            (total_rss_kb / BYTES_PER_KIB).to_string(),
        );
        metadata.insert(
            "virtual_memory_kb".to_string(),
            total_virtual_memory_kb.to_string(),
        );
        metadata.insert(
            "virtual_memory_mb".to_string(),
            (total_virtual_memory_kb / BYTES_PER_KIB).to_string(),
        );
        metadata.insert(
            "include_children".to_string(),
            self.config.include_children.to_string(),
        );
        metadata.insert(
            "alert".to_string(),
            resource_alert(&self.config, total_cpu_percent_millis, total_rss_kb).to_string(),
        );
        if self.config.include_system {
            add_system_metadata(&mut metadata, read_system_metrics()?);
        }

        Ok(Some(ResourceSampleDraft {
            trace_id: entry.trace.trace_id,
            observed_at,
            process: root.clone(),
            payload: ResourcePayload {
                scope: if self.config.include_children {
                    "process_tree".to_string()
                } else {
                    "process".to_string()
                },
                subject: format!("pid:{}", root.pid),
                cpu_percent_millis: Some(total_cpu_percent_millis),
                rss_kb: Some(total_rss_kb),
                virtual_memory_kb: Some(total_virtual_memory_kb),
                metadata,
            },
        }))
    }

    fn cpu_percent_millis(
        &mut self,
        identity: &ProcessIdentity,
        total_cpu_ticks: u64,
        sampled_at: Instant,
        units: SystemUnits,
    ) -> u64 {
        let current = CpuSample {
            total_cpu_ticks,
            sampled_at,
        };
        let percent = self
            .previous_cpu
            .get(identity)
            .and_then(|previous| {
                let elapsed = sampled_at.checked_duration_since(previous.sampled_at)?;
                let elapsed_nanos = elapsed.as_nanos();
                if elapsed_nanos == u128::default() || total_cpu_ticks < previous.total_cpu_ticks {
                    return None;
                }
                let tick_delta = u128::from(total_cpu_ticks - previous.total_cpu_ticks);
                let numerator = tick_delta
                    .saturating_mul(NANOS_PER_SECOND)
                    .saturating_mul(PERCENT_MILLIS_SCALE);
                let denominator =
                    u128::from(units.clock_ticks_per_second).saturating_mul(elapsed_nanos);
                u64::try_from(numerator / denominator).ok()
            })
            .unwrap_or_default();
        self.previous_cpu.insert(identity.clone(), current);
        percent
    }

    fn units(&mut self) -> Result<SystemUnits, ControlError> {
        if self.units.is_none() {
            self.units = Some(
                SystemUnits::read()
                    .map_err(|message| ControlError::new("resource_metrics_sysconf", message))?,
            );
        }
        self.units
            .ok_or_else(|| ControlError::new("resource_metrics_sysconf", "missing system units"))
    }
}

#[derive(Clone, Copy)]
struct CpuSample {
    total_cpu_ticks: u64,
    sampled_at: Instant,
}

fn trace_requests_resource_metrics(entry: &TraceEntry) -> bool {
    entry.sensor_plan.collectors.iter().any(|collector| {
        collector
            .capabilities
            .iter()
            .any(|capability| *capability == Capability::ResourceMetrics)
    })
}

fn sample_identities(entry: &TraceEntry, include_children: bool) -> Vec<ProcessIdentity> {
    if include_children {
        return entry
            .memberships
            .memberships()
            .filter(|membership| membership.capture_enabled)
            .filter(|membership| {
                matches!(
                    membership.state,
                    MembershipState::Starting | MembershipState::Active
                )
            })
            .map(|membership| membership.identity.clone())
            .collect();
    }
    vec![entry.trace.root_process_identity.clone()]
}

fn resource_alert(config: &ResourceMetricsConfig, cpu_percent_millis: u64, rss_kb: u64) -> bool {
    config
        .cpu_alert_percent_millis
        .map(|threshold| cpu_percent_millis >= threshold)
        .unwrap_or(false)
        || config
            .memory_alert_rss_kb
            .map(|threshold| rss_kb >= threshold)
            .unwrap_or(false)
}

fn add_system_metadata(metadata: &mut BTreeMap<String, String>, system: SystemMetrics) {
    metadata.insert(
        "host_mem_total_kb".to_string(),
        system.mem_total_kb.to_string(),
    );
    metadata.insert(
        "host_mem_free_kb".to_string(),
        system.mem_free_kb.to_string(),
    );
    metadata.insert(
        "host_mem_available_kb".to_string(),
        system.mem_available_kb.to_string(),
    );
    metadata.insert("host_loadavg_1m".to_string(), system.loadavg_1m);
    metadata.insert("host_loadavg_5m".to_string(), system.loadavg_5m);
    metadata.insert("host_loadavg_15m".to_string(), system.loadavg_15m);
    metadata.insert(
        "host_loadavg_running_threads".to_string(),
        system.loadavg_running_threads,
    );
    metadata.insert(
        "host_loadavg_total_threads".to_string(),
        system.loadavg_total_threads,
    );
    metadata.insert("host_loadavg_last_pid".to_string(), system.loadavg_last_pid);
}

fn format_percent_millis(value: u64) -> String {
    format!("{}.{:03}", value / 1000, value % 1000)
}
