import { compactRows, kindClass, shortTime } from '../../action-tree/common.js';
import { isBashWrapperCommand, semanticActionLabel, semanticActionTarget } from '../../../actionLabels.js';
import { ToolCallDisplay } from '../../../shared/toolCallDisplay.js';
import { attachLlmCallDetails, ensureLlmMessages, llmMessagesFromAction } from './llm.js';
import { toBigInt, nanosDiffMs, microsLabel, formatOffset } from './utils.js';

export const WATERFALL_METRICS = Object.freeze([
  { key: 'duration', label: 'Duration', get: (action) => action.duration },
  {
    key: 'ttft',
    label: 'TTFT',
    get: (action) => microsLabel(action.attributes?.['llm.first_token_us']),
  },
  {
    key: 'output_tokens',
    label: 'Output tokens',
    get: (action) => action.attributes?.['llm.response.completion_tokens'],
  },
  {
    key: 'total_tokens',
    label: 'Total tokens',
    get: (action) => action.attributes?.['llm.response.total_tokens'],
  },
  {
    key: 'model',
    label: 'Model',
    get: (action, llmMessages) =>
      llmMessages?.model ??
      action.attributes?.['llm.call.model'] ??
      action.attributes?.['llm.response.model'],
  },
  {
    key: 'exit_code',
    label: 'Exit code',
    get: (action) =>
      action.attributes?.['command.exit_code'] ??
      action.attributes?.['process.exit_status'] ??
      action.attributes?.['process.exit_code'],
  },
]);

const KIND_GROUPS = Object.freeze({
  'llm.call': 'llm',
  'llm.request': 'llm',
  'llm.response': 'llm',
  'sse.stream': 'sse',
  'sse.event': 'sse',
  'http.message': 'http',
  'command.invocation': 'command',
  'agent.invocation': 'command',
  'process.exec': 'process',
  'process.fork_attempt': 'process',
  'file.read': 'file',
  'file.write': 'file',
  'file.modify': 'file',
  'file.tty_io': 'file',
  'file.bulk_read': 'file',
  'fs.enumerate': 'file',
  'enforcement.decision': 'enforcement',
});

export function kindGroup(kind) {
  return KIND_GROUPS[kind] ?? 'other';
}

// Default Waterfall legend selection: keep noisy groups off until the user
// explicitly enables them on large traces.
export const WATERFALL_DEFAULT_ACTIVE_GROUPS = Object.freeze(['command', 'llm']);

export function defaultActiveGroups(groups) {
  const available = new Set((groups ?? []).map((group) => group.group));
  return new Set(WATERFALL_DEFAULT_ACTIVE_GROUPS.filter((group) => available.has(group)));
}

export function buildWaterfall(actions, links, idleIntervals = [], axisEndNanos = null) {
  const validActions = actions ?? [];
  const intervals = (idleIntervals ?? []).map(idleIntervalNode);
  const window = computeWindow(validActions, intervals, axisEndNanos);
  if (!validActions.length) {
    return { roots: [], window, groups: [], totalActions: 0, idleIntervals: intervals };
  }

  const toolDisplay = new ToolCallDisplay(new Map(validActions.map((action) => [action.id, action])));
  const nodeById = new Map(
    validActions.map((action) => [action.id, actionNode(action, window, toolDisplay)]),
  );
  const childrenByParent = groupChildren(links, nodeById);
  const childIds = new Set();
  for (const children of childrenByParent.values()) {
    for (const child of children) {
      childIds.add(child);
    }
  }

  const parentByChild = new Map();
  for (const [parentId, childIds] of childrenByParent) {
    for (const childId of childIds) {
      parentByChild.set(childId, parentId);
    }
  }

  const placed = new Set();
  const attach = (node) => {
    if (placed.has(node.id)) {
      return null;
    }
    placed.add(node.id);
    const childList = childrenByParent.get(node.id) ?? [];
    node.children = childList
      .map((childId) => nodeById.get(childId))
      .filter(Boolean)
      .map(attach)
      .filter(Boolean)
      .sort(compareNodes);
    node.hasChildren = node.children.length > 0;
    if (node.kind === 'llm.call') {
      attachLlmCallDetails(node, nodeById, parentByChild, window);
    }
    return node;
  };

  const roots = validActions
    .filter((action) => !childIds.has(action.id))
    .map((action) => nodeById.get(action.id))
    .filter(Boolean)
    .map(attach)
    .filter(Boolean)
    .sort(compareNodes);

  // Defensive: surface any node not reachable from a root (cyclic links).
  for (const node of nodeById.values()) {
    if (!placed.has(node.id)) {
      attach(node);
      roots.push(node);
    }
  }
  roots.sort(compareNodes);

  return {
    roots,
    window,
    groups: groupSummary(validActions),
    totalActions: validActions.length,
    idleIntervals: intervals,
  };
}

