//! Reusable TLS payload capture runtime.

mod assembly;
mod config;
mod ebpf;
mod event;
mod ring_stats;
mod segment;
mod session;
mod target;

pub(crate) use assembly::{
    AssembledHttp, HttpAssembler, HttpAssemblyOutput, HttpBody, HttpBodyFragment,
    HttpBodyFragmentBody, HttpDecodeConfig, SseAssembler, SseFrame, SseFrameEvent,
    decoded_text_from_headers,
};
pub(crate) use config::{
    CaptureConfig, DEFAULT_ASSEMBLE_BUFFER_BYTES, DEFAULT_DECODE_INPUT_BYTES,
    DEFAULT_DECODE_OUTPUT_BYTES, DEFAULT_DECODE_READER_BUFFER_BYTES, DEFAULT_DRAIN_MILLIS,
    DEFAULT_MATCH_LIMIT, DEFAULT_MAX_CAPTURE_BYTES, DEFAULT_PENDING_OPS, DEFAULT_POLL_MILLIS,
    DEFAULT_RING_BUFFER_BYTES, DEFAULT_RUSTLS_CHUNKS,
};
pub(crate) use event::{CaptureDirection, CaptureEvent};
pub(crate) use ring_stats::{RingStatsCollector, RingStatsSnapshot};
pub(crate) use session::ProbeSession;
