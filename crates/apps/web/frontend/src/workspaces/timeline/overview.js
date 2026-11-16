// Retain only trace-wide timing summaries while full query windows stay bounded.
import { ref } from 'vue';
import { TimelineRange } from './range.js';

export class TimelineOverviewIndex {
  constructor(readPage, options) {
    this.readPage = readPage;
    this.options = options;
    this.records = new Map();
    // Derived extrema of records avoid scanning trace history on every live poll.
    this.endNanos = null;
    this.lastStartNanos = null;
    this.revision = ref(0);
    this.loading = ref(false);
    this.error = ref(false);
    this.controller = null;
    this.pending = null;
    this.activeRange = null;
    this.initialized = false;
  }

  get requested() { return this.initialized || this.controller !== null || this.error.value; }

  clear() {
    this.controller?.abort();
    this.controller = null;
    this.pending = null;
    this.activeRange = null;
    this.initialized = false;
    this.records.clear();
    this.endNanos = null;
    this.lastStartNanos = null;
    this.revision.value += 1;
    this.loading.value = false;
    this.error.value = false;
  }

  merge(actions, overwrite = true, notify = true) {
    let changed = false;
    for (const action of actions) {
      if (action.id == null || !action.kind || action.start_time_unix_nanos == null) continue;
      try {
        const next = { id: action.id, kind: action.kind, status: action.status,
          startNanos: BigInt(action.start_time_unix_nanos),
          endNanos: action.end_time_unix_nanos == null ? null : BigInt(action.end_time_unix_nanos) };
        const previous = this.records.get(next.id);
        // A background page cannot replace a newer visible snapshot, or reopen
        // a completed action. It can complete a previously running summary.
        if (previous && ((!overwrite && (previous.endNanos !== null || next.endNanos === null))
          || (previous.endNanos !== null && next.endNanos === null))) continue;
        if (previous && previous.kind === next.kind && previous.status === next.status
          && previous.startNanos === next.startNanos && previous.endNanos === next.endNanos) continue;
        this.records.set(next.id, next);
        const endNanos = next.endNanos === null || next.endNanos < next.startNanos ? next.startNanos : next.endNanos;
        if (this.endNanos === null || endNanos > this.endNanos) this.endNanos = endNanos;
        if (this.lastStartNanos === null || next.startNanos > this.lastStartNanos) this.lastStartNanos = next.startNanos;
        changed = true;
      } catch { /* An invalid timing record only affects its own overview interval. */ }
    }
    if (changed && notify) this.revision.value += 1;
    return changed;
  }

  observed(window) {
    if (this.endNanos === null) return window;
    const endNanos = this.endNanos > window.endNanos ? this.endNanos : window.endNanos;
    const lastStartNanos = this.lastStartNanos > window.lastStartNanos ? this.lastStartNanos : window.lastStartNanos;
    return { ...window, endNanos, lastStartNanos, spanMs: Number(endNanos - window.startNanos) / 1e6,
      endIso: new Date(Number(endNanos / 1_000_000n)).toISOString() };
  }

  extend(traceId, filters, window, range) {
    // TODO: Reuse completed ranges when metadata exposes a reliable change watermark.
    // Terminal lifecycle states can precede semantic drain; time bounds are insufficient.
    if (!this.controller) return this.load(traceId, filters, window, range);
    const next = TimelineRange.query(window, range);
    if (this.activeRange && BigInt(this.activeRange.from) <= BigInt(next.from)
      && BigInt(this.activeRange.to) >= BigInt(next.to)) return;
    this.enqueue(next);
  }

  enqueue(next) {
    const previous = this.pending ?? next;
    this.pending = {
      from: BigInt(previous.from) < BigInt(next.from) ? previous.from : next.from,
      to: BigInt(previous.to) > BigInt(next.to) ? previous.to : next.to,
    };
  }

  async load(traceId, filters, window, range) {
    const controller = new AbortController();
    this.controller?.abort();
    this.controller = controller;
    this.loading.value = true;
    this.error.value = false;
    let query = { ...filters, ...TimelineRange.query(window, range),
      limit: Math.min(Number(filters.limit), this.options.maxActions), signal: controller.signal };
    this.activeRange = query;
    let cursor = null;
    let changed = false;
    try {
      while (true) {
        const page = await this.readPage(traceId, { ...query, cursor });
        if (controller.signal.aborted || this.controller !== controller) return;
        changed = this.merge(page.actions ?? [], false, false) || changed;
        // Candidate budgets can produce an empty page with a continuation.
        // Link truncation does not truncate the action index.
        const next = page.next_cursor;
        if (!next) {
          if (page.has_more) throw new Error('Missing overview continuation');
          if (!this.pending) { this.initialized = true; break; }
          query = { ...query, ...this.pending };
          this.activeRange = query;
          this.pending = null;
          cursor = null;
          continue;
        }
        if (next === cursor) throw new Error('Overview cursor did not advance');
        cursor = next;
      }
    } catch (error) {
      if (this.controller === controller && error.name !== 'AbortError') {
        this.enqueue(query);
        this.error.value = true;
      }
    } finally {
      if (this.controller === controller) {
        // Publish once per scan rather than rebuilding all intervals each page.
        if (changed) this.revision.value += 1;
        this.loading.value = false;
        this.controller = null;
        this.activeRange = null;
      }
    }
  }
}
