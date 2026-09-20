//! TLS runtime event models.

use tls_payload_core::PayloadDirection;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadEvent {
    pub trace_id: u64,
    pub pid: u32,
    pub start_time_ticks: u64,
    pub pid_namespace: String,
    pub direction: PayloadDirection,
    pub provider: String,
    pub symbol: String,
    pub stream_key: u64,
    pub sequence: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionEvent {
    pub trace_id: u64,
    pub pid: u32,
    pub start_time_ticks: u64,
    pub pid_namespace: String,
    pub direction: PayloadDirection,
    pub provider: String,
    pub symbol: String,
    pub stream_key: u64,
    pub sequence: u64,
    pub action: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SummaryEvent {
    pub trace_id: u64,
    pub pid: u32,
    pub start_time_ticks: u64,
    pub pid_namespace: String,
    pub direction: PayloadDirection,
    pub provider: String,
    pub symbol: String,
    pub stream_key: u64,
    pub sequence: u64,
    pub observed_size: u64,
    pub emitted_size: u64,
    pub reason: String,
    pub protocol_hint: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncEvent {
    Payload(PayloadEvent),
    Decision(DecisionEvent),
    Summary(SummaryEvent),
}
