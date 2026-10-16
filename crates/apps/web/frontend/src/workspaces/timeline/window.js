// Load visible ranges on demand and reuse a bounded collection of prior windows.
import { markRaw, ref, shallowRef } from 'vue';
import { TimelineFilters } from './filters.js';
import { TimelineRange } from './range.js';
import { TimelineWindowCache } from './cache.js';
import { TIMELINE_WINDOW_DEFAULTS } from './constants.js';
import { constrainTimeViewport } from '../../tabs/core/time-navigation/model.js';

export class TimelineWindow {
  constructor(readPage, options = {}) {
    this.readPage = readPage;
    this.options = { ...TIMELINE_WINDOW_DEFAULTS, ...options };
    for (const name of ['initialSpanMs', 'debounceMs', 'maxActions', 'maxLinks', 'maxPages',
      'maxCachedWindows', 'maxCachedActions', 'maxCachedContexts', 'maxCachedLinks']) {
      if (!Number.isSafeInteger(this.options[name]) || this.options[name] <= 0) throw new Error(`Invalid timeline option: ${name}`);
    }
    if (!Number.isFinite(this.options.prefetchRatio) || this.options.prefetchRatio < 0 || this.options.prefetchRatio > 1) {
      throw new Error('Invalid timeline prefetch ratio');
    }
    if (this.options.maxCachedActions < this.options.maxActions
      || this.options.maxCachedContexts < this.options.maxActions
      || this.options.maxCachedLinks < this.options.maxLinks) {
      throw new Error('Timeline retention budgets must hold one complete query');
    }
    this.cache = new TimelineWindowCache(this.options);
    this.data = shallowRef(this.empty());
    this.loading = ref(false);
    this.error = ref('');
    this.window = shallowRef(null);
    this.bounds = shallowRef({ startMs: 0, spanMs: 1 });
    this.viewport = shallowRef({ startMs: 0, spanMs: 1 });
    this.filters = { kinds: TimelineFilters.defaultKinds.join(','), process_id: '', from: '', to: '', limit: 250 };
    this.controller = null;
    this.timer = null;
    this.traceId = null;
    this.attemptedRange = null;
  }

  empty() {
    return { actions: [], links: [], associations: [], contextActions: [], idleIntervals: [],
      partial: true, loadedTraceId: null, hasMore: false, scanned: 0, linksTruncated: false };
  }

  get viewProps() {
    return { waterfall: this.data.value, timelineWindow: this.window.value,
      timelineBounds: this.bounds.value, timelineViewport: this.viewport.value };
  }

  reset(traceId = null) {
    this.cancel();
    this.traceId = traceId;
    this.cache.clear();
    this.data.value = this.empty();
    this.window.value = null;
    this.attemptedRange = null;
    this.error.value = '';
  }

  cancel() {
    clearTimeout(this.timer);
    this.timer = null;
    this.controller?.abort();
    this.controller = null;
    this.loading.value = false;
    this.attemptedRange = null;
  }

  async load(traceId, { filters = this.filters, metadata } = {}) {
    this.cancel();
    const changed = this.traceId !== traceId || JSON.stringify(this.filters) !== JSON.stringify(filters);
    const preserveViewport = !changed && this.cache.size > 0;
    this.traceId = traceId;
    this.filters = { ...filters };
    // load is the explicit refresh/filter boundary; pan and zoom use setViewport.
    this.cache.clear();
    this.data.value = this.empty();
    if (metadata) this.window.value = TimelineRange.fromMetadata(metadata);
    if (!this.window.value) return;
    try {
      this.bounds.value = TimelineRange.bounds(this.window.value, this.filters);
    } catch {
      this.attemptedRange = null;
      this.error.value = 'range';
      return;
    }
    if (!preserveViewport) this.viewport.value = this.initialViewport();
    this.attemptedRange = null;
    return this.loadVisible();
  }

  initialViewport() {
    return { startMs: this.bounds.value.startMs,
      spanMs: Math.min(this.bounds.value.spanMs, this.options.initialSpanMs) };
  }

