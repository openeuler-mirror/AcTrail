async function fetchJson(path) {
  const response = await fetch(path);
  if (!response.ok) {
    const body = await response.text();
    throw new Error(`${response.status} ${response.statusText}: ${body}`);
  }
  return response.json();
}

export function listTraces() {
  return fetchJson('/api/traces');
}

export function readTrace(traceId) {
  return fetchJson(`/api/traces/${traceId}`);
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

export function readActionTreeChildren(traceId, parentId) {
  return fetchJson(`/api/traces/${traceId}/action-tree/children/${encodeURIComponent(parentId)}`);
}

export function readCommands(traceId) {
  return fetchJson(`/api/traces/${traceId}/commands`);
}

export function readPayload(traceId, payloadId) {
  return fetchJson(`/api/traces/${traceId}/payloads/${payloadId}`);
}
