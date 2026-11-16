use super::{ManagedV1Preflight, ManagedV1ScopeConfig};
use crate::ProfilingError;
use crate::collection::workload::cgroup::common::cgroup_error;
use crate::collection::workload::cgroup::common::managed::ManagedHierarchy;
use crate::collection::workload::cgroup::common::sampler::CgroupCounterSource;
use crate::model::ResourceAccountingMethod;
use linux_cgroup::{
    CgroupCounters, CgroupError, CgroupV1CounterReader, ManagedCgroupV1, ManagedScopeDirectory,
    ManagedScopeScan, TraceScopePaths,
};
use std::path::Path;

/// Managed trace scopes replicated under `<mount>/<root_name>/traces` in each
/// selected cgroup v1 hierarchy.
pub(crate) struct ManagedV1Hierarchy {
    adapter: ManagedCgroupV1,
    controllers: Vec<String>,
}

impl ManagedHierarchy for ManagedV1Hierarchy {
    type Config = ManagedV1ScopeConfig;
    type Reader = CgroupV1CounterReader;

    const ACCOUNTING_METHOD: ResourceAccountingMethod = ResourceAccountingMethod::CgroupV1;
    const CPU_RATE_BASIS: &'static str = "delta(cpuacct.usage)/monotonic_elapsed_usec";
    const MISSING_IO_FILE_KEY: &'static str =
        "missing_optional_file.blkio.throttle.io_service_bytes_recursive";
    const SAMPLE_METADATA: &'static [(&'static str, &'static str)] = &[
        ("cgroup_version", "v1"),
        ("ownership", "managed"),
        (
            "memory_usage_basis",
            "memory.usage_in_bytes (kernel approximate charged memory)",
        ),
        (
            "memory_stat_basis",
            "hierarchical total_rss/total_cache/total_swap",
        ),
        (
            "io_basis",
            "blkio.throttle.io_service_bytes_recursive; buffered writeback may be charged to the root cgroup",
        ),
    ];

    fn prepare(config: &ManagedV1ScopeConfig) -> Result<Self, ProfilingError> {
        let (adapter, report) =
            ManagedV1Preflight::prepare(config).map_err(cgroup_error("cgroup_preflight"))?;
        tracing::info!(
            roots = ?report.hierarchies.iter().map(|h| h.root.display().to_string()).collect::<Vec<_>>(),
            optional_controllers = ?report.enabled_optional_controllers,
            "cgroup v1 managed hierarchies prepared"
        );
        let controllers = adapter.enabled_controllers().into_iter().collect();
        Ok(Self {
            adapter,
            controllers,
        })
    }
    fn recovery_scan_limit(config: &ManagedV1ScopeConfig) -> u32 {
        config.recovery_scan_limit
    }
    fn finalization_timeout_ms(config: &ManagedV1ScopeConfig) -> u64 {
        config.finalization_timeout_ms
    }
    fn controllers(&self) -> &[String] {
        &self.controllers
    }

    fn trace_paths(&self, trace_id: u64, nonce: &str) -> Result<TraceScopePaths, CgroupError> {
        self.adapter.trace_paths(trace_id, nonce)
    }
    fn trace_paths_from_relative(
        &self,
        relative_path: &Path,
    ) -> Result<ManagedScopeDirectory, CgroupError> {
        self.adapter.trace_paths_from_relative(relative_path)
    }
    fn scan_managed_scopes(&self, limit: usize) -> Result<ManagedScopeScan, CgroupError> {
        self.adapter.scan_managed_scopes(limit)
    }
    fn recovery_scope_complete(&self, paths: &TraceScopePaths) -> Result<bool, CgroupError> {
        self.adapter.scope_replicas_present(paths)
    }
    fn create_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.adapter.create_trace_scope(paths)
    }
    fn move_pid(&self, paths: &TraceScopePaths, pid: u32) -> Result<(), CgroupError> {
        self.adapter.move_pid(paths, pid)
    }
    fn verify_pid_membership(&self, pid: u32, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.adapter.verify_pid_membership(pid, paths)
    }
    fn subtree_is_empty(&self, aggregate: &Path) -> Result<bool, CgroupError> {
        self.adapter.subtree_is_empty(aggregate)
    }
    fn read_populated(&self, aggregate: &Path) -> Result<bool, CgroupError> {
        self.adapter.read_populated(aggregate)
    }
    fn counter_reader(&self, aggregate: &Path) -> Result<CgroupV1CounterReader, CgroupError> {
        self.adapter.counter_reader(aggregate)
    }
    fn remove_empty_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.adapter.remove_empty_trace_scope(paths)
    }
}

impl CgroupCounterSource for CgroupV1CounterReader {
    fn read_counters(&mut self) -> Result<CgroupCounters, CgroupError> {
        self.read_counters()
    }
    fn read_subtree_pids(&self) -> Result<Vec<u32>, CgroupError> {
        self.read_subtree_pids()
    }
}
