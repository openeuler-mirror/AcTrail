// Anchor shared timeline coordinates and navigation limits to trace metadata.
export class TimelineRange {
  static fromMetadata(metadata, observed = false) {
    const trace = metadata?.trace ?? {};
    const value = raw => raw == null ? null : BigInt(raw);
    const starts = [metadata?.timeline_bounds?.start_time_unix_nanos,
      trace.started_at_unix_nanos ?? trace.created_at_unix_nanos].map(value).filter(v => v !== null);
    if (!starts.length) return null;
    const startNanos = starts.reduce((a, b) => a < b ? a : b);
    const terminal = [trace.completed_at_unix_nanos, trace.exited_at_unix_nanos,
      trace.failed_at_unix_nanos].map(value).filter(v => v !== null);
    const ends = [value(metadata?.timeline_bounds?.end_time_unix_nanos), ...(observed ? [] : terminal)].filter(v => v !== null);
    if (!observed && !terminal.length && !['completed', 'exited', 'failed'].includes(String(trace.state).toLowerCase())) ends.push(BigInt(Date.now()) * 1_000_000n);
    const endNanos = ends.reduce((a, b) => a > b ? a : b, startNanos + 1_000_000n);
    return { startNanos, endNanos, rootProcessId: trace.root_process_id,
      spanMs: Number(endNanos - startNanos) / 1e6,
      startIso: new Date(Number(startNanos / 1_000_000n)).toISOString(),
      endIso: new Date(Number(endNanos / 1_000_000n)).toISOString() };
  }

  static observed(metadata, actions, previous) {
    const window = this.fromMetadata(metadata, true);
    if (!window) throw new Error('Missing trace time bounds');
    let endNanos = window.startNanos + 1_000_000n;
    let lastStartNanos = window.startNanos;
    // Only an observed endpoint can survive a refresh; initial loads include wall time.
    if (previous?.observed && previous.endNanos > endNanos) endNanos = previous.endNanos;
    if (previous?.observed && previous.lastStartNanos > lastStartNanos) lastStartNanos = previous.lastStartNanos;
    for (const action of actions) {
      if (action.start_time_unix_nanos != null) {
        const start = BigInt(action.start_time_unix_nanos);
        if (start > lastStartNanos) lastStartNanos = start;
      }
      for (const raw of [action.start_time_unix_nanos, action.end_time_unix_nanos]) {
        if (raw == null) continue;
        const nanos = BigInt(raw);
        if (nanos > endNanos) endNanos = nanos;
      }
    }
    return { ...window, endNanos, lastStartNanos, observed: true,
      spanMs: Number(endNanos - window.startNanos) / 1e6,
      endIso: new Date(Number(endNanos / 1_000_000n)).toISOString() };
  }

  static bounds(window, filters) {
    if (!window) return { startMs: 0, spanMs: 1 };
    const startMs = filters.from ? Math.max(0, Number(BigInt(filters.from) - window.startNanos) / 1e6) : 0;
    const endMs = filters.to ? Math.min(window.spanMs, Number(BigInt(filters.to) - window.startNanos) / 1e6) : window.spanMs;
    if (startMs >= endMs) throw new Error('Selected time range does not intersect this trace');
    return { startMs, spanMs: endMs - startMs };
  }

  static contains(outer, inner) {
    return outer && inner && outer.startMs <= inner.startMs
      && outer.startMs + outer.spanMs >= inner.startMs + inner.spanMs;
  }

  static query(window, range) {
    return { from: String(window.startNanos + BigInt(Math.round(range.startMs * 1e6))),
      to: String(window.startNanos + BigInt(Math.round((range.startMs + range.spanMs) * 1e6))) };
  }
}
