const TOKEN_FIELDS = Object.freeze([
  'prompt_tokens',
  'completion_tokens',
  'total_tokens',
  'cached_prompt_tokens',
  'reasoning_tokens',
  'prompt_cache_hit_tokens',
  'prompt_cache_miss_tokens',
]);

const TOKEN_PRICING_AXIS_DEFINITIONS = Object.freeze([
  {
    key: 'input',
    label: 'Input / Prompt',
    field: 'prompt_tokens',
    scope: 'Pricing axis',
  },
  {
    key: 'output',
    label: 'Output / Completion',
    field: 'completion_tokens',
    scope: 'Pricing axis',
  },
  {
    key: 'reasoning',
    label: 'Reasoning',
    field: 'reasoning_tokens',
    scope: 'Pricing axis',
  },
]);

export const TOKEN_CATEGORY_FILTERS = Object.freeze([
  { id: 'input', label: 'Input', field: 'prompt_tokens' },
  { id: 'output', label: 'Output', field: 'completion_tokens' },
  { id: 'reasoning', label: 'Reasoning', field: 'reasoning_tokens' },
]);

const INPUT_CACHE_DETAIL_DEFINITIONS = Object.freeze([
  {
    key: 'cache_hit',
    label: 'Prompt Cache Hit',
    value: (totals) => totals.input_cache_hit_tokens,
    scope: 'Input cache detail',
  },
  {
    key: 'cache_miss',
    label: 'Prompt Cache Miss',
    value: (totals) => totals.prompt_cache_miss_tokens,
    scope: 'Input cache detail',
  },
]);

export const TOKEN_TIME_BUCKETS = Object.freeze([
  { id: 'request', label: 'Request' },
  { id: 'minute', label: 'Minute' },
  { id: 'hour', label: 'Hour' },
  { id: 'day', label: 'Day' },
  { id: 'week', label: 'Week' },
  { id: 'month', label: 'Month' },
]);

export function defaultDateRange(traces = []) {
  const times = traces
    .map((trace) => Number(trace.created_at))
    .filter((value) => Number.isFinite(value) && value > 0);
  const now = Date.now();
  const minMs = times.length ? Math.min(...times) : now;
  const maxMs = times.length ? Math.max(now, Math.max(...times)) : now;
  return {
    fromDate: dateInputValue(new Date(minMs)),
    toDate: dateInputValue(new Date(maxMs)),
  };
}

export function rangeToMillis({ fromDate, toDate }) {
  const from = localDateStart(fromDate);
  const to = localDateStart(toDate);
  if (!from || !to) {
    return { ok: false, error: 'Select a valid date range.' };
  }
  const toExclusive = new Date(to);
  toExclusive.setDate(toExclusive.getDate() + 1);
  if (from.getTime() >= toExclusive.getTime()) {
    return { ok: false, error: 'Start date must be on or before end date.' };
  }
  return {
    ok: true,
    fromMs: from.getTime(),
    toMs: toExclusive.getTime(),
  };
}

export function buildTokenSeriesByBucket(requests, bucketId) {
  return buildTokenBuckets(requests, bucketId).map((bucket) => stripSets(bucket));
}

export function buildTokenBreakdownByBucket(requests, bucketId) {
  return buildTokenBuckets(requests, bucketId).map((bucket) => ({
    ...stripSets(bucket),
    model_count: bucket.modelIds.size,
    trace_count: bucket.traceIds.size,
  }));
}

export function buildTokenBreakdownByModel(requests) {
  const buckets = new Map();
  for (const request of requests ?? []) {
    const model = request.model || '(unknown)';
    if (!buckets.has(model)) {
      buckets.set(model, emptyTokenBucket({ model, traceIds: new Set() }));
    }
    const bucket = buckets.get(model);
    bucket.response_count += 1;
    bucket.traceIds.add(request.trace_id);
    if (!requestHasUsage(request)) {
      bucket.missing_usage_count += 1;
    }
    addTokenFields(bucket, request);
  }
  return Array.from(buckets.values())
    .map((bucket) => ({
      ...stripSets(bucket),
      trace_count: bucket.traceIds.size,
    }))
    .sort((left, right) =>
      right.total_tokens - left.total_tokens ||
      right.response_count - left.response_count ||
      left.model.localeCompare(right.model),
    );
}

export function buildTokenSummary(requests) {
  const modelIds = new Set();
  const traceIds = new Set();
  const summary = emptyTokenBucket({
    trace_count: 0,
    model_count: 0,
    usage_response_count: 0,
  });
  for (const request of requests ?? []) {
    summary.response_count += 1;
    traceIds.add(request.trace_id);
    modelIds.add(modelName(request));
    if (requestHasUsage(request)) {
      summary.usage_response_count += 1;
    }
    addTokenFields(summary, request);
  }
  summary.trace_count = traceIds.size;
  summary.model_count = modelIds.size;
  return stripSets(summary);
}

