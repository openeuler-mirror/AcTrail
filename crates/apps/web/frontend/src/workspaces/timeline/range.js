// Anchor shared timeline coordinates and navigation limits to trace metadata.
export class TimelineRange {
  static fromMetadata(metadata, now = Date.now()) {
    const trace = metadata?.trace ?? {};
    const value = raw => raw == null ? null : BigInt(raw);
    const starts = [metadata?.timeline_bounds?.start_time_unix_nanos,
      trace.started_at_unix_nanos ?? trace.created_at_unix_nanos].map(value).filter(v => v !== null);
    if (!starts.length) return null;
    const startNanos = starts.reduce((a, b) => a < b ? a : b);
    const terminal = [trace.completed_at_unix_nanos, trace.exited_at_unix_nanos,
      trace.failed_at_unix_nanos].map(value).filter(v => v !== null);
    const ends = [value(metadata?.timeline_bounds?.end_time_unix_nanos), ...terminal].filter(v => v !== null);
    if (!terminal.length && !['completed', 'exited', 'failed'].includes(String(trace.state).toLowerCase())) ends.push(BigInt(now) * 1_000_000n);
    const endNanos = ends.reduce((a, b) => a > b ? a : b, startNanos + 1_000_000n);
    return { startNanos, endNanos, spanMs: Number(endNanos - startNanos) / 1e6,
      startIso: new Date(Number(startNanos / 1_000_000n)).toISOString(),
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
