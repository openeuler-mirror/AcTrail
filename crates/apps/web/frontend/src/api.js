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

export function listTraces() {
  return fetchJson('/api/traces');
}

export function listAlerts(limit = null) {
  const query = limit == null ? '' : `?limit=${encodeURIComponent(limit)}`;
  return fetchJson(`/api/alerts${query}`);
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
  return fetchJson(`/api/traces/${traceId}`);
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
  return fetchJson(`/api/traces/${traceId}/summary`);
}

export function readTraceEvents(traceId) {
  return fetchJson(`/api/traces/${traceId}/events`);
}

export function readTracePayloads(traceId) {
  return fetchJson(`/api/traces/${traceId}/payloads`);
}

export function readTraceTimeline(traceId) {
  return fetchJson(`/api/traces/${traceId}/timeline`);
}

export function readTraceProcesses(traceId) {
  return fetchJson(`/api/traces/${traceId}/processes`);
}

export function readTraceDiagnostics(traceId) {
  return fetchJson(`/api/traces/${traceId}/diagnostics`);
}

export function readActionTree(traceId) {
  return fetchJson(`/api/traces/${traceId}/action-tree`);
}

export function readActionTreeRoot(traceId) {
  return fetchJson(`/api/traces/${traceId}/action-tree/root`);
}

export function readActionTreeChildren(traceId, parentId, { offset, limit }) {
  return fetchJson(
    `/api/traces/${traceId}/action-tree/children/${encodeURIComponent(parentId)}?offset=${offset}&limit=${limit}`,
  );
}

export function readActionDetail(traceId, actionId) {
  return fetchJson(`/api/traces/${traceId}/actions/${encodeURIComponent(actionId)}`);
}

export function readActionFilePathSet(traceId, actionId, { offset, limit }) {
  return fetchJson(
    `/api/traces/${traceId}/actions/${encodeURIComponent(actionId)}/file-path-set?offset=${offset}&limit=${limit}`,
  );
}

export function readActionLlmRequestContent(traceId, actionId, { maxBytes }) {
  return fetchJson(
    `/api/traces/${traceId}/actions/${encodeURIComponent(actionId)}/content/llm-request?max_bytes=${maxBytes}`,
  );
}

export function readCommands(traceId) {
  return fetchJson(`/api/traces/${traceId}/commands`);
}

export function readPayload(traceId, payloadId) {
  return fetchJson(`/api/traces/${traceId}/payloads/${payloadId}`);
}
