//! Structured payload events emitted by the capture runtime.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CaptureDirection {
    Inbound,
    Outbound,
}

impl CaptureDirection {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CaptureFlags {
    pub(crate) truncated: bool,
    pub(crate) rustls_chunk: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureEvent {
    pub(crate) pid: u32,
    pub(crate) tid: u32,
    pub(crate) provider: String,
    pub(crate) symbol: String,
    pub(crate) direction: CaptureDirection,
    pub(crate) requested_size: u64,
    pub(crate) observed_ktime_ns: u64,
    pub(crate) stream_key: u64,
    pub(crate) flags: CaptureFlags,
    pub(crate) captured: Vec<u8>,
    pub(crate) ring_captured_sizes: Vec<usize>,
    pub(crate) ring_reserved_sizes: Vec<usize>,
}
