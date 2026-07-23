use super::*;

pub(super) fn parse_value<T>(key: &'static str, raw: &str) -> Result<T, String>
where
    T: FromStr<Err = String>,
{
    required_non_empty(key, raw)?
        .parse::<T>()
        .map_err(|error| format!("invalid {key}: {error}"))
}

pub(super) fn parse_values<T>(key: &'static str, values: &[String]) -> Result<Vec<T>, String>
where
    T: FromStr<Err = String>,
{
    if values.is_empty() {
        return Err(format!("missing config key {key}"));
    }
    values
        .iter()
        .map(|raw| parse_value::<T>(key, raw))
        .collect()
}

pub(super) fn required_non_empty<'a>(key: &'static str, raw: &'a str) -> Result<&'a str, String> {
    if raw.is_empty() {
        return Err(format!("config key {key} must not be empty"));
    }
    Ok(raw)
}

pub(super) fn parse_octal(key: &'static str, raw: &str) -> Result<u32, String> {
    u32::from_str_radix(required_non_empty(key, raw)?, 8)
        .map_err(|error| format!("invalid {key}: {error}"))
}

pub(super) fn require_positive_u32(key: &'static str, value: u32) -> Result<u32, String> {
    if value == u32::default() {
        return Err(format!("invalid {key}: value must be positive"));
    }
    Ok(value)
}

pub(super) fn require_positive_u64(key: &'static str, value: u64) -> Result<u64, String> {
    if value == u64::default() {
        return Err(format!("invalid {key}: value must be positive"));
    }
    Ok(value)
}

pub(super) fn parse_disabled_or_positive_u64(
    key: &'static str,
    raw: &str,
) -> Result<Option<u64>, String> {
    if raw == "disabled" {
        return Ok(None);
    }
    let value = raw
        .parse::<u64>()
        .map_err(|error| format!("invalid {key}: {error}"))?;
    if value == u64::default() {
        return Err(format!("invalid {key}: value must be positive or disabled"));
    }
    Ok(Some(value))
}

pub(super) fn parse_duration_millis(
    key: &'static str,
    raw: &str,
) -> Result<Option<Duration>, String> {
    if raw == "disabled" {
        return Ok(None);
    }
    let value = raw
        .parse::<u64>()
        .map_err(|error| format!("invalid {key}: {error}"))?;
    if value == u64::default() {
        return Err(format!("invalid {key}: value must be positive or disabled"));
    }
    Ok(Some(Duration::from_millis(value)))
}

pub(super) fn parse_required_duration(key: &'static str, raw: &str) -> Result<Duration, String> {
    let raw = required_non_empty(key, raw)?;
    if let Some(number) = raw.strip_suffix("min") {
        return duration_from_secs_unit(key, raw, number, 60);
    }
    if let Some(number) = raw.strip_suffix("ms") {
        let value = parse_positive_duration_number(key, raw, number)?;
        return Ok(Duration::from_millis(value));
    }
    if let Some(number) = raw.strip_suffix('s') {
        return duration_from_secs_unit(key, raw, number, 1);
    }
    if let Some(number) = raw.strip_suffix('m') {
        return duration_from_secs_unit(key, raw, number, 60);
    }
    if let Some(number) = raw.strip_suffix('h') {
        return duration_from_secs_unit(key, raw, number, 60 * 60);
    }
    if let Some(number) = raw.strip_suffix('d') {
        return duration_from_secs_unit(key, raw, number, 24 * 60 * 60);
    }
    Err(format!(
        "invalid {key}: expected a duration with unit ms, s, m, min, h, or d"
    ))
}

pub(super) fn duration_as_string(value: Duration) -> String {
    let millis = value.as_millis();
    if value.subsec_nanos() == 0 {
        let seconds = value.as_secs();
        if seconds % (24 * 60 * 60) == 0 {
            return format!("{}d", seconds / (24 * 60 * 60));
        }
        if seconds % (60 * 60) == 0 {
            return format!("{}h", seconds / (60 * 60));
        }
        if seconds % 60 == 0 {
            return format!("{}m", seconds / 60);
        }
        return format!("{seconds}s");
    }
    format!("{millis}ms")
}

fn duration_from_secs_unit(
    key: &'static str,
    raw: &str,
    number: &str,
    multiplier: u64,
) -> Result<Duration, String> {
    let value = parse_positive_duration_number(key, raw, number)?;
    value
        .checked_mul(multiplier)
        .map(Duration::from_secs)
        .ok_or_else(|| format!("invalid {key}: duration overflow"))
}

fn parse_positive_duration_number(
    key: &'static str,
    raw: &str,
    number: &str,
) -> Result<u64, String> {
    let value = number
        .parse::<u64>()
        .map_err(|error| format!("invalid {key} duration {raw}: {error}"))?;
    if value == u64::default() {
        return Err(format!("invalid {key}: duration must be positive"));
    }
    Ok(value)
}

pub(super) fn capability_as_str(capability: &Capability) -> &'static str {
    capability.as_str()
}

pub(super) fn diagnostic_log_level_as_str(value: super::super::DiagnosticLogLevel) -> &'static str {
    match value {
        super::super::DiagnosticLogLevel::Off => "off",
        super::super::DiagnosticLogLevel::Info => "info",
        super::super::DiagnosticLogLevel::Debug => "debug",
    }
}

