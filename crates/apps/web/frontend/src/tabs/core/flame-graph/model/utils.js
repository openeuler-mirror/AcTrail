
export const BACKGROUND_LABELS = Object.freeze({
  title_generation: 'Title generation',
  conversation_summary: 'Conversation summary',
  context_compaction: 'Context compaction',
  auxiliary_inferred: 'Background LLM (inferred)',
});

const AGENT_GRAPH_ROLES = new Set([
  'agent.invocation.exec',
  'agent.invocation.child_llm_request',
  'command.contains_file_access',
  'command.contains_command_invocation',
  'command.contains_llm_call',
  'command.contains_mcp_tool_call',
  'command.contains_process_exec',
  'command.contains_process_fork_attempt',
  'file.write.contains_file_event',
  'llm.call.request',
  'llm.call.response',
  'llm.response.tool_call',
  'llm.tool_call.result',
  'llm.tool_call.agent_invocation',
  'llm.request.llm_response',
  'llm.request.trajectory_parent',
  'llm.request.trajectory_fork',
  'mcp.tool_call.request',
  'mcp.tool_call.response',
  'mcp.request.stdout',
  'mcp.response.stdin',
]);

export const FLAME_GRAPH_ASSOCIATION_ROLES = new Set([
  'llm.response.tool_call',
  'llm.tool_call.result',
  'agent.invocation.child_llm_request',
  'llm.tool_call.agent_invocation',
  'llm.request.trajectory_parent',
  'llm.request.trajectory_fork',
]);

export const STRUCTURAL_KINDS = new Set([
  'agent.identity',
  'agent.exit',
]);

export const BACKGROUND_LLM_KINDS = new Set([
  'llm.call',
  'llm.request',
  'llm.response',
]);

export const AGENT_GROUP_ORDER = Object.freeze(['model', 'dialogue', 'tools', 'detail']);
export const HARNESS_GROUP_ORDER = Object.freeze([
  'model',
  'dialogue',
  'tools',
  'commands',
  'filesystem',
  'process',
  'runtime',
  'protocol',
]);

export const GROUP_LABELS = Object.freeze({
  model: 'LLM calls',
  dialogue: 'Messages',
  tools: 'Tool calls',
  detail: 'Tool effects',
  commands: 'Harness command',
  filesystem: 'Background file I/O',
  process: 'Process lifecycle',
  runtime: 'Runtime',
  protocol: 'Network / protocol',
});

export const GROUP_DESCRIPTIONS = Object.freeze({
  model: 'Model request lifecycle',
  dialogue: 'Request and assistant response',
  tools: 'Declared tools, results, and child agents',
  detail: 'File effects linked to Agent work',
  commands: 'Top-level harness execution',
  filesystem: 'Framework file activity without an Agent tool link',
  process: 'fork, vfork, clone, exec, and exit observations',
  runtime: 'Enforcement and uncategorized runtime activity',
  protocol: 'HTTP, SSE, and MCP transport details',
});

export function flameSummaryMarkerKind(kind) {
  return kind === 'file.tty_io';
}

export function observedInterval(action) {
  const startNanos = parseNanos(action.start_time_unix_nanos);
  return {
    startNanos,
    endNanos: action.end_time_unix_nanos
      ? parseNanos(action.end_time_unix_nanos)
      : startNanos,
  };
}

export function intervalOverlapNanos(interval, startNanos, endNanos) {
  const start = interval.startNanos > startNanos ? interval.startNanos : startNanos;
  const end = interval.endNanos < endNanos ? interval.endNanos : endNanos;
  return end >= start ? end - start : -1n;
}

export function graphChildren(links, actionById) {
  const children = new Map();
  for (const link of links ?? []) {
    if (!link.valid || !AGENT_GRAPH_ROLES.has(link.role)) {
      continue;
    }
    if (!actionById.has(link.parent) || !actionById.has(link.child)) {
      continue;
    }
    if (!children.has(link.parent)) {
      children.set(link.parent, []);
    }
    children.get(link.parent).push(link.child);
  }
  return children;
}

