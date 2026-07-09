//! Payload capture configuration.

use std::path::PathBuf;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTlsLibrary {
    Auto,
    Openssl,
    Boringssl,
    Rustls,
    Go,
    Gnutls,
    Nss,
}

impl FromStr for PayloadTlsLibrary {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "openssl" => Ok(Self::Openssl),
            "boringssl" => Ok(Self::Boringssl),
            "rustls" => Ok(Self::Rustls),
            "go" => Ok(Self::Go),
            "gnutls" => Ok(Self::Gnutls),
            "nss" => Ok(Self::Nss),
            other => Err(format!("unsupported payload TLS library {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTlsSource {
    Auto,
    SharedLibrary,
    Executable,
}

impl FromStr for PayloadTlsSource {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "shared-library" => Ok(Self::SharedLibrary),
            "executable" => Ok(Self::Executable),
            other => Err(format!("unsupported payload TLS source {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTlsResolver {
    Auto,
    OpensslSymbols,
    BoringsslPatterns,
    BunStaticBoringssl,
    BoringsslStatic,
    RustlsSymbolMap,
    GoPclntab,
    GnutlsSymbols,
    NssNsprSymbols,
}

impl FromStr for PayloadTlsResolver {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "openssl-symbols" => Ok(Self::OpensslSymbols),
            "boringssl-patterns" => Ok(Self::BoringsslPatterns),
            "bun-static-boringssl" => Ok(Self::BunStaticBoringssl),
            "boringssl-static" => Ok(Self::BoringsslStatic),
            "rustls-symbol-map" => Ok(Self::RustlsSymbolMap),
            "go-pclntab" => Ok(Self::GoPclntab),
            "gnutls-symbols" => Ok(Self::GnutlsSymbols),
            "nss-nspr-symbols" => Ok(Self::NssNsprSymbols),
            other => Err(format!("unsupported payload TLS resolver {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTlsCaptureBackend {
    SeccompUserRead,
    BpfCopySeccompFallback,
    TlsSync,
}

impl FromStr for PayloadTlsCaptureBackend {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "seccomp-user-read" => Ok(Self::SeccompUserRead),
            "bpf-copy-seccomp-fallback" => Ok(Self::BpfCopySeccompFallback),
            "tls-sync" => Ok(Self::TlsSync),
            other => Err(format!("unsupported payload TLS capture backend {other}")),
        }
    }
}

impl PayloadTlsCaptureBackend {
    pub const fn requires_seccomp_notify(self) -> bool {
        matches!(self, Self::SeccompUserRead | Self::BpfCopySeccompFallback)
    }

    pub const fn is_sync(self) -> bool {
        matches!(self, Self::TlsSync)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadSocketCaptureBackend {
    BpfCopy,
    BpfCopySeccompFallback,
}

impl FromStr for PayloadSocketCaptureBackend {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "bpf-copy" => Ok(Self::BpfCopy),
            "bpf-copy-seccomp-fallback" => Ok(Self::BpfCopySeccompFallback),
            other => Err(format!(
                "unsupported payload socket capture backend {other}"
            )),
        }
    }
}

impl PayloadSocketCaptureBackend {
    pub const fn requires_seccomp_notify(self) -> bool {
        matches!(self, Self::BpfCopySeccompFallback)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PayloadTlsSeccompSyscall {
    Write,
    Writev,
    Sendto,
    Sendmsg,
}

impl FromStr for PayloadTlsSeccompSyscall {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "write" => Ok(Self::Write),
            "writev" => Ok(Self::Writev),
            "sendto" => Ok(Self::Sendto),
            "sendmsg" => Ok(Self::Sendmsg),
            other => Err(format!("unsupported payload TLS seccomp syscall {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PayloadSocketSeccompSyscall {
    Write,
    Writev,
    Sendto,
    Sendmsg,
}

impl FromStr for PayloadSocketSeccompSyscall {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "write" => Ok(Self::Write),
            "writev" => Ok(Self::Writev),
            "sendto" => Ok(Self::Sendto),
            "sendmsg" => Ok(Self::Sendmsg),
            other => Err(format!(
                "unsupported payload socket seccomp syscall {other}; supported: write, writev, sendto, sendmsg"
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PayloadTlsLibraryPath {
    Auto,
    Path(PathBuf),
}

impl FromStr for PayloadTlsLibraryPath {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "auto" {
            return Ok(Self::Auto);
        }
        if value.is_empty() {
            return Err("payload TLS library path must not be empty".to_string());
        }
        Ok(Self::Path(PathBuf::from(value)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PayloadTlsSyncRuntimeLibraryPath {
    Auto,
    Path(PathBuf),
}

impl FromStr for PayloadTlsSyncRuntimeLibraryPath {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "auto" {
            return Ok(Self::Auto);
        }
        if value.is_empty() {
            return Err("payload TLS sync runtime library path must not be empty".to_string());
        }
        Ok(Self::Path(PathBuf::from(value)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisabledOrPath {
    Disabled,
    Path(PathBuf),
}

impl FromStr for DisabledOrPath {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "disabled" {
            return Ok(Self::Disabled);
        }
        if value.is_empty() {
            return Err("path value must not be empty".to_string());
        }
        Ok(Self::Path(PathBuf::from(value)))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadRedactionPolicy {
    Disabled,
    AuthorizationHeader,
}

impl FromStr for PayloadRedactionPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "authorization-header" => Ok(Self::AuthorizationHeader),
            other => Err(format!("unsupported payload redaction policy {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadStdioStorageMode {
    Full,
    MetadataOnly,
    Drop,
}

impl FromStr for PayloadStdioStorageMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "full" => Ok(Self::Full),
            "metadata-only" => Ok(Self::MetadataOnly),
            "drop" => Ok(Self::Drop),
            other => Err(format!("unsupported payload stdio storage mode {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadTlsConfig {
    pub enabled: bool,
    pub capture_backend: PayloadTlsCaptureBackend,
    pub source: PayloadTlsSource,
    pub resolver: PayloadTlsResolver,
    pub library: PayloadTlsLibrary,
    pub library_path: PayloadTlsLibraryPath,
    pub binary_path: DisabledOrPath,
    pub pattern_path: DisabledOrPath,
    pub max_segment_bytes: u32,
    pub max_operation_bytes: u32,
    pub ring_buffer_bytes: u32,
    pub pending_operation_max_entries: u32,
    pub seccomp_syscalls: Vec<PayloadTlsSeccompSyscall>,
    pub diagnostics_enabled: bool,
    pub retention_max_bytes_per_trace: u64,
    pub redaction_policy: PayloadRedactionPolicy,
    pub sync_runtime_library_path: PayloadTlsSyncRuntimeLibraryPath,
    pub sync_event_socket_path: PathBuf,
    pub sync_socket_mode: u32,
    pub sync_match_limit: u32,
    pub sync_flow_control_enabled: bool,
    pub sync_flow_sniff_bytes: u32,
    pub sync_flow_max_header_bytes: u32,
    pub sync_flow_large_transfer_bytes: u64,
    pub sync_flow_unknown_stream_bytes: u64,
    pub sync_flow_h2_data_probe_bytes: u64,
    pub java_agent_enabled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadStdioConfig {
    pub enabled: bool,
    pub capture_stdin: bool,
    pub capture_stdout: bool,
    pub capture_stderr: bool,
    pub stdin_storage_mode: PayloadStdioStorageMode,
    pub stdout_storage_mode: PayloadStdioStorageMode,
    pub stderr_storage_mode: PayloadStdioStorageMode,
    pub max_segment_bytes: u32,
    pub ring_buffer_bytes: u32,
    pub pending_operation_max_entries: u32,
    pub stream_state_max_entries: u32,
    pub retention_max_bytes_per_trace: u64,
    pub redaction_policy: PayloadRedactionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadSocketConfig {
    pub enabled: bool,
    pub capture_backend: PayloadSocketCaptureBackend,
    pub max_segment_bytes: u32,
    pub max_operation_bytes: u32,
    pub ring_buffer_bytes: u32,
    pub pending_operation_max_entries: u32,
    pub stream_state_max_entries: u32,
    pub retention_max_bytes_per_trace: u64,
    pub redaction_policy: PayloadRedactionPolicy,
    pub http_sniff_max_bytes: u64,
    pub seccomp_syscalls: Vec<PayloadSocketSeccompSyscall>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadConfig {
    pub tls: PayloadTlsConfig,
    pub stdio: PayloadStdioConfig,
    pub socket: PayloadSocketConfig,
}
