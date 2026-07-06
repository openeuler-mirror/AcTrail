//! Command-line input shapes.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use tls_probe_point_finder::fast::{ArchFilter, ProviderFilter, SourceFilter};

use crate::capture::{
    CaptureConfig, DEFAULT_ASSEMBLE_BUFFER_BYTES, DEFAULT_DECODE_INPUT_BYTES,
    DEFAULT_DECODE_OUTPUT_BYTES, DEFAULT_DECODE_READER_BUFFER_BYTES, DEFAULT_DRAIN_MILLIS,
    DEFAULT_MATCH_LIMIT, DEFAULT_MAX_CAPTURE_BYTES, DEFAULT_PENDING_OPS, DEFAULT_POLL_MILLIS,
    DEFAULT_RING_BUFFER_BYTES, DEFAULT_RUSTLS_CHUNKS,
};
use crate::cli::report_config::{EventFilter, RedactionMode, ReportEvent, ReporterConfig};
use crate::{ToolError, ToolResult};

#[derive(Clone, Debug, Parser)]
#[command(
    name = "tls-payload-probe",
    about = "Capture TLS payloads with uprobes"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, Subcommand)]
pub(crate) enum Command {
    #[command(about = "Launch a command and stream captured TLS payloads")]
    Probe(ProbeArgs),
}

#[derive(Clone, Debug, Parser)]
pub(crate) struct ProbeArgs {
    /// Fail if the resolved ELF does not match this architecture.
    #[arg(long, value_enum, default_value = "auto")]
    arch: ArchChoice,

    /// TLS provider to use for fast probe-point resolution.
    #[arg(long, value_enum, default_value = "auto")]
    provider: ProviderChoice,

    /// Probe source to inspect.
    #[arg(long, value_enum, default_value = "auto")]
    source: SourceChoice,

    /// Maximum bytes to read from a single payload event.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_MAX_CAPTURE_BYTES)]
    max_capture_bytes: usize,

    /// eBPF ring buffer capacity in bytes.
    #[arg(long, value_name = "N", value_parser = parse_u32, default_value_t = DEFAULT_RING_BUFFER_BYTES)]
    ring_buffer_bytes: u32,

    /// Maximum pending read operations tracked by BPF.
    #[arg(long, value_name = "N", value_parser = parse_u32, default_value_t = DEFAULT_PENDING_OPS)]
    pending_ops: u32,

    /// Maximum static pattern matches to inspect per pattern.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_MATCH_LIMIT)]
    match_limit: usize,

    /// Maximum rustls outbound chunks inspected in BPF.
    #[arg(long, value_name = "N", value_parser = parse_u32, default_value_t = DEFAULT_RUSTLS_CHUNKS)]
    rustls_chunks: u32,

    /// Trace event polling interval in milliseconds.
    #[arg(long, value_name = "MS", value_parser = parse_u64, default_value_t = DEFAULT_POLL_MILLIS)]
    poll_ms: u64,

    /// Time to keep draining trace events after the target exits.
    #[arg(long, value_name = "MS", value_parser = parse_u64, default_value_t = DEFAULT_DRAIN_MILLIS)]
    drain_ms: u64,

    /// Maximum buffered TLS plaintext bytes per HTTP assembly stream.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_ASSEMBLE_BUFFER_BYTES)]
    assemble_buffer_bytes: usize,

    /// Maximum compressed HTTP body bytes eligible for decoding.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_DECODE_INPUT_BYTES)]
    decode_input_bytes: usize,

    /// Maximum decoded HTTP body bytes to materialize.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_DECODE_OUTPUT_BYTES)]
    decode_output_bytes: usize,

    /// Reader buffer size used by streaming content decoders.
    #[arg(long, value_name = "N", value_parser = parse_usize, default_value_t = DEFAULT_DECODE_READER_BUFFER_BYTES)]
    decode_reader_buffer_bytes: usize,

    /// Payload preview redaction mode.
    #[arg(long, value_enum, default_value = "redact")]
    redaction: RedactionChoice,

    /// Event groups to print. Omit this option to print every group.
    #[arg(long = "events", value_enum, value_delimiter = ',', num_args = 1..)]
    events: Vec<ReportEvent>,

    /// Print ring-buffer utilization and loss counters after target exit.
    #[arg(long)]
    ring_stats: bool,