pub(super) fn memlock_rlimit_as_str(value: MemlockRlimit) -> String {
    match value {
        MemlockRlimit::Inherit => "inherit".to_string(),
        MemlockRlimit::Unlimited => "unlimited".to_string(),
        MemlockRlimit::Bytes(bytes) => format!("bytes:{bytes}"),
    }
}

pub(super) fn disabled_or_u64_as_string(value: Option<u64>) -> String {
    value
        .map(|raw| raw.to_string())
        .unwrap_or_else(|| "disabled".to_string())
}

pub(super) fn payload_tls_capture_backend_as_str(value: PayloadTlsCaptureBackend) -> &'static str {
    match value {
        PayloadTlsCaptureBackend::SeccompUserRead => "seccomp-user-read",
        PayloadTlsCaptureBackend::BpfCopySeccompFallback => "bpf-copy-seccomp-fallback",
        PayloadTlsCaptureBackend::TlsSync => "tls-sync",
    }
}

pub(super) fn payload_tls_source_as_str(value: PayloadTlsSource) -> &'static str {
    match value {
        PayloadTlsSource::Auto => "auto",
        PayloadTlsSource::SharedLibrary => "shared-library",
        PayloadTlsSource::Executable => "executable",
    }
}

pub(super) fn payload_tls_resolver_as_str(value: PayloadTlsResolver) -> &'static str {
    match value {
        PayloadTlsResolver::Auto => "auto",
        PayloadTlsResolver::OpensslSymbols => "openssl-symbols",
        PayloadTlsResolver::BoringsslPatterns => "boringssl-patterns",
        PayloadTlsResolver::BunStaticBoringssl => "bun-static-boringssl",
        PayloadTlsResolver::BoringsslStatic => "boringssl-static",
        PayloadTlsResolver::RustlsSymbolMap => "rustls-symbol-map",
        PayloadTlsResolver::GoPclntab => "go-pclntab",
        PayloadTlsResolver::GnutlsSymbols => "gnutls-symbols",
        PayloadTlsResolver::NssNsprSymbols => "nss-nspr-symbols",
    }
}

pub(super) fn payload_tls_library_as_str(value: PayloadTlsLibrary) -> &'static str {
    match value {
        PayloadTlsLibrary::Auto => "auto",
        PayloadTlsLibrary::Openssl => "openssl",
        PayloadTlsLibrary::Boringssl => "boringssl",
        PayloadTlsLibrary::Rustls => "rustls",
        PayloadTlsLibrary::Go => "go",
        PayloadTlsLibrary::Gnutls => "gnutls",
        PayloadTlsLibrary::Nss => "nss",
    }
}

