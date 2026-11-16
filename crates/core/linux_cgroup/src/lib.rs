//! Narrow filesystem adapters for cgroup resource scopes: shared types and
//! helpers in `common`, one module per cgroup version.

mod common;
mod v1;
mod v2;

pub use common::{
    CgroupCounters, CgroupError, CpuStat, IoStat, ManagedScopeDirectory, ManagedScopeScan,
    MemoryEvents, TraceScopePaths, read_cgroup_subtree_pids,
};
pub use v1::{
    CgroupV1CounterReader, ManagedCgroupV1, V1_OPTIONAL_CONTROLLERS, V1_REQUIRED_CONTROLLERS,
    V1Hierarchy, V1PreflightReport, parse_blkio_service_bytes, parse_v1_process_cgroup,
};
pub use v2::{
    CgroupPreflightReport, CgroupV2CounterReader, ManagedCgroupV2, OPTIONAL_CONTROLLERS,
    REQUIRED_CONTROLLERS, SYSTEMD_DELEGATION_GUIDANCE, UnifiedHierarchy,
    discover_unified_hierarchy, discover_unified_hierarchy_from, parse_cgroup2_mount,
    parse_cgroup2_mount_id, parse_cpu_stat, parse_io_stat, parse_memory_events, parse_populated,
    parse_unified_process_cgroup, resolve_process_cgroup_path,
};
