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

export function readTrace(traceId) {
  return fetchJson(`/api/traces/${traceId}`);
}

export function readTokenUsageStats({ fromMs, toMs, signal } = {}) {
  return fetchJson(`/api/stats/token-usage?from_ms=${fromMs}&to_ms=${toMs}`, { signal });
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
