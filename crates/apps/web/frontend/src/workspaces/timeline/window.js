// Load visible ranges on demand and reuse a bounded collection of prior windows.
import { getCurrentScope, markRaw, onScopeDispose, ref, shallowRef } from 'vue';
import { TimelineFilters } from './filters.js';
import { TimelineRange } from './range.js';
import { TimelineWindowCache } from './cache.js';
import { TimelineOverviewIndex } from './overview.js';
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
    this.overview = markRaw(new TimelineOverviewIndex(readPage, this.options));
    if (getCurrentScope()) onScopeDispose(() => this.overview.clear());
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
      partial: true, loadedTraceId: null, hasMore: false, scanned: 0, linksTruncated: false,
      overview: this.overview };
  }

  get viewProps() {
    return { waterfall: this.data.value, timelineWindow: this.window.value,
      timelineBounds: this.bounds.value, timelineViewport: this.viewport.value };
  }

  reset(traceId = null) {
    this.cancel();
    this.traceId = traceId;
    this.cache.clear();
    this.overview.clear();
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

  async load(traceId, { filters = this.filters, metadata, overview = false } = {}) {
    this.cancel();
    const changed = this.traceId !== traceId || JSON.stringify(this.filters) !== JSON.stringify(filters);
    const preserveViewport = !changed && this.cache.size > 0;
    this.traceId = traceId;
    this.filters = { ...filters };
    // load is the explicit refresh/filter boundary; pan and zoom use setViewport.
    this.cache.clear();
    this.overview.clear();
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
    this.error.value = '';
    if (!preserveViewport) this.viewport.value = this.initialViewport();
    this.attemptedRange = null;
    if (overview) this.ensureOverview(traceId);
    return this.loadVisible();
  }

  ensureOverview(traceId = this.traceId) {
    if (!traceId || traceId !== this.traceId || !this.window.value || this.error.value === 'range'
      || this.overview.controller || (this.overview.initialized && !this.overview.error.value)) return;
    return this.overview.load(traceId, this.filters, this.window.value, this.bounds.value);
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
    if (this.overview.requested) this.overview.merge(this.observedActions(data));
    this.cache.add(range, data);
    this.data.value = { ...this.empty(), ...this.cache.project(this.viewport.value, this.data.value), loadedTraceId: this.traceId };
  }

  updateStatus() {
    if (this.cache.rootCommandSnapshot(this.viewport.value) !== this.data.value.rootCommandSnapshot) {
      this.data.value = { ...this.data.value, ...this.cache.project(this.viewport.value, this.data.value) };
      return;
    }
    const status = this.cache.status(this.viewport.value);
    if (status.hasMore !== this.data.value.hasMore || status.linksTruncated !== this.data.value.linksTruncated) {
      this.data.value = { ...this.data.value, ...status };
    }
  }

  observedActions(data) {
    const root = data.rootCommandSnapshot?.action;
    return root ? [...data.actions, root] : data.actions;
  }

  async refreshTail(readMetadata, spanMs, signal) {
    const traceId = this.traceId;
    const metadata = await readMetadata(traceId, { signal });
    // Metadata includes kinds hidden by the current filters. Use it to discover
    // new actions, while retaining a lookback around the last actual activity.
    const queryWindow = TimelineRange.fromMetadata(metadata, true);
    if (!queryWindow) throw new Error('Missing trace time bounds');
    const queryBounds = TimelineRange.bounds(queryWindow, this.filters);
    const knownWindow = this.overview.observed(TimelineRange.observed(metadata, this.data.value.actions, this.window.value));
    const previous = this.window.value?.observed ? this.window.value : knownWindow;
    const lastStartMs = Number(knownWindow.lastStartNanos - knownWindow.startNanos) / 1e6;
    const startMs = Math.max(queryBounds.startMs, Math.min(lastStartMs - spanMs,
      queryBounds.startMs + Math.max(queryBounds.spanMs - spanMs, 0)));
    const range = { startMs, spanMs: Math.max(queryBounds.startMs + queryBounds.spanMs - startMs, 1) };
    const data = await this.readWindow(range, signal, queryWindow);
    if (signal.aborted || this.traceId !== traceId) return null;
    const window = this.overview.observed(TimelineRange.observed(metadata, this.observedActions(data), knownWindow));
    window.activityAdvanced = window.endNanos > previous.endNanos || window.lastStartNanos > previous.lastStartNanos;
    const bounds = TimelineRange.bounds(window, this.filters);
    const target = { startMs: bounds.startMs + Math.max(bounds.spanMs - spanMs, 0), spanMs };
    // Commit only complete requests. Old data remains visible during reads/failures.
    const originShift = Number(this.window.value.startNanos - window.startNanos) / 1e6;
    if (originShift !== 0) this.viewport.value = { ...this.viewport.value,
      startMs: this.viewport.value.startMs + originShift };
    this.window.value = window;
    this.bounds.value = bounds;
    // Keep the outgoing window during movement. Ranges have to share an origin.
    if (originShift !== 0 || data.hasMore || data.linksTruncated) this.cache.clear();
    // Retain fixed-width coverage while the observed trace is shorter than the span.
    if (this.overview.requested) {
      this.overview.merge(this.observedActions(data));
      if (data.hasMore) this.overview.extend(traceId, this.filters, queryWindow, range);
    }
    this.cache.add(target, data);
    this.data.value = { ...this.empty(), ...this.cache.project(target, this.data.value), loadedTraceId: traceId };
    this.error.value = '';
    return target;
  }

  async readWindow(range, signal, window = this.window.value) {
    const query = TimelineRange.query(window, range);
    const rootCommandSnapshot = { rootProcessId: window.rootProcessId, queryFrom: query.from, action: null };
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
        ...query, cursor, limit, signal });
      if (signal.aborted) throw new DOMException('Cancelled', 'AbortError');
      if (page === 0 && result.root_command_before_window) {
        rootCommandSnapshot.action = markRaw(result.root_command_before_window);
      }
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
      links: [...links.values()], rootCommandSnapshot, hasMore, linksTruncated, scanned };
  }
}
