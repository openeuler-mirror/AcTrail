use model_core::{host_identity::HostBootId, ids::TraceId};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::SystemTime};
/// One controller's independently verified directory identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CgroupV1Boundary {
    pub mount_point: PathBuf,
    pub mount_root: PathBuf,
    pub membership: PathBuf,
    pub relative_path: PathBuf,
    pub device: u64,
    pub inode: u64,
}
/// A read-only cgroup v1 binding to the cgroups a process is already in.
///
/// A binding means AcTrail did not create the cgroup (the table name keeps the
/// older "external" wording). The bound cgroups may be a login session, a
/// service or a container, so samples are `broader_than_trace`. The binding is
/// immutable, with a durable stale/closed disposition. Stored in
/// `trace_external_cgroup_v1_bindings`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CgroupV1Binding {
    pub trace_id: TraceId,
    pub host_boot_id: HostBootId,
    pub cpuacct: CgroupV1Boundary,
    pub memory: CgroupV1Boundary,
    pub created_at: SystemTime,
    pub stale_reason: Option<String>,

    pub closed_at: Option<SystemTime>,
}
