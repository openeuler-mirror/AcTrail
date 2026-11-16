use super::super::sampler::CgroupCounterSource;
use crate::ProfilingError;
use crate::model::ResourceAccountingMethod;
use linux_cgroup::{CgroupError, ManagedScopeDirectory, ManagedScopeScan, TraceScopePaths};
use std::path::Path;

/// The filesystem operations one cgroup version provides to the managed
/// scope lifecycle. Implemented under `implementations/<version>/managed/`.
pub(crate) trait ManagedHierarchy: Sized {
    type Config;
    type Reader: CgroupCounterSource;

    const ACCOUNTING_METHOD: ResourceAccountingMethod;
    /// `cpu_rate_basis` metadata of periodic samples.
    const CPU_RATE_BASIS: &'static str;
    /// Metadata key set when the optional I/O counter file is missing.
    const MISSING_IO_FILE_KEY: &'static str;
    /// Fixed metadata describing how this version's counters are measured.
    const SAMPLE_METADATA: &'static [(&'static str, &'static str)];

    /// Discovers and prepares the hierarchy. An "unavailable" error lets the
    /// next version be tried.
    fn prepare(config: &Self::Config) -> Result<Self, ProfilingError>;
    fn recovery_scan_limit(config: &Self::Config) -> u32;
    fn finalization_timeout_ms(config: &Self::Config) -> u64;
    /// Controllers enabled for trace scopes, for diagnostics.
    fn controllers(&self) -> &[String];

    fn trace_paths(&self, trace_id: u64, nonce: &str) -> Result<TraceScopePaths, CgroupError>;
    fn trace_paths_from_relative(
        &self,
        relative_path: &Path,
    ) -> Result<ManagedScopeDirectory, CgroupError>;
    fn scan_managed_scopes(&self, limit: usize) -> Result<ManagedScopeScan, CgroupError>;
    fn recovery_scope_complete(&self, _paths: &TraceScopePaths) -> Result<bool, CgroupError> {
        Ok(true)
    }
    fn create_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError>;
    /// Hierarchies whose controllers are attached per mount have nothing to enable.
    fn enable_trace_controllers(&self, _paths: &TraceScopePaths) -> Result<(), CgroupError> {
        Ok(())
    }
    fn move_pid(&self, paths: &TraceScopePaths, pid: u32) -> Result<(), CgroupError>;
    fn verify_pid_membership(&self, pid: u32, paths: &TraceScopePaths) -> Result<(), CgroupError>;
    fn subtree_is_empty(&self, aggregate: &Path) -> Result<bool, CgroupError>;
    fn read_populated(&self, aggregate: &Path) -> Result<bool, CgroupError>;
    fn counter_reader(&self, aggregate: &Path) -> Result<Self::Reader, CgroupError>;
    fn remove_empty_trace_scope(&self, paths: &TraceScopePaths) -> Result<(), CgroupError>;
}