pub(super) fn payload_tls_library_path_as_string(value: &PayloadTlsLibraryPath) -> String {
    match value {
        PayloadTlsLibraryPath::Auto => "auto".to_string(),
        PayloadTlsLibraryPath::Path(path) => path.display().to_string(),
    }
}

pub(super) fn payload_tls_sync_runtime_library_path_as_string(
    value: &PayloadTlsSyncRuntimeLibraryPath,
) -> String {
    match value {
        PayloadTlsSyncRuntimeLibraryPath::Auto => "auto".to_string(),
        PayloadTlsSyncRuntimeLibraryPath::Path(path) => path.display().to_string(),
    }
}

pub(super) fn disabled_or_path_as_string(value: &DisabledOrPath) -> String {
    match value {
        DisabledOrPath::Disabled => "disabled".to_string(),
        DisabledOrPath::Path(path) => path.display().to_string(),
    }
}

pub(super) fn payload_redaction_policy_as_str(value: PayloadRedactionPolicy) -> &'static str {
    match value {
        PayloadRedactionPolicy::Disabled => "disabled",
        PayloadRedactionPolicy::AuthorizationHeader => "authorization-header",
    }
}

pub(super) fn payload_tls_seccomp_syscall_as_str(value: &PayloadTlsSeccompSyscall) -> &'static str {
    match value {
        PayloadTlsSeccompSyscall::Write => "write",
        PayloadTlsSeccompSyscall::Writev => "writev",
        PayloadTlsSeccompSyscall::Sendto => "sendto",
        PayloadTlsSeccompSyscall::Sendmsg => "sendmsg",
    }
}

pub(super) fn payload_socket_capture_backend_as_str(
    value: PayloadSocketCaptureBackend,
) -> &'static str {
    match value {
        PayloadSocketCaptureBackend::BpfCopy => "bpf-copy",
        PayloadSocketCaptureBackend::BpfCopySeccompFallback => "bpf-copy-seccomp-fallback",
    }
}

pub(super) fn payload_socket_seccomp_syscall_as_str(
    value: &PayloadSocketSeccompSyscall,
) -> &'static str {
    match value {
        PayloadSocketSeccompSyscall::Write => "write",
        PayloadSocketSeccompSyscall::Writev => "writev",
        PayloadSocketSeccompSyscall::Sendto => "sendto",
        PayloadSocketSeccompSyscall::Sendmsg => "sendmsg",
    }
}

pub(super) fn payload_stdio_storage_mode_as_str(value: PayloadStdioStorageMode) -> &'static str {
    match value {
        PayloadStdioStorageMode::Full => "full",
        PayloadStdioStorageMode::MetadataOnly => "metadata-only",
        PayloadStdioStorageMode::Drop => "drop",
    }
}

pub(super) fn process_seccomp_syscall_as_str(value: &ProcessSeccompSyscall) -> &'static str {
    match value {
        ProcessSeccompSyscall::Execve => "execve",
        ProcessSeccompSyscall::Execveat => "execveat",
        ProcessSeccompSyscall::Fork => "fork",
        ProcessSeccompSyscall::Vfork => "vfork",
        ProcessSeccompSyscall::Clone => "clone",
        ProcessSeccompSyscall::Clone3 => "clone3",
    }
}

pub(super) fn network_control_seccomp_syscall_as_str(
    value: NetworkControlSeccompSyscall,
) -> &'static str {
    match value {
        NetworkControlSeccompSyscall::Connect => "connect",
    }
}

pub(super) fn enforcement_seccomp_syscall_as_str(
    value: &EnforcementSeccompSyscall,
) -> &'static str {
    match value {
        EnforcementSeccompSyscall::Mkdir => "mkdir",
        EnforcementSeccompSyscall::Rmdir => "rmdir",
    }
}

pub(super) fn semantic_content_owner_as_str(value: SemanticContentOwner) -> &'static str {
    match value {
        SemanticContentOwner::HighestConsumed => "highest_consumed",
        SemanticContentOwner::ConfiguredLayers => "configured_layers",
    }
}

