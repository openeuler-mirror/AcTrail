use super::{
    DEFAULT_CONTROL_PENDING_CONNECTION_MAX, OPERATOR_CONFIG_TEMPLATE, OperatorConfig,
    OperatorConfigInitStatus,
};
use crate::daemon::{
    DiagnosticLogLevel, DisabledOrPath, PayloadSocketSeccompSyscall, PayloadTlsCaptureBackend,
    PayloadTlsLibrary, PayloadTlsResolver, PayloadTlsSeccompSyscall, PayloadTlsSource,
    ProcessSeccompSyscall,
};
use storage_factory::StorageBackendKind;

#[test]
fn default_operator_config_is_full_monitor_collection() {
    let config = OperatorConfig::parse(OPERATOR_CONFIG_TEMPLATE).unwrap();

    assert_eq!(config.capture_profile.name.as_str(), "default-full-monitor");
    assert!(config.application_protocol.enabled);
    assert!(config.application_protocol.http1_enabled);
    assert!(config.application_protocol.http2_enabled);
    assert!(config.application_protocol.capture_host);
    assert!(config.payload_config.tls.enabled);
    assert!(config.payload_config.stdio.enabled);
    assert!(config.payload_config.socket.enabled);
    assert!(config.process_seccomp.enabled);
    assert!(config.agent_invocation.enabled);
    assert!(config.resource_metrics.enabled);
    assert!(config.ebpf_config.file_path_capture_enabled);
    assert!(config.export_runtime.enabled);
    assert!(!config.enforcement.enabled);
    assert_eq!(config.export_runtime.routes().len(), 1);
    assert_eq!(
        config.export_runtime.routes()[0].target.kind().as_str(),
        "otel-jsonl"
    );
    assert_eq!(
        config.export_runtime.routes()[0].delivery.as_str(),
        "best-effort"
    );
    assert_eq!(
        config.export_runtime.enabled_output_files()[0].path,
        std::path::PathBuf::from("/tmp/actrail-live-spans.otlp.jsonl")
    );
    assert_eq!(config.diagnostic_log_level, DiagnosticLogLevel::Info);
    assert_eq!(config.storage.backend(), StorageBackendKind::Sqlite);
    assert_eq!(
        config.control_pending_connection_max,
        DEFAULT_CONTROL_PENDING_CONNECTION_MAX
    );
    assert_eq!(
        config.payload_config.socket.seccomp_syscalls,
        vec![
            PayloadSocketSeccompSyscall::Write,
            PayloadSocketSeccompSyscall::Writev,
            PayloadSocketSeccompSyscall::Sendto,
            PayloadSocketSeccompSyscall::Sendmsg,
        ]
    );
}

#[test]
fn storage_sqlite_busy_timeout_is_required() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .lines()
        .filter(|line| !line.starts_with("storage_sqlite_busy_timeout_ms = "))
        .collect::<Vec<_>>()
        .join("\n");

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains("missing config key storage_sqlite_busy_timeout_ms"));
}

#[test]
fn storage_backend_is_required() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .lines()
        .filter(|line| !line.starts_with("storage_backend = "))
        .collect::<Vec<_>>()
        .join("\n");

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains("missing config key storage_backend"));
}

#[test]
fn storage_backend_rejects_unknown_value() {
    let raw =
        OPERATOR_CONFIG_TEMPLATE.replace("storage_backend = sqlite", "storage_backend = mysql");

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains("invalid storage_backend"));
}

#[test]
fn control_pending_connection_max_defaults_for_existing_configs() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .lines()
        .filter(|line| !line.starts_with("control_pending_connection_max = "))
        .collect::<Vec<_>>()
        .join("\n");

    let config = OperatorConfig::parse(&raw).unwrap();

    assert_eq!(
        config.control_pending_connection_max,
        DEFAULT_CONTROL_PENDING_CONNECTION_MAX
    );
}

#[test]
fn control_pending_connection_max_rejects_zero() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "control_pending_connection_max = 256",
        "control_pending_connection_max = 0",
    );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains("invalid control_pending_connection_max"));
}