// Map read-time user lifecycle intervals to standalone timeline segments.
export function idleIntervalRows(intervals, window) {
  return (intervals ?? []).map((interval) => {
    const startOffsetMs = nanosDiffMs(interval.startNanos, window.startNanos);
    const endOffsetMs = interval.live
      ? null
      : nanosDiffMs(interval.endNanos, window.startNanos);
    return {
      ...interval,
      startOffsetMs,
      endOffsetMs,
      durMs: interval.live ? null : Math.max(endOffsetMs - startOffsetMs, 0),
    };
  });
}

function idleIntervalNode(interval) {
  const startNanos = toBigInt(interval.start_time_unix_nanos);
  const endNanos = interval.end_time_unix_nanos
    ? toBigInt(interval.end_time_unix_nanos)
    : null;
  return {
    id: String(interval.id ?? interval.interval_id),
    taskId: interval.task_id ?? '',
    sessionId: interval.session_id ?? '',
    kind: interval.kind,
    startNanos,
    endNanos,
    live: endNanos === null,
  };
}

export function findWaterfallNode(roots, id) {
  for (const node of roots) {
    if (node.id === id) {
      return node;
    }
    const found = findWaterfallNode(node.children, id);
    if (found) {
      return found;
    }
  }
  return null;
}

export function findWaterfallPath(roots, id) {
  for (const node of roots) {
    if (node.id === id) {
      return [node];
    }
    const childPath = findWaterfallPath(node.children, id);
    if (childPath.length) {
      return [node, ...childPath];
    }
  }
  return [];
}

// Time bounds (offset ms from capture start) covering a node and all of its
// descendants. Live (unfinished) actions extend to the global span end.
export function subtreeWindow(node, globalSpanMs) {
  let startMs = node.startOffsetMs;
  let endMs = nodeEndMs(node, globalSpanMs);
  const walk = (current) => {
    startMs = Math.min(startMs, current.startOffsetMs);
    endMs = Math.max(endMs, nodeEndMs(current, globalSpanMs));
    for (const child of current.children) {
      walk(child);
    }
  };
  walk(node);
  return { startMs, spanMs: Math.max(endMs - startMs, 1) };
}

function nodeEndMs(node, globalSpanMs) {
  if (node.live) {
    return globalSpanMs;
  }
  return node.startOffsetMs + (node.durMs ?? 0);
}

export function collectParentIds(roots) {
  const ids = [];
  const walk = (nodes) => {
    for (const node of nodes) {
      if (node.hasChildren) {
        ids.push(node.id);
        walk(node.children);
      }
    }
  };
  walk(roots);
  return ids;
}

export function collectDefaultExpandedIds(roots) {
  const ids = [];
  const walk = (nodes) => {
    for (const node of nodes) {
      if (!node.hasChildren) {
        continue;
      }
      if (node.kind !== 'llm.call' && !isBashWrapperCommand(node.action)) {
        ids.push(node.id);
        walk(node.children);
      }
    }
  };
  walk(roots);
  return ids;
}

export function flattenVisibleWaterfall(roots, expandedIds, activeGroups) {
  const out = [];
  const walk = (nodes, depth) => {
    for (const node of nodes) {
      if (!subtreeMatchesGroup(node, activeGroups)) {
        continue;
      }
      const expanded = node.hasChildren && expandedIds.has(node.id);
      out.push(rowFromNode(node, depth, expanded));
      if (expanded) {
        walk(node.children, depth + 1);
      }
    }
  };
  walk(roots, 0);
  return out;
}

