async function fetchJson(path, options = {}) {
  const response = await fetch(path, options);
  if (!response.ok) {
    const body = await response.text();
    throw new Error(`${response.status} ${response.statusText}: ${body}`);
  }
  return response.json();
}

export function clearServerCache() {
  return fetch('/api/cache/clear', { method: 'POST' }).then(async (response) => {
    if (!response.ok) {
      const body = await response.text();
      throw new Error(`${response.status} ${response.statusText}: ${body}`);
    }
    return response.json();
  });
}

export function listTraces({ signal, ...filters } = {}) {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value !== '' && value != null) query.set(key, String(value));
  }
  return fetchJson(`/api/traces?${query}`, { signal });
}

export function listAlerts(limit = null) {
  const query = limit == null ? '' : `?limit=${encodeURIComponent(limit)}`;
  return fetchJson(`/api/alerts${query}`);
}

/** Collector-daemon liveness as probed through the operator control socket. */
export function readDaemonStatus() {
  return fetchJson('/api/daemon/status');
}

/** Whether the console backend that served this page is still answering. */
export function readConsoleHealth() {
  return fetch('/health').then((response) => response.ok);
}

export function readAlert(alertId) {
  return fetchJson(`/api/alerts/${encodeURIComponent(alertId)}`);
}

export function readTraceAlerts(traceId, limit = null) {
  const query = limit == null ? '' : `?limit=${encodeURIComponent(limit)}`;
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/alerts${query}`);
}

export function readCurrentConfig() {
  return fetchJson('/api/config/current');
}

export function readPluginEnablement() {
  return fetchJson('/api/plugins/enabled');
}

export function readPluginRuntimeStatus() {
  return fetchJson('/api/plugins/runtime');
}

export function readPluginCatalog() {
  return fetchJson('/api/plugins/catalog');
}

export function loadDiscoveredPlugin(packageKey, options) {
  return fetchJson(`/api/plugins/catalog/load?package=${encodeURIComponent(packageKey)}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(options),
  });
}

export function unloadRuntimePlugin(instanceId) {
  return fetchJson(`/api/plugins/runtime/unload?instance_id=${encodeURIComponent(instanceId)}`, {
    method: 'POST',
  });
}

export function sendRuntimePluginCommand(instanceId, argv) {
  return fetchJson(`/api/plugins/runtime/command?instance_id=${encodeURIComponent(instanceId)}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ argv }),
  });
}

export function readRuntimePluginConfig(instanceId) {
  return fetchJson(`/api/plugins/runtime/config?instance_id=${encodeURIComponent(instanceId)}`);
}

export function validateRuntimePluginConfig(instanceId, config) {
  return fetchJson(`/api/plugins/runtime/config/validate?instance_id=${encodeURIComponent(instanceId)}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ config }),
  });
}

export function updateRuntimePluginConfig(instanceId, config) {
  return fetchJson(`/api/plugins/runtime/config?instance_id=${encodeURIComponent(instanceId)}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ config }),
  });
}

export function readTrace(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}`);
}

export function readTokenUsageStats({ fromMs, toMs, signal } = {}) {
  return fetchJson(`/api/stats/token-usage?from_ms=${fromMs}&to_ms=${toMs}`, { signal });
}

export function readLlmRequestsActivity({ fromMs, toMs, rollup, signal } = {}) {
  const rollupQuery = rollup && rollup !== 'auto' ? `&rollup=${encodeURIComponent(rollup)}` : '';
  return fetchJson(`/api/stats/llm-requests/activity?from_ms=${fromMs}&to_ms=${toMs}${rollupQuery}`, { signal });
}

export function readLlmRequestRows({ fromMs, toMs, offset = 0, limit = 50, signal } = {}) {
  return fetchJson(
    `/api/stats/llm-requests/rows?from_ms=${fromMs}&to_ms=${toMs}&offset=${offset}&limit=${limit}`,
    { signal },
  );
}

export function readTimeAttributionActivity({ fromMs, toMs, signal } = {}) {
  return fetchJson(
    `/api/stats/time-attribution/activity?from_ms=${fromMs}&to_ms=${toMs}`,
    { signal },
  );
}

export function readTimeAttributionRows({
  fromMs,
  toMs,
  offset = 0,
  limit = 50,
  dimension = null,
  key = null,
  signal,
} = {}) {
  const filter = dimension && key !== null
    ? `&dimension=${encodeURIComponent(dimension)}&key=${encodeURIComponent(key)}`
    : '';
  return fetchJson(
    `/api/stats/time-attribution/rows?from_ms=${fromMs}&to_ms=${toMs}&offset=${offset}&limit=${limit}${filter}`,
    { signal },
  );
}

