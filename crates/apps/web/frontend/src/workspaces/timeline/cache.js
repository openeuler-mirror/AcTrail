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

  project(range) {
    const actions = new Map();
    const contexts = new Map();
    const links = new Map();
    let scanned = 0;
    for (const { data } of [...this.windows].sort((a, b) => a.sequence - b.sequence)) {
      for (const action of data.actions) actions.set(action.id, action);
      for (const action of data.contextActions) contexts.set(action.id, action);
      for (const link of data.links) links.set(`${link.parent}:${link.child}:${link.role}`, link);
      scanned += data.scanned;
    }
    return { actions: [...actions.values()], contextActions: [...contexts.values()],
      links: [...links.values()], scanned, ...this.status(range) };
  }
}
