//! Container identity resolved from a process's cgroup.
//!
//! `container_id` is the runtime-assigned, human-readable, stable handle for a
//! container. It is 1:1 with the container's pid namespace, but unlike the
//! kernel `NamespaceIdentity` (an opaque, reuse-prone inode) it maps to
//! `docker ps` / image / pod and survives collector restarts.
//!
//! The initial resolver supports Docker. The struct keeps `runtime` and `pod_uid` so
//! containerd / Podman / CRI-O / K8s can be added without changing the model.

/// Which container runtime a [`ContainerIdentity`] was parsed from.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ContainerRuntime {
    Docker,
    Containerd,
    Podman,
    CriO,
    K8s,
    Unknown,
}

/// Readable, runtime-assigned container identity.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ContainerIdentity {
    pub runtime: ContainerRuntime,
    pub container_id: String,
    /// K8s pod UID; `None` for plain Docker.
    pub pod_uid: Option<String>,
}

impl ContainerIdentity {
    pub fn new(runtime: ContainerRuntime, container_id: impl Into<String>) -> Self {
        Self {
            runtime,
            container_id: container_id.into(),
            pod_uid: None,
        }
    }

    pub fn container_id(&self) -> &str {
        &self.container_id
    }
}