    /// Explicit libssl path to inspect as a shared-library candidate.
    #[arg(long = "library", value_name = "PATH")]
    libraries: Vec<PathBuf>,

    /// Extra directory to search for DT_NEEDED libssl entries.
    #[arg(long = "library-search-dir", value_name = "DIR")]
    library_search_dirs: Vec<PathBuf>,

    /// Target command. The first item is the agent program.
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<OsString>,
}

impl ProbeArgs {
    pub(crate) fn reporter_config(&self) -> ReporterConfig {
        ReporterConfig {
            redaction: self.redaction.into(),
            events: EventFilter::from_choices(&self.events),
            ring_stats: self.ring_stats,
        }
    }

    pub(crate) fn into_config(self) -> ToolResult<CaptureConfig> {
        if self.command.is_empty() {
            return Err(ToolError::new("probe command is empty"));
        }
        Ok(CaptureConfig {
            command: self.command,
            max_capture_bytes: self.max_capture_bytes,
            ring_buffer_bytes: self.ring_buffer_bytes,
            pending_ops: self.pending_ops,
            match_limit: self.match_limit,
            rustls_chunks: self.rustls_chunks,
            poll_interval: Duration::from_millis(self.poll_ms),
            drain_after_exit: Duration::from_millis(self.drain_ms),
            assemble_buffer_bytes: self.assemble_buffer_bytes,
            decode_input_bytes: self.decode_input_bytes,
            decode_output_bytes: self.decode_output_bytes,
            decode_reader_buffer_bytes: self.decode_reader_buffer_bytes,
            arch: self.arch.into(),
            provider: self.provider.into(),
            source: self.source.into(),
            libraries: self.libraries,
            library_search_dirs: self.library_search_dirs,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ArchChoice {
    Auto,
    Aarch64,
    #[value(name = "x86_64")]
    X86_64,
}

impl From<ArchChoice> for ArchFilter {
    fn from(choice: ArchChoice) -> Self {
        match choice {
            ArchChoice::Auto => Self::Auto,
            ArchChoice::Aarch64 => Self::Aarch64,
            ArchChoice::X86_64 => Self::X86_64,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ProviderChoice {
    Auto,
    #[value(name = "openssl")]
    OpenSsl,
    #[value(name = "boringssl")]
    BoringSsl,
    Rustls,
    #[value(name = "gnutls")]
    GnuTls,
    Nss,
}

impl From<ProviderChoice> for ProviderFilter {
    fn from(choice: ProviderChoice) -> Self {
        match choice {
            ProviderChoice::Auto => Self::Auto,
            ProviderChoice::OpenSsl => Self::OpenSsl,
            ProviderChoice::BoringSsl => Self::BoringSsl,
            ProviderChoice::Rustls => Self::Rustls,
            ProviderChoice::GnuTls => Self::GnuTls,
            ProviderChoice::Nss => Self::Nss,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SourceChoice {
    Auto,
    Executable,
    #[value(name = "shared-library")]
    SharedLibrary,
}

impl From<SourceChoice> for SourceFilter {
    fn from(choice: SourceChoice) -> Self {
        match choice {
            SourceChoice::Auto => Self::Auto,
            SourceChoice::Executable => Self::Executable,
            SourceChoice::SharedLibrary => Self::SharedLibrary,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum RedactionChoice {
    Redact,
    None,
}

impl From<RedactionChoice> for RedactionMode {
    fn from(choice: RedactionChoice) -> Self {
        match choice {
            RedactionChoice::Redact => Self::Redact,
            RedactionChoice::None => Self::None,
        }
    }
}

pub(crate) fn try_parse_args() -> Result<Command, clap::Error> {
    Cli::try_parse().map(|cli| cli.command)
}

fn parse_usize(value: &str) -> Result<usize, String> {
    usize::try_from(parse_u64(value)?).map_err(|_| format!("integer value is too large: {value}"))
}

fn parse_u32(value: &str) -> Result<u32, String> {
    u32::try_from(parse_u64(value)?).map_err(|_| format!("integer value is too large: {value}"))
}

fn parse_u64(value: &str) -> Result<u64, String> {
    if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse()
    }
    .map_err(|error| format!("invalid integer {value}: {error}"))
}
