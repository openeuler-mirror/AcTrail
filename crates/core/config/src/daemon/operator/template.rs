//! Default operator config template.

pub const OPERATOR_CONFIG_TEMPLATE: &str = r#"# AcTrail operator configuration.
# Every runtime constant used by AcTrail apps is explicit in this file.

socket_path = /tmp/actrail.sock
socket_mode_octal = 660
pid_file = /tmp/actraild.pid
storage_path = /tmp/actrail.sqlite
web_listen_addr = 127.0.0.1:18080
web_request_read_timeout_ms = 1000
export_directory = /tmp/actrail-export
log_path = /tmp/actraild.log
diagnostic_log_level = info

graph_schema_version = manual-v1
allow_active_trace_snapshot = true
export_payload_bytes_enabled = false
export_payload_text_enabled = false

otel_live_export_enabled = false
otel_live_export_path = /tmp/actrail-live-spans.otlp.jsonl
otel_live_export_overwrite_enabled = false
otel_live_export_queue_capacity = 1024
otel_live_export_flush_every_spans = 1

profile_name = live-required
required_capability = proc-lifecycle
# required_capability = proc-exec-context
# required_capability = fs-access-basic
# required_capability = fs-mmap
# required_capability = fs-exec-access
required_capability = net-transport
# required_capability = net-dns
# required_capability = net-tls-metadata
# required_capability = net-provider-classification
# required_capability = net-application-plaintext-http
# required_capability = net-application-http2-frames
# required_capability = net-application-plaintext-ws
# required_capability = tls-plaintext-payload
# required_capability = socket-plaintext-payload
# required_capability = resource-metrics
# required_capability = ipc-unix-socket
# required_capability = ipc-pipe-fifo
# required_capability = stdio-chunk
# required_capability = policy-ingest-processing
# required_capability = policy-plugin-host
# required_capability = policy-decision-record
# required_capability = enforcement-file-permission-fanotify

ebpf_enabled = true
memlock_rlimit = inherit
tracked_process_max_entries = 4096
pending_operation_max_entries = 4096
event_ring_buffer_max_bytes = 1048576
file_path_capture_enabled = false
file_path_max_bytes = 255

payload_tls_enabled = false
payload_tls_capture_backend = tls-sync
payload_tls_source = shared-library
payload_tls_resolver = openssl-symbols
payload_tls_library = openssl
payload_tls_library_path = auto
payload_tls_binary_path = disabled
payload_tls_pattern_path = disabled
payload_tls_max_segment_bytes = 4095
payload_tls_max_operation_bytes = 4194304
payload_tls_ring_buffer_bytes = 8388608
payload_tls_pending_operation_max_entries = 4096
payload_tls_seccomp_syscall = write
payload_tls_seccomp_syscall = writev
payload_tls_seccomp_syscall = sendto
payload_tls_seccomp_syscall = sendmsg
payload_tls_diagnostics_enabled = false
payload_tls_retention_max_bytes_per_trace = 10485760
payload_tls_redaction_policy = authorization-header
payload_tls_sync_runtime_library_path = auto
payload_tls_sync_event_socket_path = /tmp/actrail-template-tls-sync.sock
payload_tls_sync_socket_mode_octal = 660
payload_tls_sync_match_limit = 8

seccomp_notify_enabled = false
seccomp_notify_reserved_listener_fd = 253

process_seccomp_enabled = false
process_seccomp_syscall = execve
process_seccomp_syscall = execveat
process_seccomp_syscall = fork
process_seccomp_syscall = vfork
process_seccomp_syscall = clone
process_seccomp_syscall = clone3
process_seccomp_max_args = 64
process_seccomp_max_arg_bytes = 4096
process_seccomp_pending_max_entries = 4096

agent_invocation_enabled = false
# Optional TLS sync probe-plan prewarm hints; agent identity comes from observed LLM requests.
agent_invocation_command = opencode
agent_invocation_command = .opencode
agent_invocation_command = xiaoo
agent_invocation_command = xiaoo-tui
agent_invocation_command = claude

payload_stdio_enabled = false
payload_stdio_capture_stdin = false
payload_stdio_capture_stdout = true
payload_stdio_capture_stderr = true
payload_stdio_max_segment_bytes = 4095
payload_stdio_ring_buffer_bytes = 1048576
payload_stdio_pending_operation_max_entries = 4096
payload_stdio_stream_state_max_entries = 4096
payload_stdio_retention_max_bytes_per_trace = 10485760
payload_stdio_redaction_policy = authorization-header

payload_socket_enabled = false
payload_socket_capture_backend = bpf-copy-seccomp-fallback
payload_socket_max_segment_bytes = 4095
payload_socket_max_operation_bytes = 4194304
payload_socket_ring_buffer_bytes = 2097152
payload_socket_pending_operation_max_entries = 4096
payload_socket_stream_state_max_entries = 4096
payload_socket_retention_max_bytes_per_trace = 10485760
payload_socket_redaction_policy = authorization-header
payload_socket_http_sniff_max_bytes = 8192
payload_socket_seccomp_syscall = write
payload_socket_seccomp_syscall = sendto

application_protocol_enabled = false
application_protocol_http1_enabled = false
application_protocol_http2_enabled = false
application_http_capture_host = false
application_http_sse_enabled = false
application_http_sse_data_policy = disabled
application_http_sse_max_buffer_bytes = 1048576
application_http_sse_max_data_bytes = 4096
application_http2_max_frame_bytes = 16384
application_http2_max_connection_buffer_bytes = 1048576
application_http2_emit_data_preview = false
application_http2_max_data_preview_bytes = 4096

resource_metrics_enabled = false
resource_metrics_interval_ms = 1000
resource_metrics_include_children = true
resource_metrics_include_system = true
resource_metrics_cpu_alert_percent_millis = disabled
resource_metrics_memory_alert_rss_kb = disabled

provider_rules_enabled = false
provider_rules_path = /etc/actrail/provider-rules.conf
provider_unknown_provider_label = unknown

enforcement_enabled = false
enforcement_backend = fanotify
enforcement_scope = trace
enforcement_rules_path = /etc/actrail/enforcement-rules.conf
enforcement_default_decision = allow
enforcement_mark_strategy = parent-directories
enforcement_audit_enabled = true
enforcement_event_buffer_bytes = 65536

startup_wait_ms = 5000
shutdown_wait_ms = 5000
supervision_poll_interval_ms = 100
"#;
