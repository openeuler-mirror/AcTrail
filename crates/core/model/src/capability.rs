//! Capability requests, collector guarantees, and sensor declarations.

use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    ProcLifecycle,
    ProcExecContext,
    FsAccessBasic,
    FsMmap,
    FsExecAccess,
    NetTransport,
    NetDns,
    NetTlsMetadata,
    NetProviderClassification,
    NetApplicationPlaintextHttp,
    NetApplicationHttp2Frames,
    NetApplicationPlaintextWs,
    TlsPlaintextPayload,
    SocketPlaintextPayload,
    ResourceMetrics,
    IpcUnixSocket,
    IpcPipeFifo,
    StdioChunk,
    PolicyIngestProcessing,
    PolicyDecisionRecord,
    EnforcementFilePermissionFanotify,
}

impl Capability {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ProcLifecycle => "proc-lifecycle",
            Self::ProcExecContext => "proc-exec-context",
            Self::FsAccessBasic => "fs-access-basic",
            Self::FsMmap => "fs-mmap",
            Self::FsExecAccess => "fs-exec-access",
            Self::NetTransport => "net-transport",
            Self::NetDns => "net-dns",
            Self::NetTlsMetadata => "net-tls-metadata",
            Self::NetProviderClassification => "net-provider-classification",
            Self::NetApplicationPlaintextHttp => "net-application-plaintext-http",
            Self::NetApplicationHttp2Frames => "net-application-http2-frames",
            Self::NetApplicationPlaintextWs => "net-application-plaintext-ws",
            Self::TlsPlaintextPayload => "tls-plaintext-payload",
            Self::SocketPlaintextPayload => "socket-plaintext-payload",
            Self::ResourceMetrics => "resource-metrics",
            Self::IpcUnixSocket => "ipc-unix-socket",
            Self::IpcPipeFifo => "ipc-pipe-fifo",
            Self::StdioChunk => "stdio-chunk",
            Self::PolicyIngestProcessing => "policy-ingest-processing",
            Self::PolicyDecisionRecord => "policy-decision-record",
            Self::EnforcementFilePermissionFanotify => {
                "enforcement-file-permission-fanotify"
            }
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "proc-lifecycle" => Ok(Self::ProcLifecycle),
            "proc-exec-context" => Ok(Self::ProcExecContext),
            "fs-access-basic" => Ok(Self::FsAccessBasic),
            "fs-mmap" => Ok(Self::FsMmap),
            "fs-exec-access" => Ok(Self::FsExecAccess),
            "net-transport" => Ok(Self::NetTransport),
            "net-dns" => Ok(Self::NetDns),
            "net-tls-metadata" => Ok(Self::NetTlsMetadata),
            "net-provider-classification" => Ok(Self::NetProviderClassification),
            "net-application-plaintext-http" => Ok(Self::NetApplicationPlaintextHttp),
            "net-application-http2-frames" => Ok(Self::NetApplicationHttp2Frames),
            "net-application-plaintext-ws" => Ok(Self::NetApplicationPlaintextWs),
            "tls-plaintext-payload" => Ok(Self::TlsPlaintextPayload),
            "socket-plaintext-payload" => Ok(Self::SocketPlaintextPayload),
            "resource-metrics" => Ok(Self::ResourceMetrics),
            "ipc-unix-socket" => Ok(Self::IpcUnixSocket),
            "ipc-pipe-fifo" => Ok(Self::IpcPipeFifo),
            "stdio-chunk" => Ok(Self::StdioChunk),
            "policy-ingest-processing" => Ok(Self::PolicyIngestProcessing),
            "policy-decision-record" => Ok(Self::PolicyDecisionRecord),
            "enforcement-file-permission-fanotify" => {
                Ok(Self::EnforcementFilePermissionFanotify)
            }
            other => Err(format!("unknown capability {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RequestMode {
    Required,
    Opportunistic,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GuaranteeClass {
    GuaranteedByTransportCollector,
    AvailableWhenMetadataObservable,
    RequiresPayloadCollector,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityRequest {
    pub capability: Capability,
    pub mode: RequestMode,
}

impl CapabilityRequest {
    pub fn new(capability: Capability, mode: RequestMode) -> Self {
        Self { capability, mode }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityField {
    pub name: String,
    pub guarantee: GuaranteeClass,
    pub note: Option<String>,
}

impl CapabilityField {
    pub fn new(name: impl Into<String>, guarantee: GuaranteeClass) -> Self {
        Self {
            name: name.into(),
            guarantee,
            note: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityDescriptor {
    pub capability: Capability,
    pub fields: Vec<CapabilityField>,
}

impl CapabilityDescriptor {
    pub fn new(capability: Capability, fields: Vec<CapabilityField>) -> Self {
        Self { capability, fields }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet {
    values: BTreeSet<Capability>,
}

impl CapabilitySet {
    pub fn new(values: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub fn contains(&self, capability: &Capability) -> bool {
        self.values.contains(capability)
    }

    pub fn insert(&mut self, capability: Capability) {
        self.values.insert(capability);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.values.iter()
    }

    pub fn missing_from<'a>(
        &'a self,
        requested: &'a [CapabilityRequest],
    ) -> impl Iterator<Item = &'a CapabilityRequest> {
        requested
            .iter()
            .filter(|request| !self.contains(&request.capability))
    }
}
