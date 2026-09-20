//! Raw observation-event contracts emitted by collectors.

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::ids::{CollectorName, TraceId};
use model_core::process::ProcessObservation;

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ExecFileIdentity {
    pub device_major: u32,
    pub device_minor: u32,
    pub inode: u64,
    pub size: u64,
    pub mtime_seconds: i64,
    pub ctime_seconds: i64,
    pub mtime_nanoseconds: u32,
    pub ctime_nanoseconds: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawEventEnvelope {
    pub trace_id: Option<TraceId>,
    pub observed_at: SystemTime,
    pub process: ProcessObservation,
    pub collector: CollectorName,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawObservationPayload {
    Process {
        operation: String,
        parent: Option<ProcessObservation>,
        exec_file_identity: Option<Box<ExecFileIdentity>>,
        metadata: BTreeMap<String, String>,
    },
    File {
        operation: String,
        path: Option<String>,
        metadata: BTreeMap<String, String>,
    },
    FileIoSummary {
        path: Option<String>,
        summary: model_core::event::FileIoSummary,
    },
    Net {
        transport: String,
        local: Option<String>,
        remote: Option<String>,
        size: Option<u64>,
        result: Option<i32>,
        metadata: BTreeMap<String, String>,
    },
    Ipc {
        channel: String,
        peer: Option<String>,
        metadata: BTreeMap<String, String>,
    },
    Stdio {
        stream: String,
        bytes: Vec<u8>,
        metadata: BTreeMap<String, String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawCollectorEvent {
    pub envelope: RawEventEnvelope,
    pub payload: RawObservationPayload,
}