export function flattenMatchingWaterfall(roots, query, activeGroups) {
  const out = [];
  const walk = (nodes, depth) => {
    for (const node of nodes) {
      if (!subtreeMatchesGroup(node, activeGroups)) {
        continue;
      }
      if (!subtreeMatchesQuery(node, query)) {
        continue;
      }
      out.push(rowFromNode(node, depth, node.hasChildren));
      walk(node.children, depth + 1);
    }
  };
  walk(roots, 0);
  return out;
}

export function actionDetail(action, llmMessages = null, target = semanticActionTarget(action)) {
  const label = semanticActionLabel(action);
  const messages = llmMessages ?? llmMessagesFromAction(action);
  return {
    selectionId: action.id,
    title: label,
    kind: label,
    rows: compactRows({
      semantic_label: label,
      raw_action_kind: action.kind,
      target,
      status: ToolCallDisplay.statusLabel(action),
      completeness: action.completeness,
      pid: action.process?.pid,
      started: action.start_time,
      ended: action.end_time,
      request_message: messages?.requestFull,
      response_message: messages?.responseFull,
      agent_scope: messages?.scope,
      parent_command: messages?.parent,
      ttft: messages?.ttft,
      ...metricRows(action, messages),
    }),
    attributes: {
      ...(action.attributes ?? {}),
      ...(messages?.requestFull
        ? { 'llm.request.message_preview': messages.requestFull }
        : {}),
      ...(messages?.responseFull
        ? { 'llm.response.message_preview': messages.responseFull }
        : {}),
    },
    evidence: action.evidence ?? [],
    raw: action,
  };
}

function metricRows(action, llmMessages = null) {
  const rows = {};
  for (const metric of WATERFALL_METRICS) {
    const value = metric.get(action, llmMessages);
    if (value !== undefined && value !== null && value !== '') {
      rows[metric.key] = value;
    }
  }
  return rows;
}

function rowFromNode(node, depth, expanded) {
  const llmMessages = ensureLlmMessages(node);
  return {
    id: node.id,
    depth,
    hasChildren: node.hasChildren,
    expanded,
    kind: node.kind,
    kindGroup: node.kindGroup,
    kindClass: node.kindClass,
    label: node.label,
    target: node.target,
    status: node.status,
    statusLabel: ToolCallDisplay.statusLabel(node.action),
    durationLabel: node.action.duration ?? null,
    live: node.live,
    startOffsetMs: node.startOffsetMs,
    durMs: node.durMs,
    startClock: shortTime(node.action.start_time) || '',
    startOffsetLabel: `+${formatOffset(node.startOffsetMs)}`,
    durationText: node.live
      ? 'running…'
      : node.action.duration ?? formatOffset(node.durMs ?? 0),
    llmRequestPreview: llmMessages?.requestPreview ?? '',
    llmResponsePreview: llmMessages?.responsePreview ?? '',
    llmMessages,
    llmPhases: node.llmPhases ?? null,
    llmScope: node.llmContext?.scopeLabel ?? '',
    agentContext: node.llmContext?.parentLabel ?? '',
    metrics: tooltipMetrics(node.action, llmMessages),
    action: node.action,
  };
}

function tooltipMetrics(action, llmMessages = null) {
  return WATERFALL_METRICS.map((metric) => ({
    label: metric.label,
    value: metric.get(action, llmMessages),
  })).filter((item) => item.value !== undefined && item.value !== null && item.value !== '');
}

function actionNode(action, window, toolDisplay) {
  const startNanos = toBigInt(action.start_time_unix_nanos);
  const endNanos = action.end_time_unix_nanos ? toBigInt(action.end_time_unix_nanos) : null;
  const startOffsetMs = nanosDiffMs(startNanos, window.startNanos);
  const durMs = endNanos === null ? null : nanosDiffMs(endNanos, startNanos);
  return {
    id: action.id,
    action,
    kind: action.kind,
    kindGroup: kindGroup(action.kind),
    kindClass: kindClass(action.kind),
    label: semanticActionLabel(action) || action.kind,
    target: toolDisplay.target(action),
    status: action.status,
    startOffsetMs,
    durMs,
    live: endNanos === null,
    children: [],
    hasChildren: false,
  };
}

