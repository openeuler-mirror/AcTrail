export const LLM_TABS = Object.freeze([
  { id: 'overview' },
  { id: 'trends' },
  { id: 'latency' },
  { id: 'explore' },
  { id: 'settings' },
]);

export const QUICK_RANGES = Object.freeze([
  { id: '24h', label: '24h', days: 1 },
  { id: '7d', label: '7d', days: 7 },
  { id: '30d', label: '30d', days: 30 },
]);

export const EXPLORE_METRICS = Object.freeze([
  { id: 'total_tokens' },
  { id: 'input_tokens' },
  { id: 'output_tokens' },
  { id: 'reasoning_tokens' },
  { id: 'cache_hit_tokens' },
  { id: 'cache_miss_tokens' },
  { id: 'completed_requests' },
  { id: 'missing_usage_count' },
]);

export const EXPLORE_GROUPS = Object.freeze([
  { id: 'model' },
  { id: 'endpoint' },
  { id: 'app' },
]);

export const ROLLUPS = Object.freeze([
  { id: 'minute' },
  { id: 'hour' },
  { id: 'day' },
  { id: 'week' },
  { id: 'month' },
]);

export const CHART_KINDS = Object.freeze([
  { id: 'line' },
  { id: 'bar' },
  { id: 'histogram' },
  { id: 'donut' },
]);

export const TOP_N_OPTIONS = Object.freeze([5, 10, 20, 50]);
export const LATENCY_BIN_OPTIONS = Object.freeze([12, 20, 32, 48]);
export const DEFAULT_LATENCY_BIN_COUNT = 20;
export const DEFAULT_LATENCY_KDE_POINTS = 96;

export function defaultRange() {
  const to = new Date();
  const from = new Date(to);
  from.setDate(from.getDate() - 7);
  return {
    fromDate: dateInputValue(from),
    toDate: dateInputValue(to),
  };
}

export function rangeToMillis(range) {
  const from = localDateStart(range.fromDate);
  const to = localDateStart(range.toDate);
  if (!from || !to) {
    return { ok: false, error: 'Select a valid date range.' };
  }
  const toExclusive = new Date(to);
  toExclusive.setDate(toExclusive.getDate() + 1);
  if (from.getTime() >= toExclusive.getTime()) {
    return { ok: false, error: 'Start date must be on or before end date.' };
  }
  return { ok: true, fromMs: from.getTime(), toMs: toExclusive.getTime() };
}

export function quickRange(days) {
  const to = new Date();
  const from = new Date(to);
  from.setDate(from.getDate() - days);
  return {
    fromDate: dateInputValue(from),
    toDate: dateInputValue(to),
  };
}

export function formatNumber(value) {
  return Number(value ?? 0).toLocaleString();
}

export function formatOptional(value) {
  return value === null || value === undefined ? '-' : formatNumber(value);
}

export function formatPercent(value) {
  return `${(Number(value ?? 0) * 100).toFixed(1)}%`;
}

export function formatTime(value) {
  const date = new Date(Number(value));
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleString();
}

export function formatLatencyUs(value) {
  if (value === null || value === undefined) {
    return '-';
  }
  const micros = Number(value);
  if (!Number.isFinite(micros)) {
    return '-';
  }
  if (micros < 1_000) {
    return `${Math.round(micros)} us`;
  }
  if (micros < 1_000_000) {
    return `${(micros / 1_000).toFixed(1)} ms`;
  }
  return `${(micros / 1_000_000).toFixed(2)} s`;
}

export function tokenCategorySeries(rows = [], t = null) {
  const categories = new Map(rows.map((row) => [row.key, row]));
  return ['input', 'cache_hit', 'cache_miss', 'output', 'reasoning']
    .map((key) => categories.get(key))
    .filter(Boolean)
    .map((row) => ({
      key: row.key,
      label: tokenCategoryLabel(row.key, row.label, t),
      total: Number(row.total ?? 0),
      parentKey: tokenCategoryParent(row.key),
      color: tokenCategoryColor(row.key),
      points: Array.isArray(row.points) ? row.points : [],
    }));
}

function tokenCategoryLabel(key, fallback, t) {
  if (!t) {
    return fallback;
  }
  switch (key) {
    case 'input':
      return t('stats.llm.metrics.inputTokens');
    case 'output':
      return t('stats.llm.metrics.outputTokens');
    case 'reasoning':
      return t('stats.llm.metrics.reasoningTokens');
    case 'cache_hit':
      return t('stats.llm.metrics.cacheHitTokens');
    case 'cache_miss':
      return t('stats.llm.metrics.cacheMissTokens');
    default:
      return fallback;
  }
}

export function resolveBoundHiddenKeys({ key, hiddenKeys, series }) {
  const next = new Set(hiddenKeys);
  const target = series.find((item) => item.key === key);
  if (!target) {
    return Array.from(next);
  }
  const willHide = !next.has(key);
  setHidden(next, key, willHide);
  for (const descendant of descendantsOf(series, key)) {
    setHidden(next, descendant.key, willHide);
  }
  if (target.parentKey) {
    const siblings = series.filter((item) => item.parentKey === target.parentKey);
    const allSiblingsHidden = siblings.every((item) => next.has(item.key));
    setHidden(next, target.parentKey, allSiblingsHidden);
  }
  return Array.from(next);
}

export function resolvePartitionedVisibleSeries({ series, hiddenKeys }) {
  const visible = series.filter(
    (item) => !hiddenKeys.has(item.key) && !(item.parentKey && hiddenKeys.has(item.parentKey)),
  );
  const visibleChildParents = new Set(visible.map((item) => item.parentKey).filter(Boolean));
  return visible.filter((item) => item.parentKey || !visibleChildParents.has(item.key));
}

export function tokenCategoryColor(key) {
  switch (key) {
    case 'input':
      return 'var(--stats-chart-input)';
    case 'output':
      return 'var(--stats-chart-output)';
    case 'reasoning':
      return 'var(--stats-chart-reasoning)';
    case 'cache_hit':
      return 'var(--stats-chart-cache-hit)';
    case 'cache_miss':
      return 'var(--stats-chart-cache-miss)';
    default:
      return 'var(--stats-chart-total)';
  }
}

export function downloadText(filename, mimeType, text) {
  const blob = new Blob([text], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  URL.revokeObjectURL(url);
}

function dateInputValue(date) {
  if (!(date instanceof Date) || Number.isNaN(date.getTime())) {
    return '';
  }
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 10);
}

function localDateStart(value) {
  if (!value) {
    return null;
  }
  const date = new Date(`${value}T00:00:00`);
  return Number.isNaN(date.getTime()) ? null : date;
}

function tokenCategoryParent(key) {
  if (key === 'cache_hit' || key === 'cache_miss') {
    return 'input';
  }
  return null;
}

function descendantsOf(series, parentKey) {
  const children = series.filter((item) => item.parentKey === parentKey);
  return children.flatMap((child) => [child, ...descendantsOf(series, child.key)]);
}

function setHidden(hiddenKeys, key, hidden) {
  if (hidden) {
    hiddenKeys.add(key);
  } else {
    hiddenKeys.delete(key);
  }
}
