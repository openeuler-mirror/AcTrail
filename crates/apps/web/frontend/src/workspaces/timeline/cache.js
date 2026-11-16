// Keep bounded query windows; rendered actions are a deduplicated projection of them.
import { TimelineRange } from './range.js';

export class TimelineWindowCache {
  constructor(options) {
    this.options = options;
    this.windows = [];
    this.sequence = 0;
  }

  clear() { this.windows = []; this.sequence = 0; }
  get size() { return this.windows.length; }

  complete(entry) { return !entry.data.hasMore && !entry.data.linksTruncated; }

  covers(range, touch = false) {
    const candidates = this.windows.filter(entry => this.complete(entry)
      && entry.range.startMs <= range.startMs + range.spanMs
      && entry.range.startMs + entry.range.spanMs >= range.startMs)
      .sort((a, b) => a.range.startMs - b.range.startMs);
    let end = range.startMs;
    for (const entry of candidates) {
      if (entry.range.startMs > end) break;
      end = Math.max(end, entry.range.startMs + entry.range.spanMs);
      if (end >= range.startMs + range.spanMs) {
        if (touch) {
          const used = new Set(candidates.filter(item => item.range.startMs <= end));
          this.windows = [...this.windows.filter(item => !used.has(item)),
            ...this.windows.filter(item => used.has(item))];
        }
        return true;
      }
    }
    return false;
  }

  add(range, data) {
    const entry = { range: { ...range }, data, sequence: this.sequence++ };
    // A complete replacement contains all actions in these ranges. A partial
    // response never discards a previously read window, even at the same zoom.
    if (this.complete(entry)) {
      this.windows = this.windows.filter(old => !TimelineRange.contains(range, old.range));
    }
    this.windows.push(entry);
    while (this.overBudget()) this.windows.shift();
  }

  overBudget() {
    const totals = this.windows.reduce((sum, { data }) => ({
      actions: sum.actions + data.actions.length,
      contexts: sum.contexts + data.contextActions.length,
      links: sum.links + data.links.length,
    }), { actions: 0, contexts: 0, links: 0 });
    return this.windows.length > this.options.maxCachedWindows
      || totals.actions > this.options.maxCachedActions
      || totals.contexts > this.options.maxCachedContexts
      || totals.links > this.options.maxCachedLinks;
  }

  status(range) {
    const complete = this.covers(range);
    return { hasMore: !complete, linksTruncated: !complete && this.windows.some(entry =>
      entry.data.linksTruncated && entry.range.startMs <= range.startMs + range.spanMs
      && entry.range.startMs + entry.range.spanMs >= range.startMs) };
  }

  // Only the bounded published JSON is compared, never the overview history.
  #sameValue(left, right) {
    if (Object.is(left, right)) return true;
    if (left === null || right === null || typeof left !== 'object' || typeof right !== 'object') return false;
    if (Array.isArray(left)) {
      if (!Array.isArray(right) || left.length !== right.length) return false;
      for (let index = 0; index < left.length; index += 1) {
        if (!this.#sameValue(left[index], right[index])) return false;
      }
      return true;
    }
    if (Array.isArray(right)) return false;
    let remaining = 0;
    for (const key in left) {
      if (!Object.hasOwn(left, key)) continue;
      if (!Object.hasOwn(right, key) || !this.#sameValue(left[key], right[key])) return false;
      remaining += 1;
    }
    for (const key in right) {
      if (Object.hasOwn(right, key)) remaining -= 1;
    }
    return remaining === 0;
  }

  rootCommandSnapshot(range) {
    let anchor = null;
    for (const entry of this.windows) {
      // The predecessor is owned by the query covering the viewport's left edge.
      // Its single-row read is complete even when ordinary pages hit a budget.
      if (entry.range.startMs <= range.startMs
        && entry.range.startMs + entry.range.spanMs > range.startMs
        && (!anchor || entry.sequence > anchor.sequence)) anchor = entry;
    }
    return anchor?.data.rootCommandSnapshot ?? null;
  }

  #rootCommand(action, processId) {
    return processId != null && action.kind === 'command.invocation'
      && action.process?.process_id === processId;
  }

  project(range, previous = null) {
    const snapshot = this.rootCommandSnapshot(range);
    const rootVersions = new Map();
    const actions = new Map();
    const contexts = new Map();
    const links = new Map();
    let scanned = 0;
    for (const { data } of [...this.windows].sort((a, b) => a.sequence - b.sequence)) {
      for (const action of data.actions) {
        actions.set(action.id, action);
        if (this.#rootCommand(action, snapshot?.rootProcessId)) rootVersions.set(action.id, action);
      }
      const root = data.rootCommandSnapshot?.action;
      if (root && this.#rootCommand(root, snapshot?.rootProcessId)) rootVersions.set(root.id, root);
      for (const action of data.contextActions) contexts.set(action.id, action);
      for (const link of data.links) links.set(`${link.parent}:${link.child}:${link.role}`, link);
      scanned += data.scanned;
    }
    if (snapshot) {
      const from = BigInt(snapshot.queryFrom);
      for (const [id, action] of actions) {
        if (this.#rootCommand(action, snapshot.rootProcessId)
          && BigInt(action.start_time_unix_nanos) < from) actions.delete(id);
      }
      if (snapshot.action) actions.set(snapshot.action.id, snapshot.action);
      for (const [id, action] of rootVersions) {
        if (actions.has(id)) actions.set(id, action);
      }
    }
    const projection = { actions: [...actions.values()], contextActions: [...contexts.values()], associations: [],
      links: [...links.values()], rootCommandSnapshot: snapshot, scanned, ...this.status(range) };
    for (const key of ['actions', 'contextActions', 'links', 'associations']) {
      if (this.#sameValue(projection[key], previous?.[key])) projection[key] = previous[key];
    }
    return projection;
  }
}