export function runLlmRequestsExplore(query, { signal } = {}) {
  return fetchJson('/api/stats/llm-requests/explore', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(query),
    signal,
  });
}

export async function readLlmRequestsCsv({ fromMs, toMs, view = 'rows', signal } = {}) {
  const response = await fetch(
    `/api/stats/llm-requests/export.csv?from_ms=${fromMs}&to_ms=${toMs}&view=${encodeURIComponent(view)}`,
    { signal },
  );
  if (!response.ok) {
    const body = await response.text();
    throw new Error(`${response.status} ${response.statusText}: ${body}`);
  }
  return response.text();
}

export function readTraceSummary(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/summary`);
}

export function readTraceTimeAttribution(traceId, { signal } = {}) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/time-attribution`, { signal });
}

export function readTraceEvents(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/events`);
}

export function readTracePayloads(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/payloads`);
}

export function readTraceTimeline(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/timeline`);
}

export function readTraceProcesses(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/processes`);
}

export function readTraceDiagnostics(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/diagnostics`);
}

export function readTraceMetadata(traceId, { signal } = {}) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/metadata`, { signal });
}

export function readTraceCounts(traceId, { signal } = {}) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/counts`, { signal });
}

export function readTimelinePage(traceId, { signal, ...filters } = {}) {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value !== '' && value != null) query.set(key, String(value));
  }
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/actions/timeline?${query}`, { signal });
}

export function readIdleIntervals(traceId, { signal, ...filters } = {}) {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    if (value !== '' && value != null) query.set(key, String(value));
  }
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/idle-intervals?${query}`, { signal });
}

export function readActionTreeRoot(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/action-tree/root`);
}

export function readActionTreeChildren(traceId, parentId, { offset, limit }) {
  return fetchJson(
    `/api/traces/${encodeURIComponent(traceId)}/action-tree/children/${encodeURIComponent(parentId)}?offset=${offset}&limit=${limit}`,
  );
}

export function readActionTreeLlmNav(traceId, { mode, afterId = null } = {}) {
  const after = afterId ? `&after=${encodeURIComponent(afterId)}` : '';
  return fetchJson(
    `/api/traces/${encodeURIComponent(traceId)}/action-tree/llm-nav?mode=${encodeURIComponent(mode)}${after}`,
  );
}

export function readLlmTrajectoryGraph(traceId, { signal } = {}) {
  return fetchJson(
    `/api/traces/${encodeURIComponent(traceId)}/llm-trajectories`,
    { signal },
  );
}

export function readActionDetail(traceId, actionId, { signal } = {}) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/actions/${encodeURIComponent(actionId)}`, { signal });
}

export function readActionContentSource(traceId, actionId, { signal } = {}) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/actions/${encodeURIComponent(actionId)}/content/source`, { signal });
}

export function readActionHttpContent(traceId, actionId, { offset = 0, signal } = {}) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/actions/${encodeURIComponent(actionId)}/content/http?offset=${offset}`, { signal });
}

export function readActionFilePathSet(traceId, actionId, { offset, limit }) {
  return fetchJson(
    `/api/traces/${encodeURIComponent(traceId)}/actions/${encodeURIComponent(actionId)}/file-path-set?offset=${offset}&limit=${limit}`,
  );
}

export function readActionLlmRequestContent(traceId, actionId, { maxBytes, signal }) {
  return fetchJson(
    `/api/traces/${encodeURIComponent(traceId)}/actions/${encodeURIComponent(actionId)}/content/llm-request?max_bytes=${maxBytes}`,
    { signal },
  );
}

export function readActionMcpJsonRpcContent(traceId, actionId, { maxBytes }) {
  return fetchJson(
    `/api/traces/${encodeURIComponent(traceId)}/actions/${encodeURIComponent(actionId)}/content/mcp-jsonrpc?max_bytes=${maxBytes}`,
  );
}

export function readActionLlmRequestContentNode(traceId, actionId, { pointer, offset, limit, signal }) {
  const query = [
    `pointer=${encodeURIComponent(pointer)}`,
    `offset=${encodeURIComponent(offset)}`,
    `limit=${encodeURIComponent(limit)}`,
  ].join('&');
  return fetchJson(
    `/api/traces/${encodeURIComponent(traceId)}/actions/${encodeURIComponent(actionId)}/content/llm-request/node?${query}`,
    { signal },
  );
}

export function readCommands(traceId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/commands`);
}

export function readPayload(traceId, payloadId) {
  return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/payloads/${payloadId}`);
}

export function readTraceResources(traceId) { return fetchJson(`/api/traces/${encodeURIComponent(traceId)}/resources?limit=1000`); }
