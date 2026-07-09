use tls_payload_core::PayloadDirection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FlowControlConfig {
    pub(in crate::runtime) enabled: bool,
    pub(in crate::runtime) sniff_bytes: usize,
    pub(in crate::runtime) max_header_bytes: usize,
    pub(in crate::runtime) large_transfer_bytes: u64,
    pub(in crate::runtime) unknown_stream_bytes: u64,
    pub(in crate::runtime) h2_data_probe_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FlowDecision {
    EmitPayload,
    EmitSummary(FlowSummary),
    EmitMany(Vec<FlowEmission>),
    DropBody,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FlowEmission {
    Payload(Vec<u8>),
    Summary(FlowSummary),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FlowSummary {
    pub(in crate::runtime) observed_size: u64,
    pub(in crate::runtime) reason: &'static str,
    pub(in crate::runtime) protocol_hint: &'static str,
    pub(in crate::runtime) bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct FlowKey {
    pub(super) stream_key: usize,
    pub(super) direction: FlowDirection,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum FlowDirection {
    Outbound,
    Inbound,
}

impl From<PayloadDirection> for FlowDirection {
    fn from(value: PayloadDirection) -> Self {
        match value {
            PayloadDirection::Outbound => Self::Outbound,
            PayloadDirection::Inbound => Self::Inbound,
        }
    }
}
