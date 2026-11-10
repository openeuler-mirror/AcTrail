const redaction = ['disabled', 'authorization-header'];
const storageModes = ['full', 'metadata-only', 'drop'];

const fieldHints = [
  { id: 'capture', fields: [
    { key: 'payload.tls.enabled', label: 'tlsCapture' },
    { key: 'payload.socket.enabled', label: 'socketCapture' },
    { key: 'payload.stdio.enabled', label: 'stdioCapture' },
    { key: 'payload.mcp.enabled', label: 'mcpCapture' },
    { key: 'payload.stdio.capture_stdin', label: 'captureStdin' },
    { key: 'payload.stdio.stdin_storage_mode', label: 'stdinStorage', options: storageModes },
    { key: 'payload.stdio.capture_stdout', label: 'captureStdout' },
    { key: 'payload.stdio.stdout_storage_mode', label: 'stdoutStorage', options: storageModes },
    { key: 'payload.stdio.capture_stderr', label: 'captureStderr' },
    { key: 'payload.stdio.stderr_storage_mode', label: 'stderrStorage', options: storageModes },
  ] },
  { id: 'content', fields: [
    { key: 'payload.tls.redaction_policy', label: 'tlsRedaction', options: redaction },
    { key: 'payload.tls.retention_max_bytes_per_trace', label: 'tlsLimit', unit: 'bytes' },
    { key: 'payload.stdio.redaction_policy', label: 'stdioRedaction', options: redaction },
    { key: 'payload.stdio.retention_max_bytes_per_trace', label: 'stdioLimit', unit: 'bytes' },
    { key: 'payload.socket.redaction_policy', label: 'socketRedaction', options: redaction },
    { key: 'payload.socket.retention_max_bytes_per_trace', label: 'socketLimit', unit: 'bytes' },
  ] },
  { id: 'retention', fields: [
    { key: 'storage.retention.enabled', label: 'traceRetention' },
    { key: 'storage.retention.max_trace_age', label: 'traceMaxAge', hint: 'traceAgeHint' },
    { key: 'storage.retention.sweep_interval', label: 'traceSweepInterval', hint: 'traceSweepHint' },
    { key: 'storage.retention.min_terminal_age', label: 'traceMinTerminalAge', hint: 'traceMinTerminalHint' },
    { key: 'storage.retention.max_traces_per_sweep', label: 'traceSweepLimit' },
    { key: 'storage.retention.protected_tags', label: 'protectedTags', wide: true },
    { key: 'resource_profiling.storage.database.sqlite.retention.enabled', label: 'resourceRetention' },
    { key: 'resource_profiling.storage.database.sqlite.retention.max_age_seconds', label: 'resourceMaxAge', unit: 'seconds' },
  ] },
  { id: 'resources', fields: [
    { key: 'resource_profiling.enabled', label: 'resourceEnabled', wide: true },
    { key: 'resource_profiling.collection.host.interval_ms', label: 'hostInterval', unit: 'ms' },
    { key: 'resource_profiling.collection.workload.interval_ms', label: 'workloadInterval', unit: 'ms' },
    { key: 'resource_profiling.alerts.cpu_alert_percent_millis', label: 'cpuAlert', type: 'percent', unit: 'percent', hint: 'cpuHint' },
    { key: 'resource_profiling.alerts.memory_alert_current_bytes', label: 'memoryAlert', type: 'threshold', unit: 'bytes' },
    { key: 'resource_profiling.alerts.memory_alert_rss_kb', label: 'rssAlert', type: 'threshold', unit: 'kib' },
  ] },
  { id: 'activity', fields: [
    { key: 'idle_detection.enabled', label: 'idleEnabled' },
    { key: 'idle_detection.threshold_secs', label: 'idleThreshold', hint: 'idleHint' },
    { key: 'export.snapshot.payload_bytes_enabled', label: 'exportBytes' },
    { key: 'export.snapshot.payload_text_enabled', label: 'exportText' },
  ] },
  { id: 'daemon', fields: [
    { key: 'control.socket_path', label: 'controlSocket' },
    { key: 'control.pending_connection_max', label: 'pendingConnections', hint: 'pendingConnectionsHint' },
    { key: 'control.active_trace_max', label: 'activeTraceLimit', hint: 'activeTraceLimitHint' },
    { key: 'control.pid_file', label: 'daemonPidFile' },
    { key: 'control.log_path', label: 'daemonLog' },
    { key: 'control.diagnostic_log_level', label: 'diagnosticLevel', options: ['off', 'info', 'debug'] },
    { key: 'control.workload_diagnostics.enabled', label: 'workloadDiagnostics' },
    { key: 'control.workload_diagnostics.interval_ms', label: 'workloadDiagnosticsInterval', unit: 'ms' },
    { key: 'payload.tls.sync_event_socket_path', label: 'tlsEventSocket', wide: true },
  ] },
  { id: 'service', fields: [
    { key: 'storage.backend', label: 'storageBackend', options: ['sqlite', 'noop'], hint: 'storageBackendHint' },
    { key: 'storage.sqlite.path', label: 'sqlitePath' },
    { key: 'storage.sqlite.busy_timeout_ms', label: 'sqliteBusyTimeout', unit: 'ms' },
    { key: 'storage.sqlite.event_record_layout', label: 'eventRecordLayout', options: ['rows', 'blocks'], hint: 'eventRecordLayoutHint' },
    { key: 'web.listen_addr', label: 'webListen', required: true, hint: 'listenHint' },
    { key: 'web.request_read_timeout_ms', label: 'webReadTimeout', unit: 'ms', hint: 'webReadTimeoutHint' },
    { key: 'plugins.discovery.directory', label: 'pluginDirectory', hint: 'pluginDirectoryHint', wide: true },
    { key: 'plugins.discovery.max_packages', label: 'pluginPackageLimit', hint: 'pluginPackageLimitHint' },
    { key: 'plugins.discovery.manifest_max_bytes', label: 'pluginManifestLimit', unit: 'bytes' },
  ] },
];

const hintsByKey = new Map(fieldHints.flatMap(group => group.fields.map(field => [field.key, field])));

export class ConfigFieldCatalog {
  static field(key, kind) {
    const hint = hintsByKey.get(key) ?? {};
    const type = kind === 'string_array' ? 'tags'
      : kind === 'text' && ['percent', 'threshold'].includes(hint.type) ? hint.type : kind;
    return { ...hint, key, type, wide: kind === 'toml' || hint.wide };
  }

  static groups(kinds) {
    const remaining = new Set(Object.keys(kinds));
    const groups = fieldHints.map(group => ({
      id: group.id,
      fields: group.fields.filter(field => remaining.delete(field.key))
        .map(field => this.field(field.key, kinds[field.key])),
    })).filter(group => group.fields.length);
    if (remaining.size) {
      groups.push({
        id: 'additional',
        fields: [...remaining].map(key => this.field(key, kinds[key])),
      });
    }
    return groups;
  }
}
