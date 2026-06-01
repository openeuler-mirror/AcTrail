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
    PolicyPluginHost,
    PolicyDecisionRecord,
    EnforcementFilePermissionFanotify,
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