pub(super) fn llm_request_content_retention_as_str(
    value: LlmRequestContentRetention,
) -> &'static str {
    match value {
        LlmRequestContentRetention::None => "none",
        LlmRequestContentRetention::Shape => "shape",
        LlmRequestContentRetention::CanonicalBlocks => "canonical_blocks",
    }
}

pub(super) fn llm_response_content_retention_as_str(
    value: LlmResponseContentRetention,
) -> &'static str {
    match value {
        LlmResponseContentRetention::None => "none",
        LlmResponseContentRetention::AssembledProvider => "assembled_provider",
    }
}

pub(super) fn llm_tool_call_retention_as_str(value: LlmToolCallRetention) -> &'static str {
    match value {
        LlmToolCallRetention::None => "none",
        LlmToolCallRetention::AssembledJson => "assembled_json",
    }
}

pub(super) fn llm_usage_retention_as_str(value: LlmUsageRetention) -> &'static str {
    match value {
        LlmUsageRetention::None => "none",
        LlmUsageRetention::Summary => "summary",
    }
}

pub(super) fn sse_event_content_retention_as_str(value: SseEventContentRetention) -> &'static str {
    match value {
        SseEventContentRetention::None => "none",
        SseEventContentRetention::Parsed => "parsed",
        SseEventContentRetention::Raw => "raw",
    }
}

pub(super) fn http_headers_retention_as_str(value: HttpHeadersRetention) -> &'static str {
    match value {
        HttpHeadersRetention::None => "none",
        HttpHeadersRetention::Metadata => "metadata",
        HttpHeadersRetention::Full => "full",
    }
}

pub(super) fn http_body_retention_as_str(value: HttpBodyRetention) -> &'static str {
    match value {
        HttpBodyRetention::None => "none",
        HttpBodyRetention::Text => "text",
        HttpBodyRetention::Json => "json",
        HttpBodyRetention::Raw => "raw",
    }
}

pub(super) fn http2_data_content_retention_as_str(
    value: Http2DataContentRetention,
) -> &'static str {
    match value {
        Http2DataContentRetention::None => "none",
        Http2DataContentRetention::Preview => "preview",
        Http2DataContentRetention::Raw => "raw",
    }
}

pub(super) fn payload_body_content_retention_as_str(
    value: PayloadBodyContentRetention,
) -> &'static str {
    match value {
        PayloadBodyContentRetention::None => "none",
        PayloadBodyContentRetention::Retained => "retained",
    }
}

pub(super) fn file_metadata_retention_as_str(value: FileMetadataRetention) -> &'static str {
    match value {
        FileMetadataRetention::Full => "full",
        FileMetadataRetention::Compact => "compact",
    }
}

pub(super) fn file_raw_event_retention_as_str(value: FileRawEventRetention) -> &'static str {
    match value {
        FileRawEventRetention::Full => "full",
        FileRawEventRetention::ErrorsOnly => "errors_only",
        FileRawEventRetention::Summary => "summary",
    }
}

pub(super) fn sse_data_policy_as_str(value: SseDataPolicy) -> &'static str {
    match value {
        SseDataPolicy::Disabled => "disabled",
        SseDataPolicy::Preview => "preview",
    }
}

pub(super) fn enforcement_backend_as_str(value: EnforcementBackend) -> &'static str {
    match value {
        EnforcementBackend::Fanotify => "fanotify",
    }
}

pub(super) fn enforcement_scope_as_str(value: EnforcementScope) -> &'static str {
    match value {
        EnforcementScope::Trace => "trace",
    }
}

pub(super) fn enforcement_mark_strategy_as_str(value: EnforcementMarkStrategy) -> &'static str {
    match value {
        EnforcementMarkStrategy::ParentDirectories => "parent-directories",
    }
}

pub(super) fn parse_capability(raw: &str) -> Result<Capability, String> {
    raw.parse()
}
