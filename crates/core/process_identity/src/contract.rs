use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NamespaceIdentity(String);

impl NamespaceIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessIdentity(u64);

impl ProcessIdentity {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ProcessIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "process-{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HostProcessCoordinates {
    pub pid: u32,
    pub task_id: Option<u32>,
    pub start_time_ticks: u64,
    pub start_boottime_ns: Option<u64>,
}

impl HostProcessCoordinates {
    pub const fn new(pid: u32, start_time_ticks: u64) -> Self {
        Self {
            pid,
            task_id: None,
            start_time_ticks,
            start_boottime_ns: None,
        }
    }

    pub const fn with_task_id(mut self, task_id: u32) -> Self {
        self.task_id = Some(task_id);
        self
    }

    pub const fn with_start_boottime_ns(mut self, start_boottime_ns: u64) -> Self {
        self.start_boottime_ns = Some(start_boottime_ns);
        self
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NamespaceProcessCoordinates {
    pub pid_namespace: NamespaceIdentity,
    pub pid: u32,
    pub start_time_ticks: u64,
}

impl NamespaceProcessCoordinates {
    pub fn new(pid_namespace: NamespaceIdentity, pid: u32, start_time_ticks: u64) -> Self {
        Self {
            pid_namespace,
            pid,
            start_time_ticks,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProcessObservation {
    pub host: Option<HostProcessCoordinates>,
    pub namespace: Option<NamespaceProcessCoordinates>,
}

impl ProcessObservation {
    pub fn host(host: HostProcessCoordinates) -> Self {
        Self {
            host: Some(host),
            namespace: None,
        }
    }

    pub fn namespace(namespace: NamespaceProcessCoordinates) -> Self {
        Self {
            host: None,
            namespace: Some(namespace),
        }
    }

    pub fn with_namespace(mut self, namespace: NamespaceProcessCoordinates) -> Self {
        self.namespace = Some(namespace);
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessResolutionState {
    Provisional,
    Resolved,
    Conflicted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessRecord {
    pub identity: ProcessIdentity,
    pub host: Option<HostProcessCoordinates>,
    pub namespaces: BTreeSet<NamespaceProcessCoordinates>,
    pub resolution_state: ProcessResolutionState,
}

impl ProcessRecord {
    pub fn new(identity: ProcessIdentity, observation: ProcessObservation) -> Self {
        let namespaces = observation.namespace.into_iter().collect();
        let resolution_state = if observation.host.is_some() {
            ProcessResolutionState::Resolved
        } else {
            ProcessResolutionState::Provisional
        };
        Self {
            identity,
            host: observation.host,
            namespaces,
            resolution_state,
        }
    }

    pub fn observation(&self) -> ProcessObservation {
        ProcessObservation {
            host: self.host.clone(),
            namespace: self.namespaces.iter().next().cloned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuppressedFdPurpose {
    TlsSyncEvent,
    InternalUpload,
    InternalControl,
}

impl SuppressedFdPurpose {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TlsSyncEvent => "tls-sync-event",
            Self::InternalUpload => "internal-upload",
            Self::InternalControl => "internal-control",
        }
    }
}

impl std::str::FromStr for SuppressedFdPurpose {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "tls-sync-event" => Ok(Self::TlsSyncEvent),
            "internal-upload" => Ok(Self::InternalUpload),
            "internal-control" => Ok(Self::InternalControl),
            _ => Err(format!("unknown suppressed fd purpose {value}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialSuppressedFd {
    pub fd: i32,
    pub purpose: SuppressedFdPurpose,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KernelProcessCoordinates {
    pub pid: u32,
    pub start_time: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessSuppressedFd {
    pub process: KernelProcessCoordinates,
    pub fd: i32,
    pub purpose: SuppressedFdPurpose,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityLookupError {
    NotFound { pid: u32 },
    PermissionDenied { pid: u32 },
    Incomplete { pid: u32, detail: String },
}

pub trait ProcessIdentityReader {
    fn read_identity(&self, pid: u32) -> Result<ProcessObservation, IdentityLookupError>;
}
