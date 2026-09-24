export class AgentStatsModel {
  constructor({ llm = null, attribution = null, trace = null } = {}) {
    this.llm = llm;
    this.attribution = attribution;
    this.trace = trace;
  }

  get traceCount() {
    return Number(this.attribution?.coverage?.trace_count ?? this.llm?.summary?.trace_count ?? 0);
  }

  get requestCount() {
    return Number(this.llm?.summary?.completed_requests ?? 0);
  }

  get metrics() {
    const summary = this.llm?.summary ?? {};
    const toolCalls = this.toolWorkloads.reduce((total, tool) => total + tool.callCount, 0);
    return {
      turns: this.average(summary.completed_requests, summary.trace_count),
      tools: this.average(toolCalls, this.traceCount),
      reasoningTokens: this.average(summary.reasoning_tokens, summary.trace_count),
      promptTokens: this.average(summary.input_tokens, this.inputTokenSamples.length),
      blocks: this.average(summary.block_count, summary.block_count_rows),
      ttftUs: this.llm?.latency?.ttft?.mean_us ?? null,
    };
  }

  get toolWorkloads() {
    const merged = new Map();
    for (const tool of this.attribution?.tool_workloads ?? []) {
      const key = normalizeToolName(tool.key ?? tool.label);
      if (!key) {
        continue;
      }
      const entry = merged.get(key) ?? {
        key,
        callCount: 0,
        measuredIntervalCount: 0,
        measuredDuration: null,
      };
      entry.callCount += Number(tool.call_count ?? 0);
      entry.measuredIntervalCount += Number(tool.measured_interval_count ?? 0);
      if (tool.measured_duration_nanos != null) {
        entry.measuredDuration = (entry.measuredDuration ?? 0) + Number(tool.measured_duration_nanos);
      }
      merged.set(key, entry);
    }
    const rows = [...merged.values()].sort(
      (left, right) => right.callCount - left.callCount || left.key.localeCompare(right.key),
    );
    const totalCalls = rows.reduce((sum, row) => sum + row.callCount, 0) || 1;
    return rows.map((row, index) => ({
      ...row,
      label: row.key,
      share: row.callCount / totalCalls,
      totalCalls,
      averageDuration: row.measuredDuration != null && row.measuredIntervalCount > 0
        ? row.measuredDuration / row.measuredIntervalCount
        : null,
      color: `var(--ui-chart-${(index % 8) + 1})`,
    }));
  }

  /**
   * The ring sits next to the trace timeline and summarises the same three
   * categories, so it reuses the lane colours the timeline draws.
   */
  get timeSeries() {
    const colors = {
      model_side: 'var(--ui-chart-3)',
      agent_side: 'var(--ui-chart-4)',
      unattributed: 'var(--ui-chart-7)',
    };
    return (this.attribution?.categories ?? []).map((row) => ({
      key: row.key,
      label: row.label,
      total: Number(row.duration_nanos ?? 0),
      color: colors[row.key] ?? 'var(--ui-chart-5)',
    }));
  }

  get tokenSeries() {
    const colors = {
      cache_hit: 'var(--ui-chart-cache-hit)',
      cache_miss: 'var(--ui-chart-cache-miss)',
      output: 'var(--ui-chart-output)',
      reasoning: 'var(--ui-chart-reasoning)',
    };
    return (this.llm?.overview?.token_categories ?? [])
      .filter((row) => Object.hasOwn(colors, row.key))
      .map((row) => ({ ...row, total: Number(row.total ?? 0), color: colors[row.key] }));
  }

  get inputTokenSamples() {
    return (this.llm?.request_shape?.input_tokens_samples ?? [])
      .map(Number)
      .filter((value) => Number.isFinite(value) && value >= 0);
  }

  get traceTimelineSegments() {
    return (this.trace?.segments ?? []).map((row) => ({
      ...row,
      concurrent: row.subcategory === 'concurrent_tools',
      label: row.subcategory === 'concurrent_tools' && row.agent_tools?.length
        ? `${row.label}: ${this.concurrentToolLabels(row.agent_tools)}`
        : row.label,
    }));
  }

  get traceTimelineWindows() {
    return this.trace?.scope?.windows ?? [];
  }

  get traceTimeSeries() {
    return new AgentStatsModel({ attribution: this.trace }).timeSeries;
  }

  average(total, count) {
    const divisor = Number(count ?? 0);
    return divisor > 0 ? Number(total ?? 0) / divisor : null;
  }

  concurrentToolLabels(labels) {
    const counts = new Map();
    for (const label of labels) counts.set(label, (counts.get(label) ?? 0) + 1);
    return Array.from(counts, ([label, count]) => count > 1 ? `${label} ×${count}` : label).join(', ');
  }
}

export function formatDurationNanos(value) {
  const nanos = Number(value ?? 0);
  if (!Number.isFinite(nanos)) return '—';
  if (nanos < 1e6) return `${(nanos / 1e3).toFixed(0)} µs`;
  if (nanos < 1e9) return `${(nanos / 1e6).toFixed(1)} ms`;
  return `${(nanos / 1e9).toFixed(nanos < 1e10 ? 2 : 1)} s`;
}

/**
 * Tool names arrive from more than one harness, so the same tool can be
 * recorded as `bash`, `Bash` or `shell`. Normalise the spelling before the
 * workload rows are merged, otherwise one tool renders as several rows.
 */
const TOOL_NAME_ALIASES = Object.freeze({
  shell: 'bash',
  terminal: 'bash',
  exec: 'bash',
  file_write: 'write',
  write_file: 'write',
  file_edit: 'edit',
  edit_file: 'edit',
  str_replace_editor: 'edit',
  todo_write: 'todo',
  todowrite: 'todo',
  run_code: 'code',
  local_shell: 'bash',
});

export function normalizeToolName(name) {
  const raw = String(name ?? '').trim();
  if (!raw) {
    return '';
  }
  const collapsed = raw.replace(/\s+/g, '_').toLowerCase();
  return TOOL_NAME_ALIASES[collapsed] ?? collapsed;
}