export function resolveCallRequest(call, actionById, childrenByParent) {
  const requestId = call.attributes?.['llm.call.request_action_id'];
  const direct = requestId ? actionById.get(requestId) : null;
  if (direct?.kind === 'llm.request') {
    return direct;
  }
  return (childrenByParent.get(call.id) ?? [])
    .map((id) => actionById.get(id))
    .find((action) => action?.kind === 'llm.request') ?? null;
}

export function collectDescendants(rootId, childrenByParent, destination) {
  const pending = [rootId];
  while (pending.length) {
    const id = pending.pop();
    if (!id || destination.has(id)) {
      continue;
    }
    destination.add(id);
    pending.push(...(childrenByParent.get(id) ?? []));
  }
}

export function packActivityDepths(activities) {
  const depthEnds = [];
  const packed = [];
  const sorted = [...activities].sort(compareActivities);
  for (const activity of sorted) {
    const start = activity.startOffsetMs;
    let depth = depthEnds.findIndex((endMs) => endMs <= start);
    if (depth < 0) {
      depth = depthEnds.length;
    }
    depthEnds[depth] = activityEndMs(activity);
    packed.push({ ...activity, depth });
  }
  return {
    activities: packed,
    depthCount: Math.max(depthEnds.length, 1),
  };
}

export function computeWindow(actions) {
  let startNanos = null;
  let endNanos = null;
  let startIso = null;
  let endIso = null;
  for (const action of actions) {
    const start = parseNanos(action.start_time_unix_nanos);
    const end = action.end_time_unix_nanos ? parseNanos(action.end_time_unix_nanos) : start;
    if (startNanos === null || start < startNanos) {
      startNanos = start;
      startIso = action.start_time;
    }
    if (endNanos === null || end > endNanos) {
      endNanos = end;
      endIso = action.end_time ?? action.start_time;
    }
  }
  return {
    startNanos,
    endNanos,
    spanMs: Math.max(nanosDiffMs(endNanos, startNanos), 1),
    startIso,
    endIso,
  };
}

export function coveredDuration(activities) {
  if (!activities.length) {
    return 0;
  }
  const intervals = activities
    .map((activity) => [activity.startOffsetMs, activityEndMs(activity)])
    .sort((left, right) => left[0] - right[0]);
  let total = 0;
  let [start, end] = intervals[0];
  for (const [nextStart, nextEnd] of intervals.slice(1)) {
    if (nextStart <= end) {
      end = Math.max(end, nextEnd);
    } else {
      total += end - start;
      start = nextStart;
      end = nextEnd;
    }
  }
  return total + end - start;
}

export function activityEnvelope(activities) {
  let startOffsetMs = activities.length ? Number.POSITIVE_INFINITY : 0;
  let endOffsetMs = activities.length ? Number.NEGATIVE_INFINITY : 0;
  for (const activity of activities) {
    startOffsetMs = Math.min(startOffsetMs, activity.startOffsetMs);
    endOffsetMs = Math.max(endOffsetMs, activityEndMs(activity));
  }
  return { startOffsetMs, endOffsetMs };
}

function activityEndMs(activity) {
  return activity.live
    ? activity.traceSpanMs
    : activity.startOffsetMs + Math.max(activity.durMs ?? 0, 0.001);
}

export function validTimedAction(action) {
  return Boolean(action?.id && action?.kind && action.start_time_unix_nanos != null);
}

export function compareActivities(left, right) {
  return left.startOffsetMs - right.startOffsetMs || String(left.id).localeCompare(String(right.id));
}

export function compareTimedActions(left, right) {
  const byStart = parseNanos(left.start_time_unix_nanos) - parseNanos(right.start_time_unix_nanos);
  if (byStart < 0n) {
    return -1;
  }
  if (byStart > 0n) {
    return 1;
  }
  return String(left.id).localeCompare(String(right.id));
}

export function parseNanos(value) {
  try {
    return BigInt(value ?? 0);
  } catch {
    return 0n;
  }
}

export function nanosDiffMs(later, earlier) {
  return Number(later - earlier) / 1_000_000;
}

export function humanize(value) {
  return String(value)
    .replace(/[._-]+/g, ' ')
    .replace(/^./, (letter) => letter.toUpperCase());
}

export function basename(value) {
  const normalized = String(value ?? '').replaceAll('\\', '/');
  return normalized.split('/').filter(Boolean).at(-1) ?? '';
}
