const redaction = ['disabled', 'authorization-header'];
const storageModes = ['full', 'metadata-only', 'drop'];

export const editableGroups = [
  { id: 'capture', fields: [
    { key: 'payload.tls.enabled', label: 'tlsCapture', type: 'boolean' },
    { key: 'payload.socket.enabled', label: 'socketCapture', type: 'boolean' },
    { key: 'payload.stdio.enabled', label: 'stdioCapture', type: 'boolean' },
    { key: 'payload.mcp.enabled', label: 'mcpCapture', type: 'boolean' },
    { key: 'payload.stdio.capture_stdin', label: 'captureStdin', type: 'boolean' },
    { key: 'payload.stdio.stdin_storage_mode', label: 'stdinStorage', options: storageModes },
    { key: 'payload.stdio.capture_stdout', label: 'captureStdout', type: 'boolean' },
    { key: 'payload.stdio.stdout_storage_mode', label: 'stdoutStorage', options: storageModes },
    { key: 'payload.stdio.capture_stderr', label: 'captureStderr', type: 'boolean' },
    { key: 'payload.stdio.stderr_storage_mode', label: 'stderrStorage', options: storageModes },
  ] },
  { id: 'content', fields: [
    { key: 'payload.tls.redaction_policy', label: 'tlsRedaction', options: redaction },
    { key: 'payload.tls.retention_max_bytes_per_trace', label: 'tlsLimit', type: 'integer', unit: 'bytes' },
    { key: 'payload.stdio.redaction_policy', label: 'stdioRedaction', options: redaction },
    { key: 'payload.stdio.retention_max_bytes_per_trace', label: 'stdioLimit', type: 'integer', unit: 'bytes' },
    { key: 'payload.socket.redaction_policy', label: 'socketRedaction', options: redaction },
    { key: 'payload.socket.retention_max_bytes_per_trace', label: 'socketLimit', type: 'integer', unit: 'bytes' },
  ] },
  { id: 'retention', fields: [
    { key: 'storage.retention.enabled', label: 'traceRetention', type: 'boolean' },
    { key: 'storage.retention.max_trace_age', label: 'traceMaxAge', hint: 'traceAgeHint' },
    { key: 'storage.retention.protected_tags', label: 'protectedTags', type: 'tags', wide: true },
    { key: 'resource_profiling.storage.database.sqlite.retention.enabled', label: 'resourceRetention', type: 'boolean' },
    { key: 'resource_profiling.storage.database.sqlite.retention.max_age_seconds', label: 'resourceMaxAge', type: 'integer', unit: 'seconds' },
  ] },
  { id: 'resources', fields: [
    { key: 'resource_profiling.enabled', label: 'resourceEnabled', type: 'boolean', wide: true },
    { key: 'resource_profiling.collection.host.interval_ms', label: 'hostInterval', type: 'integer', unit: 'ms' },
    { key: 'resource_profiling.collection.workload.interval_ms', label: 'workloadInterval', type: 'integer', unit: 'ms' },
    { key: 'resource_profiling.alerts.cpu_alert_percent_millis', label: 'cpuAlert', type: 'percent', unit: 'percent', hint: 'cpuHint' },
    { key: 'resource_profiling.alerts.memory_alert_current_bytes', label: 'memoryAlert', type: 'threshold', unit: 'bytes' },
    { key: 'resource_profiling.alerts.memory_alert_rss_kb', label: 'rssAlert', type: 'threshold', unit: 'kib' },
  ] },
  { id: 'activity', fields: [
    { key: 'idle_detection.enabled', label: 'idleEnabled', type: 'boolean' },
    { key: 'idle_detection.threshold_secs', label: 'idleThreshold', hint: 'idleHint' },
    { key: 'export.snapshot.payload_bytes_enabled', label: 'exportBytes', type: 'boolean' },
    { key: 'export.snapshot.payload_text_enabled', label: 'exportText', type: 'boolean' },
  ] },
  { id: 'daemon', fields: [
    { key: 'control.socket_path', label: 'controlSocket' },
    { key: 'control.pid_file', label: 'daemonPidFile' },
    { key: 'control.log_path', label: 'daemonLog' },
    { key: 'control.diagnostic_log_level', label: 'diagnosticLevel', options: ['off', 'info', 'debug'] },
    { key: 'payload.tls.sync_event_socket_path', label: 'tlsEventSocket', wide: true },
  ] },
  { id: 'service', fields: [
    { key: 'storage.sqlite.path', label: 'sqlitePath' },
    { key: 'storage.sqlite.busy_timeout_ms', label: 'sqliteBusyTimeout', type: 'integer', unit: 'ms' },
    { key: 'web.listen_addr', label: 'webListen', required: true, hint: 'listenHint' },
    { key: 'plugins.discovery.directory', label: 'pluginDirectory', required: true },
  ] },
];
