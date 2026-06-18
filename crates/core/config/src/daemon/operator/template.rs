//! Default operator config template.

pub const OPERATOR_CONFIG_TEMPLATE: &str = r#"# AcTrail default operator configuration.
# This default enables broad collection, but does not enable blocking/enforcement.

socket_path = /run/actrail/control.sock
socket_mode_octal = 660
control_pending_connection_max = 256
pid_file = /run/actrail/actraild.pid
storage_backend = sqlite
storage_sqlite_path = /var/lib/actrail/actrail.sqlite
storage_sqlite_busy_timeout_ms = 5000
web_listen_addr = 127.0.0.1:18080
web_request_read_timeout_ms = 1000
export_directory = /var/lib/actrail/export
log_path = /var/log/actrail/actraild.log
diagnostic_log_level = info
workload_diagnostics_enabled = false
workload_diagnostics_interval_ms = 1000

graph_schema_version = manual-v1
allow_active_trace_snapshot = true
export_payload_bytes_enabled = true
export_payload_text_enabled = true

profile_name = default-full-monitor
required_capability = proc-lifecycle
required_capability = proc-exec-context
required_capability = fs-access-basic
required_capability = fs-mmap
required_capability = net-transport
required_capability = ipc-unix-socket
required_capability = ipc-pipe-fifo
required_capability = stdio-chunk
required_capability = tls-plaintext-payload
required_capability = socket-plaintext-payload
required_capability = net-application-plaintext-http
required_capability = net-application-http2-frames
required_capability = resource-metrics

ebpf_enabled = true
memlock_rlimit = inherit
tracked_process_max_entries = 8192
pending_operation_max_entries = 8192
suppressed_fd_max_entries = 8192
suppressed_fd_index_slots_per_process = 64
event_ring_buffer_max_bytes = 8388608
file_path_capture_enabled = true
file_path_max_bytes = 255

payload_tls_enabled = true
payload_tls_capture_backend = tls-sync
payload_tls_source = auto
payload_tls_resolver = auto
payload_tls_library = auto
payload_tls_library_path = auto
payload_tls_binary_path = disabled
payload_tls_pattern_path = disabled
payload_tls_max_segment_bytes = 4095
payload_tls_max_operation_bytes = 16777216
payload_tls_ring_buffer_bytes = 8388608
payload_tls_pending_operation_max_entries = 8192
payload_tls_seccomp_syscall = write
payload_tls_seccomp_syscall = writev
payload_tls_seccomp_syscall = sendto
payload_tls_seccomp_syscall = sendmsg
payload_tls_diagnostics_enabled = true
payload_tls_retention_max_bytes_per_trace = 104857600
payload_tls_redaction_policy = disabled
payload_tls_sync_runtime_library_path = auto
payload_tls_sync_event_socket_path = /run/actrail/tls-sync.sock
payload_tls_sync_socket_mode_octal = 660
payload_tls_sync_match_limit = 8
payload_tls_java_agent_enabled = false

seccomp_notify_enabled = true
seccomp_notify_reserved_listener_fd = 253

process_seccomp_enabled = true
process_seccomp_syscall = execve
process_seccomp_syscall = execveat
process_seccomp_syscall = fork
process_seccomp_syscall = vfork
process_seccomp_syscall = clone
process_seccomp_syscall = clone3
process_seccomp_max_args = 128
process_seccomp_max_arg_bytes = 8192
process_seccomp_pending_max_entries = 8192

agent_invocation_enabled = true
# Optional TLS sync probe-plan prewarm hints. Leave empty unless startup cost is acceptable.

payload_stdio_enabled = true
payload_stdio_capture_stdin = true
payload_stdio_capture_stdout = true
payload_stdio_capture_stderr = true
payload_stdio_stdin_storage_mode = full
payload_stdio_stdout_storage_mode = drop
payload_stdio_stderr_storage_mode = metadata-only
payload_stdio_max_segment_bytes = 4095
payload_stdio_ring_buffer_bytes = 4194304
payload_stdio_pending_operation_max_entries = 8192
payload_stdio_stream_state_max_entries = 8192
payload_stdio_retention_max_bytes_per_trace = 104857600
payload_stdio_redaction_policy = disabled

payload_socket_enabled = true
payload_socket_capture_backend = bpf-copy-seccomp-fallback
payload_socket_max_segment_bytes = 4095
payload_socket_max_operation_bytes = 4194304
payload_socket_ring_buffer_bytes = 8388608
payload_socket_pending_operation_max_entries = 8192
payload_socket_stream_state_max_entries = 8192
payload_socket_retention_max_bytes_per_trace = 104857600
payload_socket_redaction_policy = disabled
payload_socket_http_sniff_max_bytes = 65536
payload_socket_seccomp_syscall = write
payload_socket_seccomp_syscall = writev
payload_socket_seccomp_syscall = sendto
payload_socket_seccomp_syscall = sendmsg

application_protocol_enabled = true
application_protocol_http1_enabled = true
application_protocol_http2_enabled = true
application_http_capture_host = true
application_http_sse_enabled = true
application_http_sse_data_policy = preview
application_http_sse_max_buffer_bytes = 4194304
application_http_sse_max_data_bytes = 65536
application_http2_max_frame_bytes = 65536
application_http2_max_connection_buffer_bytes = 4194304
application_http2_emit_data_preview = true
application_http2_max_data_preview_bytes = 65536

resource_metrics_enabled = true
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

[semantic_retention]
content_owner = highest_consumed

[semantic_retention.L0_llm_call]
enabled = true
request_content = full_provider_json
response_content = assembled_provider
tool_calls = assembled_json
usage = summary

[semantic_retention.L1_sse]
enabled = true
stream_summary = true
event_content = none

[semantic_retention.L2_http]
enabled = true
message_summary = true
headers = metadata
body_content = text

[semantic_retention.L3_http2_frame]
enabled = true
frame_summary = true
data_content = none

[semantic_retention.L4_payload]
enabled = true
stats = true
body_content = none

[file_observation]
enabled = true
metadata_retention = compact

[file_observation.tty]
enabled = true
path = /dev/tty
path = /dev/pts/*
operation = open
operation = close
operation = read
operation = write
raw_event_retention = errors_only

[file_observation.bulk_read]
enabled = true
mode = path_set
raw_event_retention = errors_only
min_unique_paths = 128
max_paths_per_set = 4096
path_set_chunk_max_paths = 256

[file_observation.enumerate]
enabled = true
raw_event_retention = errors_only
min_unique_paths = 2
max_paths_per_set = 4096
path_set_chunk_max_paths = 256

[export]
enabled = true

[[export.routes]]
name = "live-otel"
kind = "otel-jsonl"
delivery = "best-effort"
enabled = true

[export.routes.otel-jsonl.live-otel]
path = "/var/lib/actrail/export/live-spans.otlp.jsonl"
overwrite_enabled = true
queue_capacity = 1024
flush_every_spans = 1
"#;