export function buildTokenBreakdownByCategory(requests, categories = allTokenCategoryIds()) {
  const totals = { ...emptyTokenBucket(), input_cache_hit_tokens: 0 };
  for (const request of requests ?? []) {
    for (const field of TOKEN_FIELDS) {
      totals[field] += numericToken(request[field]);
    }
    totals.input_cache_hit_tokens += inputCacheHitTokens(request);
  }
  const denominator = Math.max(1, numericToken(totals.total_tokens));
  const inputDenominator = Math.max(1, numericToken(totals.prompt_tokens));
  const categorySet = new Set(categories);
  const axisRows = TOKEN_PRICING_AXIS_DEFINITIONS.filter((definition) =>
    categorySet.has(definition.key),
  ).map((definition) => {
    const tokens = totals[definition.field];
    return {
      ...definition,
      level: 0,
      tokens,
      share: tokens / denominator,
      shareBasis: 'total',
    };
  });
  const inputCacheRows = categorySet.has('input')
    ? inputCacheRowsFromTotals(totals, inputDenominator)
    : [];
  const rows = axisRows.flatMap((row) =>
    row.key === 'input' ? [row, ...inputCacheRows] : [row],
  );
  const hasAnyCategoryUsage = rows.some((row) => row.tokens > 0);
  return hasAnyCategoryUsage ? rows : [];
}

export function listTokenModels(requests) {
  return Array.from(new Set((requests ?? []).map(modelName))).sort((left, right) =>
    left.localeCompare(right),
  );
}

export function allTokenCategoryIds() {
  return TOKEN_CATEGORY_FILTERS.map((category) => category.id);
}

export function applyTokenFilters(
  requests,
  { query = '', models = [], categories = [], validOnly = true } = {},
) {
  const modelSet = new Set(models);
  return filterTokenRequests(requests, query)
    .filter((request) => modelSet.has(modelName(request)))
    .filter((request) => !validOnly || requestHasUsage(request))
    .map((request) => projectRequestCategories(request, categories));
}

export function filterTokenRequests(requests, query) {
  const normalized = String(query ?? '').trim().toLowerCase();
  if (!normalized) {
    return requests ?? [];
  }
  return (requests ?? []).filter((request) =>
    [
      request.trace_id,
      request.trace_name,
      request.response_action_id,
      request.request_action_id,
      request.model,
      request.provider_id,
      formatTime(request.started_at_ms),
    ].some((value) => String(value ?? '').toLowerCase().includes(normalized)),
  );
}

export function categoryFlags(categories) {
  const categorySet = new Set(categories);
  return {
    input: categorySet.has('input'),
    output: categorySet.has('output'),
    reasoning: categorySet.has('reasoning'),
  };
}

export function formatNumber(value) {
  return Number(value ?? 0).toLocaleString();
}

export function formatOptionalNumber(value) {
  return value === null || value === undefined ? '-' : formatNumber(value);
}

export function formatTime(value) {
  const date = new Date(Number(value));
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleString();
}