  setViewport(viewport) {
    const next = constrainTimeViewport(viewport, this.bounds.value);
    this.viewport.value = next;
    this.updateStatus();
    if (this.cache.covers(next, true)) {
      this.cancel();
      this.error.value = '';
      return;
    }
    if (this.error.value === 'range') return;
    if (this.attemptedRange?.startMs === next.startMs && this.attemptedRange?.spanMs === next.spanMs) return;
    this.cancel();
    this.timer = setTimeout(() => { this.timer = null; this.loadVisible(); }, this.options.debounceMs);
  }

  async retry() {
    this.cancel();
    this.attemptedRange = null;
    return this.loadVisible();
  }

  async loadVisible() {
    if (!this.window.value || !this.traceId) return;
    const range = { ...this.viewport.value };
    const controller = new AbortController();
    this.controller?.abort();
    this.controller = controller;
    this.loading.value = true;
    this.error.value = '';
    this.attemptedRange = range;
    try {
      const data = await this.readWindow(range, controller.signal);
      if (this.controller !== controller) return;
      this.publish(range, data);
      this.loading.value = false;
      // Prefetch only after the visible interval is complete and comfortably within budget.
      if (!data.hasMore && !data.linksTruncated && data.actions.length < this.options.maxActions / 2) {
        const margin = range.spanMs * this.options.prefetchRatio;
        const startMs = Math.max(this.bounds.value.startMs, range.startMs - margin);
        const endMs = Math.min(this.bounds.value.startMs + this.bounds.value.spanMs, range.startMs + range.spanMs + margin);
        const expanded = { startMs, spanMs: endMs - startMs };
        if ((expanded.startMs !== range.startMs || expanded.spanMs !== range.spanMs)
          && !this.cache.covers(expanded)) {
          try {
            const prefetched = await this.readWindow(expanded, controller.signal);
            if (this.controller === controller && !prefetched.hasMore && !prefetched.linksTruncated) this.publish(expanded, prefetched);
          } catch { /* The current visible window remains available if prefetch fails. */ }
        }
      }
    } catch (error) {
      if (this.controller === controller && error.name !== 'AbortError') this.error.value = 'load';
    } finally {
      if (this.controller === controller) { this.loading.value = false; this.controller = null; }
    }
  }

  publish(range, data) {
    this.cache.add(range, data);
    this.data.value = { ...this.empty(), ...this.cache.project(this.viewport.value), loadedTraceId: this.traceId };
  }

  updateStatus() {
    const status = this.cache.status(this.viewport.value);
    if (status.hasMore !== this.data.value.hasMore || status.linksTruncated !== this.data.value.linksTruncated) {
      this.data.value = { ...this.data.value, ...status };
    }
  }

  async readWindow(range, signal) {
    const actions = new Map();
    const context = new Map();
    const links = new Map();
    let cursor = null;
    let hasMore = false;
    let linksTruncated = false;
    let scanned = 0;
    for (let page = 0; page < this.options.maxPages; page += 1) {
      const limit = Math.min(Number(this.filters.limit), this.options.maxActions - actions.size);
      if (limit <= 0) { hasMore = true; break; }
      const result = await this.readPage(this.traceId, { ...this.filters,
        ...TimelineRange.query(this.window.value, range), cursor, limit, signal });
      if (signal.aborted) throw new DOMException('Cancelled', 'AbortError');
      for (const action of result.actions ?? []) actions.set(action.id, markRaw(action));
      for (const action of result.context_actions ?? []) {
        if (context.has(action.id) || context.size < this.options.maxActions) context.set(action.id, markRaw(action));
        else linksTruncated = true;
      }
      for (const link of result.links ?? []) {
        if (links.size >= this.options.maxLinks) { linksTruncated = true; break; }
        links.set(`${link.parent}:${link.child}:${link.role}`, markRaw(link));
      }
      scanned += result.scanned ?? 0;
      linksTruncated ||= Boolean(result.links_truncated);
      hasMore = Boolean(result.has_more);
      if (!hasMore || !result.next_cursor) break;
      if (cursor === result.next_cursor) break;
      cursor = result.next_cursor;
    }
    return { actions: [...actions.values()], contextActions: [...context.values()],
      links: [...links.values()], hasMore, linksTruncated, scanned };
  }
}
