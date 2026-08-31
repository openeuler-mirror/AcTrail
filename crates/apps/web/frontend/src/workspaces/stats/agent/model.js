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
    return (this.attribution?.tool_workloads ?? []).map((tool, index) => {
      const callCount = Number(tool.call_count ?? 0);
      const measuredIntervalCount = Number(tool.measured_interval_count ?? 0);
      const measuredDuration = tool.measured_duration_nanos == null
        ? null
        : Number(tool.measured_duration_nanos);
      return {
        key: tool.key,
        label: tool.label,
        callCount,
        measuredIntervalCount,
        measuredDuration,
        averageDuration: measuredDuration != null && measuredIntervalCount > 0
          ? measuredDuration / measuredIntervalCount
          : null,
        color: `var(--stats-chart-${(index % 8) + 1})`,
      };
    });
  }

  get timeSeries() {
    const colors = {
      model_side: 'var(--stats-chart-1)',
      agent_side: 'var(--stats-chart-2)',
      unattributed: 'var(--stats-chart-7)',
    };
    return (this.attribution?.categories ?? []).map((row) => ({
      key: row.key,
      label: row.label,
      total: Number(row.duration_nanos ?? 0),
      color: colors[row.key] ?? 'var(--stats-chart-4)',
    }));
  }

  get tokenSeries() {
    const colors = {
      cache_hit: 'var(--stats-chart-cache-hit)',
      cache_miss: 'var(--stats-chart-cache-miss)',
      output: 'var(--stats-chart-output)',
      reasoning: 'var(--stats-chart-reasoning)',
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
