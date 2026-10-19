import { markRaw } from 'vue';
import { TAB_IDS } from '../../tabs/registry';

export function emptyActionTree(summary = null, rootData = null) {
  return {
    actions: [],
    links: [],
    roots: [],
    summary,
    rootData,
    loadedTraceId: null,
  };
}

export function emptyTrajectoryGraph() {
  return {
    nodes: [],
    edges: [],
    stats: {},
    capabilities: {},
    partial: false,
    loadedTraceId: null,
  };

}
export function emptyCommands() {
  return {
    actions: [],
    links: [],
    loadedTraceId: null,
  };
}

export function withCommandTrace(commandData, traceId) {
  return {
    actions: freezeTraceList(commandData.actions),
    links: freezeTraceList(commandData.links),
    loadedTraceId: traceId,
  };
}

export function freezeTraceList(items) {
  return (items ?? []).map((item) => markRaw(item));
}

export function freezeTracePayload(data) {
  if (!data || typeof data !== 'object') {
    return data;
  }
  const next = { ...data };
  for (const key of Object.keys(next)) {
    if (Array.isArray(next[key])) {
      next[key] = freezeTraceList(next[key]);
    }
  }
  return next;
}

export function eventBackedTab(tab) {
  return (
    tab === TAB_IDS.events ||
    tab === TAB_IDS.files ||
    tab === TAB_IDS.network
  );
}

export function traceIdMatches(left, right) {
  return String(left ?? '') === String(right ?? '');
}

export function compareTraceRecency(left, right) {
  const delta = traceCreatedNanos(right) - traceCreatedNanos(left);
  return delta === 0n ? 0 : delta > 0n ? 1 : -1;
}

function traceCreatedNanos(trace) {
  try {
    return BigInt(trace?.created_at_unix_nanos ?? 0);
  } catch {
    return 0n;
  }
}