#[test]
fn storage_busy_timeout_rejects_zero() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "storage_sqlite_busy_timeout_ms = 5000",
        "storage_sqlite_busy_timeout_ms = 0",
    );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains("invalid storage_sqlite_busy_timeout_ms"));
}

#[test]
fn diagnostic_log_level_rejects_unknown_value() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "diagnostic_log_level = info",
        "diagnostic_log_level = trace",
    );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains("invalid diagnostic_log_level"));
}

#[test]
fn diagnostic_log_level_does_not_mutate_tls_diagnostics_config() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .replace(
            "diagnostic_log_level = info",
            "diagnostic_log_level = debug",
        )
        .replace(
            "payload_tls_diagnostics_enabled = true",
            "payload_tls_diagnostics_enabled = false",
        );

    let config = OperatorConfig::parse(&raw).unwrap();

    assert_eq!(config.diagnostic_log_level, DiagnosticLogLevel::Debug);
    assert!(!config.payload_config.tls.diagnostics_enabled);
}

#[test]
fn export_runtime_config_parses_route_sections() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .replace("[export]\nenabled = true", "[export]\nenabled = false")
        .replace(
            "path = \"/tmp/actrail-live-spans.otlp.jsonl\"",
            "path = \"/tmp/actrail-test-live.otlp.jsonl\"",
        )
        .replace("overwrite_enabled = true", "overwrite_enabled = false")
        .replace("queue_capacity = 1024", "queue_capacity = 8")
        .replace("flush_every_spans = 1", "flush_every_spans = 4");

    let config = OperatorConfig::parse(&raw).unwrap();
    let output_files = config.export_runtime.enabled_output_files();

    assert!(!config.export_runtime.enabled);
    assert!(output_files.is_empty());
    assert_eq!(config.export_runtime.routes()[0].name, "live-otel");
    let export_factory::ExportRouteTargetConfig::OtelJsonl(otel_jsonl) =
        &config.export_runtime.routes()[0].target;
    assert_eq!(
        otel_jsonl.path,
        std::path::PathBuf::from("/tmp/actrail-test-live.otlp.jsonl")
    );
    assert!(!otel_jsonl.overwrite_enabled);
    assert_eq!(otel_jsonl.queue_capacity, 8);
    assert_eq!(otel_jsonl.flush_every_spans, 4);
}

#[test]
fn http_capability_requires_plaintext_payload_capability() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .replace("required_capability = tls-plaintext-payload\n", "")
        .replace("required_capability = socket-plaintext-payload\n", "");

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains(
        "net-application-plaintext-http requires enabled tls-plaintext-payload or socket-plaintext-payload in the same profile"
    ));
}

#[test]
fn application_protocol_config_parses_when_socket_payload_is_required() {
    let config = OperatorConfig::parse(OPERATOR_CONFIG_TEMPLATE).unwrap();

    assert!(config.payload_config.socket.enabled);
}

#[test]
fn http2_capability_requires_protocol_http2_config() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "application_protocol_http2_enabled = true",
        "application_protocol_http2_enabled = false",
    );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains(
        "net-application-http2-frames requires application_protocol_enabled=true and application_protocol_http2_enabled=true"
    ));
}

#[test]
fn sse_preview_requires_sse_enabled() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "application_http_sse_enabled = true",
        "application_http_sse_enabled = false",
    );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains(
        "application_http_sse_data_policy=preview requires application_http_sse_enabled=true"
    ));
}

#[test]
fn resource_metrics_capability_requires_sampler_config() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "resource_metrics_enabled = true",
        "resource_metrics_enabled = false",
    );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains("resource-metrics requires resource_metrics_enabled=true"));
}

#[test]
fn fanotify_enforcement_capability_requires_enforcement_config() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "required_capability = net-transport\n",
        "required_capability = net-transport\nrequired_capability = enforcement-file-permission-fanotify\n",
    );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(
        error.contains("enforcement-file-permission-fanotify requires enforcement_enabled=true")
    );
}