// Roles that represent a broad "the agent performed this action" bucket rather
// than tight containment. An action can be linked to several parents (e.g. an
// llm.response links both to its llm.request via `llm.request.llm_response` and
// to the agent process via `agent.performed_action`). We prefer the specific
// containment parent so pairs like request/response nest instead of becoming
// siblings.
const LOW_PRIORITY_LINK_ROLES = new Set(['agent.performed_action']);

function linkPriority(role) {
  return LOW_PRIORITY_LINK_ROLES.has(role) ? 0 : 1;
}

function groupChildren(links, nodeById) {
  const bestParent = new Map();
  for (const link of links ?? []) {
    const parent = link.parent;
    const child = link.child;
    if (!parent || !child || parent === child) {
      continue;
    }
    if (!nodeById.has(parent) || !nodeById.has(child)) {
      continue;
    }
    const priority = linkPriority(link.role);
    const current = bestParent.get(child);
    if (!current || priority > current.priority) {
      bestParent.set(child, { parent, priority });
    }
  }

  const map = new Map();
  for (const [child, { parent }] of bestParent) {
    if (!map.has(parent)) {
      map.set(parent, []);
    }
    map.get(parent).push(child);
  }
  return map;
}

function computeWindow(actions, intervals = [], axisEndNanos = null) {
  let startNanos = null;
  let endNanos = null;
  let startIso = null;
  let endIso = null;
  const consider = (start, end, isoStart, isoEnd) => {
    if (startNanos === null || start < startNanos) {
      startNanos = start;
      startIso = isoStart;
    }
    if (endNanos === null || end > endNanos) {
      endNanos = end;
      endIso = isoEnd;
    }
  };
  for (const action of actions) {
    const start = toBigInt(action.start_time_unix_nanos);
    const end = action.end_time_unix_nanos ? toBigInt(action.end_time_unix_nanos) : start;
    consider(start, end, action.start_time, action.end_time ?? action.start_time);
  }
  for (const interval of intervals) {
    consider(
      interval.startNanos,
      interval.endNanos ?? interval.startNanos,
      null,
      null,
    );
  }
  if (axisEndNanos) {
    const axisEnd = toBigInt(axisEndNanos);
    if (endNanos === null || axisEnd > endNanos) {
      endNanos = axisEnd;
    }
  }
  if (startNanos === null) {
    return emptyWindow();
  }
  const spanMs = Math.max(nanosDiffMs(endNanos, startNanos), 1);
  return { startNanos, endNanos, spanMs, startIso, endIso };
}

function emptyWindow() {
  return { startNanos: 0n, endNanos: 0n, spanMs: 1, startIso: null, endIso: null };
}

function groupSummary(actions) {
  const counts = new Map();
  for (const action of actions) {
    const group = kindGroup(action.kind);
    counts.set(group, (counts.get(group) ?? 0) + 1);
  }
  return Array.from(counts.entries())
    .map(([group, count]) => ({ group, count }))
    .sort((left, right) => right.count - left.count);
}

function subtreeMatchesGroup(node, activeGroups) {
  if (!activeGroups) {
    return true;
  }
  if (activeGroups.has(node.kindGroup)) {
    return true;
  }
  return node.children.some((child) => subtreeMatchesGroup(child, activeGroups));
}

function subtreeMatchesQuery(node, query) {
  if (nodeMatchesQuery(node, query)) {
    return true;
  }
  return node.children.some((child) => subtreeMatchesQuery(child, query));
}

function nodeMatchesQuery(node, query) {
  const messages = ensureLlmMessages(node);
  const llmText = [
    messages?.requestFull,
    messages?.responseFull,
    messages?.requestPreview,
    messages?.responsePreview,
    node.llmContext?.parentLabel,
    node.llmContext?.scopeLabel,
  ];
  return [node.label, node.target, node.kind, ToolCallDisplay.statusLabel(node.action), node.action.duration, ...llmText]
    .filter(Boolean)
    .join(' ')
    .toLowerCase()
    .includes(query);
}

export function emptyWaterfallModel() {
  return { roots: [], window: emptyWindow(), groups: [], totalActions: 0 };
}

function compareNodes(left, right) {
  if (left.startOffsetMs !== right.startOffsetMs) {
    return left.startOffsetMs - right.startOffsetMs;
  }
  return String(left.id).localeCompare(String(right.id));
}
