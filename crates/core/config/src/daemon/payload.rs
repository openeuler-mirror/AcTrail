//! Payload capture configuration.

use std::path::PathBuf;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTlsLibrary {
    Openssl,
    Boringssl,
    Rustls,
}

impl FromStr for PayloadTlsLibrary {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "openssl" => Ok(Self::Openssl),
            "boringssl" => Ok(Self::Boringssl),
            "rustls" => Ok(Self::Rustls),
            other => Err(format!("unsupported payload TLS library {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTlsSource {
    SharedLibrary,
    Executable,
}

impl FromStr for PayloadTlsSource {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "shared-library" => Ok(Self::SharedLibrary),
            "executable" => Ok(Self::Executable),
            other => Err(format!("unsupported payload TLS source {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTlsResolver {
    OpensslSymbols,
    BoringsslPatterns,
    BunStaticBoringssl,
    BoringsslStatic,
    RustlsSymbolMap,
}

impl FromStr for PayloadTlsResolver {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "openssl-symbols" => Ok(Self::OpensslSymbols),
            "boringssl-patterns" => Ok(Self::BoringsslPatterns),
            "bun-static-boringssl" => Ok(Self::BunStaticBoringssl),
            "boringssl-static" => Ok(Self::BoringsslStatic),
            "rustls-symbol-map" => Ok(Self::RustlsSymbolMap),
            other => Err(format!("unsupported payload TLS resolver {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadTlsCaptureBackend {
    SeccompUserRead,
    BpfCopySeccompFallback,
}

impl FromStr for PayloadTlsCaptureBackend {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "seccomp-user-read" => Ok(Self::SeccompUserRead),
            "bpf-copy-seccomp-fallback" => Ok(Self::BpfCopySeccompFallback),
            other => Err(format!("unsupported payload TLS capture backend {other}")),
        }
    }
}

impl PayloadTlsCaptureBackend {
    pub const fn requires_seccomp_notify(self) -> bool {
        matches!(self, Self::SeccompUserRead | Self::BpfCopySeccompFallback)
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
    Sendto,
}

impl FromStr for PayloadSocketSeccompSyscall {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "write" => Ok(Self::Write),
            "sendto" => Ok(Self::Sendto),
            other => Err(format!(
                "unsupported payload socket seccomp syscall {other}; supported: write, sendto"
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadStdioConfig {
    pub enabled: bool,
    pub capture_stdin: bool,
    pub capture_stdout: bool,
    pub capture_stderr: bool,
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
