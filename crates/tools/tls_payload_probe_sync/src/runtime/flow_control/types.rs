use tls_payload_core::PayloadDirection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FlowControlConfig {
    pub(in crate::runtime) enabled: bool,
    pub(in crate::runtime) sniff_bytes: usize,
    pub(in crate::runtime) max_header_bytes: usize,
    pub(in crate::runtime) large_transfer_bytes: u64,
    pub(in crate::runtime) unknown_stream_bytes: u64,
    pub(in crate::runtime) h2_data_probe_bytes: u64,
    pub(in crate::runtime) max_streams: usize,
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub(super) enum FlowScope {
    /// Whole TLS connection. Used for HTTP/1 and unknown/connection-level state.
    Connection,

    /// HTTP/2 logical stream, so binary/large data on one stream does not
    /// poison the rest of the multiplexed connection.
    Http2Stream { stream_id: u32 },

    /// HTTP/1 message scope, reserved for per-message isolation.
    #[allow(dead_code)]
    Http1Message { message_id: u64 },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub(in crate::runtime) struct FlowKey {
    pub(super) stream_key: usize,
    pub(super) direction: FlowDirection,
    /// Connection generation. Currently always 0 until TLS-sync tracks
    /// SSL object lifetime; protects against future pointer reuse.
    pub(super) generation: u32,
    pub(super) scope: FlowScope,
}

impl FlowKey {
    pub(super) fn connection(stream_key: usize, direction: FlowDirection, generation: u32) -> Self {
        Self {
            stream_key,
            direction,
            generation,
            scope: FlowScope::Connection,
        }
    }

    pub(super) fn http2_stream(
        stream_key: usize,
        direction: FlowDirection,
        generation: u32,
        stream_id: u32,
    ) -> Self {
        Self {
            stream_key,
            direction,
            generation,
            scope: FlowScope::Http2Stream { stream_id },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
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