export function shortDate(value) {
  const date = localDateStart(value);
  if (!date) {
    return value;
  }
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

function buildTokenBuckets(requests, bucketId) {
  const sortedRequests = [...(requests ?? [])].sort(
    (left, right) => numericTime(left.started_at_ms) - numericTime(right.started_at_ms),
  );
  if (bucketId === 'request') {
    return sortedRequests.map((request, index) => {
      const startedAtMs = numericTime(request.started_at_ms);
      const bucket = emptyTokenBucket({
        bucket_key: String(request.response_action_id ?? `${startedAtMs}:${index}`),
        bucket_label: `#${index + 1}`,
        bucket_detail: formatTime(startedAtMs),
        bucket_start_ms: startedAtMs,
        response_count: 1,
        modelIds: new Set(),
        traceIds: new Set(),
      });
      bucket.traceIds.add(request.trace_id);
      if (request.model) {
        bucket.modelIds.add(request.model);
      }
      addTokenFields(bucket, request);
      return bucket;
    });
  }

  const buckets = new Map();
  for (const request of sortedRequests) {
    const bucket = bucketForTime(request.started_at_ms, bucketId);
    if (!bucket) {
      continue;
    }
    if (!buckets.has(bucket.key)) {
      buckets.set(
        bucket.key,
        emptyTokenBucket({
          bucket_key: bucket.key,
          bucket_label: bucket.label,
          bucket_detail: bucket.detail,
          bucket_start_ms: bucket.startMs,
          modelIds: new Set(),
          traceIds: new Set(),
        }),
      );
    }
    const target = buckets.get(bucket.key);
    target.response_count += 1;
    target.traceIds.add(request.trace_id);
    if (request.model) {
      target.modelIds.add(request.model);
    }
    addTokenFields(target, request);
  }
  return Array.from(buckets.values()).sort(
    (left, right) => left.bucket_start_ms - right.bucket_start_ms,
  );
}

function emptyTokenBucket(extra = {}) {
  return {
    response_count: 0,
    missing_usage_count: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
    cached_prompt_tokens: 0,
    reasoning_tokens: 0,
    prompt_cache_hit_tokens: 0,
    prompt_cache_miss_tokens: 0,
    bucket_key: '',
    bucket_label: '',
    bucket_detail: '',
    bucket_start_ms: 0,
    ...extra,
  };
}

function addTokenFields(target, source) {
  for (const field of TOKEN_FIELDS) {
    target[field] += numericToken(source[field]);
  }
  if (!requestHasUsage(source)) {
    target.missing_usage_count += 1;
  }
}

function requestHasUsage(request) {
  return TOKEN_FIELDS.some((field) => request?.[field] !== null && request?.[field] !== undefined);
}

function numericToken(value) {
  const number = Number(value ?? 0);
  return Number.isFinite(number) && number > 0 ? number : 0;
}

function numericTime(value) {
  const number = Number(value ?? 0);
  return Number.isFinite(number) && number > 0 ? number : 0;
}

function modelName(request) {
  return request?.model || '(unknown)';
}

function inputCacheRowsFromTotals(totals, inputDenominator) {
  return INPUT_CACHE_DETAIL_DEFINITIONS.map((definition) => {
    const tokens = definition.value(totals);
    return {
      key: definition.key,
      label: definition.label,
      scope: definition.scope,
      level: 1,
      parentKey: 'input',
      tokens,
      share: tokens / inputDenominator,
      shareBasis: 'input',
    };
  }).filter((row) => row.tokens > 0);
}

function projectRequestCategories(request, categories) {
  const flags = categoryFlags(categories);
  const selectedAll = TOKEN_CATEGORY_FILTERS.every((category) => flags[category.id]);
  const selectedTotal =
    (flags.input ? numericToken(request.prompt_tokens) : 0) +
    (flags.output ? numericToken(request.completion_tokens) : 0) +
    (flags.reasoning ? numericToken(request.reasoning_tokens) : 0);
  return {
    ...request,
    prompt_tokens: flags.input ? request.prompt_tokens : null,
    cached_prompt_tokens: flags.input ? request.cached_prompt_tokens : null,
    prompt_cache_hit_tokens: flags.input ? request.prompt_cache_hit_tokens : null,
    prompt_cache_miss_tokens: flags.input ? request.prompt_cache_miss_tokens : null,
    completion_tokens: flags.output ? request.completion_tokens : null,
    reasoning_tokens: flags.reasoning ? request.reasoning_tokens : null,
    total_tokens: selectedAll ? request.total_tokens : selectedTotal,
  };
}

function inputCacheHitTokens(source) {
  const promptCacheHit = numericToken(source.prompt_cache_hit_tokens);
  if (promptCacheHit > 0) {
    return promptCacheHit;
  }
  return numericToken(source.cached_prompt_tokens);
}

function stripSets(bucket) {
  const { modelIds, traceIds, ...rest } = bucket;
  return rest;
}

function bucketForTime(value, bucketId) {
  const startedAtMs = numericTime(value);
  if (!startedAtMs) {
    return null;
  }
  const start = bucketStart(new Date(startedAtMs), bucketId);
  if (!start) {
    return null;
  }
  return {
    key: `${bucketId}:${start.getTime()}`,
    label: bucketLabel(start, bucketId),
    detail: bucketDetail(start, bucketId),
    startMs: start.getTime(),
  };
}

function bucketStart(date, bucketId) {
  const start = new Date(date);
  start.setSeconds(0, 0);
  if (bucketId === 'minute') {
    return start;
  }
  start.setMinutes(0, 0, 0);
  if (bucketId === 'hour') {
    return start;
  }
  start.setHours(0, 0, 0, 0);
  if (bucketId === 'day') {
    return start;
  }
  if (bucketId === 'week') {
    const mondayOffset = (start.getDay() + 6) % 7;
    start.setDate(start.getDate() - mondayOffset);
    return start;
  }
  if (bucketId === 'month') {
    start.setDate(1);
    return start;
  }
  return null;
}

function bucketLabel(date, bucketId) {
  if (bucketId === 'minute') {
    return date.toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  }
  if (bucketId === 'hour') {
    return date.toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
    });
  }
  if (bucketId === 'day') {
    return dateInputValue(date);
  }
  if (bucketId === 'week') {
    return `Week of ${dateInputValue(date)}`;
  }
  if (bucketId === 'month') {
    return date.toLocaleString(undefined, { year: 'numeric', month: 'short' });
  }
  return formatTime(date.getTime());
}

function bucketDetail(date, bucketId) {
  if (bucketId === 'week') {
    const end = new Date(date);
    end.setDate(end.getDate() + 6);
    return `${dateInputValue(date)} - ${dateInputValue(end)}`;
  }
  return bucketLabel(date, bucketId);
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
