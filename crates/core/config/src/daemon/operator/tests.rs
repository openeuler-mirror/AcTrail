use super::{OPERATOR_CONFIG_TEMPLATE, OperatorConfig};
use crate::daemon::{
    DiagnosticLogLevel, DisabledOrPath, PayloadTlsCaptureBackend, PayloadTlsLibrary,
    PayloadTlsResolver, PayloadTlsSeccompSyscall, PayloadTlsSource, ProcessSeccompSyscall,
};

#[test]
fn application_protocol_config_parses_when_tls_payload_is_required() {
    let config = OperatorConfig::parse(&http_enabled_config()).unwrap();

    assert!(config.application_protocol.enabled);
    assert!(config.application_protocol.http1_enabled);
    assert!(config.application_protocol.capture_host);
    assert!(!config.live_otel_export.enabled);
    assert_eq!(config.live_otel_export.queue_capacity, 1024);
    assert_eq!(config.diagnostic_log_level, DiagnosticLogLevel::Info);
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
            "required_capability = net-transport\n",
            "required_capability = net-transport\nrequired_capability = tls-plaintext-payload\n",
        )
        .replace("payload_tls_enabled = false", "payload_tls_enabled = true")
        .replace(
            "seccomp_notify_enabled = false",
            "seccomp_notify_enabled = true",
        );

    let config = OperatorConfig::parse(&raw).unwrap();

    assert_eq!(config.diagnostic_log_level, DiagnosticLogLevel::Debug);
    assert!(!config.payload_config.tls.diagnostics_enabled);
}

#[test]
fn live_otel_export_config_parses_as_own_section() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .replace(
            "otel_live_export_enabled = false",
            "otel_live_export_enabled = true",
        )
        .replace(
            "otel_live_export_path = /tmp/actrail-live-spans.otlp.jsonl",
            "otel_live_export_path = /tmp/actrail-test-live.otlp.jsonl",
        )
        .replace(
            "otel_live_export_overwrite_enabled = false",
            "otel_live_export_overwrite_enabled = true",
        )
        .replace(
            "otel_live_export_queue_capacity = 1024",
            "otel_live_export_queue_capacity = 8",
        )
        .replace(
            "otel_live_export_flush_every_spans = 1",
            "otel_live_export_flush_every_spans = 4",
        );

    let config = OperatorConfig::parse(&raw).unwrap();

    assert!(config.live_otel_export.enabled);
    assert_eq!(
        config.live_otel_export.path,
        std::path::PathBuf::from("/tmp/actrail-test-live.otlp.jsonl")
    );
    assert!(config.live_otel_export.overwrite_enabled);
    assert_eq!(config.live_otel_export.queue_capacity, 8);
    assert_eq!(config.live_otel_export.flush_every_spans, 4);
}

#[test]
fn http_capability_requires_plaintext_payload_capability() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .replace(
            "required_capability = net-transport\n",
            "required_capability = net-transport\nrequired_capability = net-application-plaintext-http\n",
        )
        .replace(
            "application_protocol_enabled = false",
            "application_protocol_enabled = true",
        )
        .replace(
            "application_protocol_http1_enabled = false",
            "application_protocol_http1_enabled = true",
        );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains(
        "net-application-plaintext-http requires enabled tls-plaintext-payload or socket-plaintext-payload in the same profile"
    ));
}

#[test]
fn application_protocol_config_parses_when_socket_payload_is_required() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .replace(
            "required_capability = net-transport\n",
            "required_capability = net-transport\nrequired_capability = socket-plaintext-payload\nrequired_capability = net-application-plaintext-http\n",
        )
        .replace("payload_socket_enabled = false", "payload_socket_enabled = true")
        .replace(
            "seccomp_notify_enabled = false",
            "seccomp_notify_enabled = true",
        )
        .replace(
            "application_protocol_enabled = false",
            "application_protocol_enabled = true",
        )
        .replace(
            "application_protocol_http1_enabled = false",
            "application_protocol_http1_enabled = true",
        );

    let config = OperatorConfig::parse(&raw).unwrap();

    assert!(config.payload_config.socket.enabled);
}

#[test]
fn http2_capability_requires_protocol_http2_config() {
    let raw = OPERATOR_CONFIG_TEMPLATE
        .replace(
            "required_capability = net-transport\n",
            "required_capability = net-transport\nrequired_capability = tls-plaintext-payload\nrequired_capability = net-application-http2-frames\n",
        )
        .replace("payload_tls_enabled = false", "payload_tls_enabled = true")
        .replace("seccomp_notify_enabled = false", "seccomp_notify_enabled = true")
        .replace(
            "application_protocol_enabled = false",
            "application_protocol_enabled = true",
        );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains(
        "net-application-http2-frames requires application_protocol_enabled=true and application_protocol_http2_enabled=true"
    ));
}

#[test]
fn sse_preview_requires_sse_enabled() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "application_http_sse_data_policy = disabled",
        "application_http_sse_data_policy = preview",
    );

    let error = OperatorConfig::parse(&raw).unwrap_err();

    assert!(error.contains(
        "application_http_sse_data_policy=preview requires application_http_sse_enabled=true"
    ));
}

#[test]
fn resource_metrics_capability_requires_sampler_config() {
    let raw = OPERATOR_CONFIG_TEMPLATE.replace(
        "required_capability = net-transport\n",
        "required_capability = net-transport\nrequired_capability = resource-metrics\n",
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
    assert_eq!(tls.source, PayloadTlsSource::SharedLibrary);
    assert_eq!(tls.resolver, PayloadTlsResolver::OpensslSymbols);
    assert_eq!(tls.library, PayloadTlsLibrary::Openssl);
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
    let raw = OPERATOR_CONFIG_TEMPLATE
        .replace(
            "required_capability = net-transport\n",
            "required_capability = net-transport\nrequired_capability = proc-exec-context\n",
        )
        .replace(
            "seccomp_notify_enabled = false",
            "seccomp_notify_enabled = true",
        )
        .replace(
            "process_seccomp_enabled = false",
            "process_seccomp_enabled = true",
        )
        .replace(
            "agent_invocation_enabled = false",
            "agent_invocation_enabled = true",
        );

    let config = OperatorConfig::parse(&raw).unwrap();

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

fn http_enabled_config() -> String {
    OPERATOR_CONFIG_TEMPLATE
        .replace(
            "required_capability = net-transport\n",
            "required_capability = net-transport\nrequired_capability = tls-plaintext-payload\nrequired_capability = net-application-plaintext-http\n",
        )
        .replace("payload_tls_enabled = false", "payload_tls_enabled = true")
        .replace("seccomp_notify_enabled = false", "seccomp_notify_enabled = true")
        .replace(
            "application_protocol_enabled = false",
            "application_protocol_enabled = true",
        )
        .replace(
            "application_protocol_http1_enabled = false",
            "application_protocol_http1_enabled = true",
        )
        .replace(
            "application_http_capture_host = false",
            "application_http_capture_host = true",
        )
}
