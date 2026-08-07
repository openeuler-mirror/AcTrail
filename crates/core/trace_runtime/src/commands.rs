//! Track-add, track-remove, and lifecycle command ownership.

use std::collections::BTreeSet;
use std::time::SystemTime;

use config_core::trace_snapshot::CaptureProfileSnapshot;
use model_core::ids::{TraceId, TraceName};
use model_core::process::{NamespaceIdentity, ProcessIdentity};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackTraceRequest {
    pub root_identity: ProcessIdentity,
    pub root_pid_namespace: Option<NamespaceIdentity>,
    /// Container id of the root process, resolved host-side at attach.
    /// `None` = host process or non-Docker runtime.
    pub root_container_id: Option<String>,
    /// Kubernetes pod UID of the root container (OTel `k8s.pod.uid`), parsed
    /// from the same cgroup as `root_container_id`. `None` = not a k8s pod.
    pub root_pod_uid: Option<String>,
    /// Host/VM id (OTel `host.id`) stamped by the daemon onto every trace.
    /// Daemon-wide constant resolved once at startup.
    pub root_host_id: Option<String>,
    pub root_working_directory: Option<String>,
    pub display_name: TraceName,
    pub profile_snapshot: CaptureProfileSnapshot,
    pub tags: BTreeSet<String>,
    pub created_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootRemovalRequest {
    pub trace_id: TraceId,
    pub removed_at: SystemTime,
}
