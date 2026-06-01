//! Raw observation-event contracts emitted by collectors.

use std::collections::BTreeMap;
use std::time::SystemTime;

use model_core::ids::CollectorName;
use model_core::process::ProcessIdentity;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawEventEnvelope {
    pub observed_at: SystemTime,
    pub process: ProcessIdentity,
    pub collector: CollectorName,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawObservationPayload {
    Process {
        operation: String,
        parent: Option<ProcessIdentity>,
        metadata: BTreeMap<String, String>,
    },
    File {
        operation: String,
        path: Option<String>,
        metadata: BTreeMap<String, String>,
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
