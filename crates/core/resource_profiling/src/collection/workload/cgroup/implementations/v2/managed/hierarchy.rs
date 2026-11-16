use super::ManagedV2ScopeConfig;
use crate::ProfilingError;
use crate::collection::workload::cgroup::common::cgroup_error;
use crate::collection::workload::cgroup::common::managed::ManagedHierarchy;
use crate::collection::workload::cgroup::common::sampler::CgroupCounterSource;
use crate::model::ResourceAccountingMethod;
use linux_cgroup::{
    CgroupCounters, CgroupError, CgroupPreflightReport, CgroupV2CounterReader, ManagedCgroupV2,
    ManagedScopeDirectory, ManagedScopeScan, TraceScopePaths, discover_unified_hierarchy,
};
use std::path::Path;

/// Managed trace scopes under the systemd-delegated cgroup v2 root.
pub(crate) struct ManagedV2Hierarchy {
    adapter: ManagedCgroupV2,
    preflight: CgroupPreflightReport,
    controllers: Vec<String>,
}

impl ManagedHierarchy for ManagedV2Hierarchy {
    type Config = ManagedV2ScopeConfig;
    type Reader = CgroupV2CounterReader;

    const ACCOUNTING_METHOD: ResourceAccountingMethod = ResourceAccountingMethod::CgroupV2;
    const CPU_RATE_BASIS: &'static str = "delta(cpu.stat.usage_usec)/monotonic_elapsed_usec";
    const MISSING_IO_FILE_KEY: &'static str = "missing_optional_file.io.stat";
    const SAMPLE_METADATA: &'static [(&'static str, &'static str)] = &[];

    fn prepare(config: &ManagedV2ScopeConfig) -> Result<Self, ProfilingError> {
        let hierarchy = discover_unified_hierarchy().map_err(cgroup_error("cgroup_discovery"))?;
        let adapter = ManagedCgroupV2::new(config.root.clone())
            .map_err(cgroup_error("cgroup_configuration"))?;
        let preflight = adapter
            .prepare_hierarchy(hierarchy, std::process::id())
            .map_err(cgroup_error("cgroup_preflight"))?;
        tracing::info!(
            managed_root = %preflight.managed_root.display(),
            owner_uid = preflight.owner_uid,
            owner_gid = preflight.owner_gid,
            "cgroup v2 managed hierarchy prepared"
        );
        let controllers = preflight.enabled_controllers.iter().cloned().collect();
        Ok(Self {
            adapter,
            preflight,
            controllers,
        })
    }
    fn recovery_scan_limit(config: &ManagedV2ScopeConfig) -> u32 {
        config.recovery_scan_limit
    }
    fn finalization_timeout_ms(config: &ManagedV2ScopeConfig) -> u64 {
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
    fn create_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.adapter.create_trace_scope(paths)
    }
    fn enable_trace_controllers(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        let controllers = self
            .controllers
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        self.adapter.enable_trace_controllers(paths, &controllers)
    }
    fn move_pid(&self, paths: &TraceScopePaths, pid: u32) -> Result<(), CgroupError> {
        self.adapter.move_pid(&paths.workload, pid)
    }
    fn verify_pid_membership(&self, pid: u32, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.adapter
            .verify_pid_membership(pid, &paths.workload, &self.preflight.hierarchy)
    }
    fn subtree_is_empty(&self, aggregate: &Path) -> Result<bool, CgroupError> {
        self.adapter.subtree_is_empty(aggregate)
    }
    fn read_populated(&self, aggregate: &Path) -> Result<bool, CgroupError> {
        self.adapter.read_populated(aggregate)
    }
    fn counter_reader(&self, aggregate: &Path) -> Result<CgroupV2CounterReader, CgroupError> {
        self.adapter.counter_reader(aggregate)
    }
    fn remove_empty_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError> {
        self.adapter.remove_empty_trace_scope(paths)
    }
}

impl CgroupCounterSource for CgroupV2CounterReader {
    fn read_counters(&mut self) -> Result<CgroupCounters, CgroupError> {
        self.read_counters()
    }
    fn read_subtree_pids(&self) -> Result<Vec<u32>, CgroupError> {
        self.read_subtree_pids()
    }
}