#[test]
fn xiaoo_tls_example_parses() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("docs/examples/06.xiaoo-tls-capture/operator.conf");
    let raw = std::fs::read_to_string(path).expect("read xiaoO TLS example config");
    let config = OperatorConfig::parse(&raw).expect("parse xiaoO TLS example config");
    let tls = &config.payload_config.tls;

    assert!(tls.enabled);
    assert_eq!(tls.capture_backend, PayloadTlsCaptureBackend::TlsSync);
    assert_eq!(tls.source, PayloadTlsSource::Auto);
    assert_eq!(tls.resolver, PayloadTlsResolver::Auto);
    assert_eq!(tls.library, PayloadTlsLibrary::Auto);
    assert!(matches!(tls.binary_path, DisabledOrPath::Disabled));
    assert!(matches!(tls.pattern_path, DisabledOrPath::Disabled));
    assert!(config.seccomp_notify.enabled);
    assert_eq!(config.seccomp_notify.reserved_listener_fd, 253);
    assert_eq!(
        tls.seccomp_syscalls,
        vec![
            PayloadTlsSeccompSyscall::Write,
            PayloadTlsSeccompSyscall::Writev,
            PayloadTlsSeccompSyscall::Sendto,
            PayloadTlsSeccompSyscall::Sendmsg,
        ]
    );
    assert!(config.export_config.payload_bytes_enabled);
    assert!(config.export_config.payload_text_enabled);
}

#[test]
fn process_seccomp_config_parses_for_exec_context() {
    let config = OperatorConfig::parse(OPERATOR_CONFIG_TEMPLATE).unwrap();

    assert!(config.process_seccomp.enabled);
    assert_eq!(
        config.process_seccomp.syscalls,
        vec![
            ProcessSeccompSyscall::Execve,
            ProcessSeccompSyscall::Execveat,
            ProcessSeccompSyscall::Fork,
            ProcessSeccompSyscall::Vfork,
            ProcessSeccompSyscall::Clone,
            ProcessSeccompSyscall::Clone3,
        ]
    );
    assert!(config.agent_invocation.enabled);
    assert!(
        config
            .agent_invocation
            .commands
            .iter()
            .any(|item| item == "claude")
    );
    assert!(
        config
            .agent_invocation
            .commands
            .iter()
            .any(|item| item == "xiaoo")
    );
}

#[test]
fn agent_invocation_commands_are_optional_dynamic_lookup_hints() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .lines()
        .filter(|line| !line.starts_with("agent_invocation_command = "))
        .collect::<Vec<_>>()
        .join("\n");

    let config = OperatorConfig::parse(&raw).unwrap();

    assert!(config.agent_invocation.enabled);
    assert!(config.agent_invocation.commands.is_empty());
}

#[test]
fn initialize_creates_default_operator_config() {
    let path = temp_config_path("create");
    let _ = std::fs::remove_file(&path);

    let status = OperatorConfig::initialize(&path).unwrap();

    assert_eq!(status, OperatorConfigInitStatus::Created);
    let config = OperatorConfig::load(&path).unwrap();
    assert_eq!(config.capture_profile.name.as_str(), "default-full-monitor");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn initialize_validates_existing_operator_config() {
    let path = temp_config_path("existing-valid");
    std::fs::write(&path, OPERATOR_CONFIG_TEMPLATE).unwrap();

    let status = OperatorConfig::initialize(&path).unwrap();

    assert_eq!(status, OperatorConfigInitStatus::ExistingValid);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn initialize_rejects_existing_invalid_operator_config() {
    let path = temp_config_path("existing-invalid");
    std::fs::write(&path, "profile_name = broken\n").unwrap();

    let error = OperatorConfig::initialize(&path).unwrap_err();

    assert!(error.contains("validate config"));
    std::fs::remove_file(path).unwrap();
}

fn temp_config_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "actrail-operator-config-{name}-{}.conf",
        std::process::id()
    ))
}
