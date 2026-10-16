// Bounded window defaults; TimelineWindow accepts option overrides from its owner.
export const TIMELINE_WINDOW_DEFAULTS = Object.freeze({
  initialSpanMs: 30_000,
  debounceMs: 120,
  maxActions: 1000,
  maxLinks: 4000,
  maxPages: 4,
  prefetchRatio: 0.5,
  // Retention budgets span multiple queries; maxActions/maxLinks above bound one query.
  maxCachedWindows: 4,
  maxCachedActions: 4000,
  maxCachedContexts: 4000,
  maxCachedLinks: 16000,
});
