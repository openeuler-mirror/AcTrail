//! Capture runtime configuration.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use tls_probe_point_finder::fast::{ArchFilter, ProviderFilter, SourceFilter};

pub(crate) const DEFAULT_MAX_CAPTURE_BYTES: usize = 65_535;
pub(crate) const ABI_MAX_CAPTURE_BYTES: usize = 65_535;
pub(crate) const DEFAULT_RING_BUFFER_BYTES: u32 = 4_194_304;
pub(crate) const DEFAULT_PENDING_OPS: u32 = 4_096;
pub(crate) const DEFAULT_MATCH_LIMIT: usize = 8;
pub(crate) const DEFAULT_RUSTLS_CHUNKS: u32 = 8;
pub(crate) const ABI_MAX_RUSTLS_CHUNKS: u32 = 8;
pub(crate) const DEFAULT_POLL_MILLIS: u64 = 100;
pub(crate) const DEFAULT_DRAIN_MILLIS: u64 = 2_000;
pub(crate) const BPF_EVENT_HEADER_BYTES: u32 = 72;
pub(crate) const DEFAULT_ASSEMBLE_BUFFER_BYTES: usize = 4_194_304;
pub(crate) const DEFAULT_DECODE_INPUT_BYTES: usize = 1_048_576;
pub(crate) const DEFAULT_DECODE_OUTPUT_BYTES: usize = 4_194_304;
pub(crate) const DEFAULT_DECODE_READER_BUFFER_BYTES: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureConfig {
    pub(crate) command: Vec<OsString>,
    pub(crate) max_capture_bytes: usize,
    pub(crate) ring_buffer_bytes: u32,
    pub(crate) pending_ops: u32,
    pub(crate) match_limit: usize,
    pub(crate) rustls_chunks: u32,
    pub(crate) poll_interval: Duration,
    pub(crate) drain_after_exit: Duration,
    pub(crate) assemble_buffer_bytes: usize,
    pub(crate) decode_input_bytes: usize,
    pub(crate) decode_output_bytes: usize,
    pub(crate) decode_reader_buffer_bytes: usize,
    pub(crate) arch: ArchFilter,
    pub(crate) provider: ProviderFilter,
    pub(crate) source: SourceFilter,
    pub(crate) libraries: Vec<PathBuf>,
    pub(crate) library_search_dirs: Vec<PathBuf>,
}
