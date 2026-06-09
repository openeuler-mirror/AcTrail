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

export function readActionTree(traceId) {
  return fetchJson(`/api/traces/${traceId}/action-tree`);
}

export function readActionTreeRoot(traceId) {
  return fetchJson(`/api/traces/${traceId}/action-tree/root`);
}

export function readActionTreeChildren(traceId, parentId) {
  return fetchJson(`/api/traces/${traceId}/action-tree/children/${encodeURIComponent(parentId)}`);
}

export function readPayload(traceId, payloadId) {
  return fetchJson(`/api/traces/${traceId}/payloads/${payloadId}`);
}
