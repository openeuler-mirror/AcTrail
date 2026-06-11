import { compactRows, kindClass, shortTime } from '../action-tree/common';
import { isBashWrapperCommand, semanticActionLabel, semanticActionTarget } from '../../actionLabels';

const ACTION_VALID_ATTR = 'actrail.action.valid';
const ACTION_VALID_FALSE = 'false';

// Single declarative extension seam: add a line here to surface a new metric in
// every bar's tooltip and the detail panel. Attribute-based metrics need no
// backend change because action_json passes `attributes` through untouched.
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
    get: (action) =>
      action.attributes?.['llm.call.model'] ?? action.attributes?.['llm.response.model'],
  },
  {
    key: 'exit_code',
    label: 'Exit code',
    get: (action) =>
      action.attributes?.['process.exit_status'] ?? action.attributes?.['process.exit_code'],
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
  'enforcement.decision': 'enforcement',
});

export function kindGroup(kind) {
  return KIND_GROUPS[kind] ?? 'other';
}

export function buildWaterfall(actions, links) {
  const validActions = (actions ?? []).filter((action) => !invalidatedAction(action));
  if (!validActions.length) {
    return { roots: [], window: emptyWindow(), groups: [], totalActions: 0 };
  }

  const window = computeWindow(validActions);
  const nodeById = new Map(
    validActions.map((action) => [action.id, actionNode(action, window)]),
  );
  const childrenByParent = groupChildren(links, nodeById);
  const childIds = new Set();
  for (const children of childrenByParent.values()) {
    for (const child of children) {
      childIds.add(child);
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
      if (!isBashWrapperCommand(node.action)) {
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

export function actionDetail(action) {
  const label = semanticActionLabel(action);
  const target = semanticActionTarget(action);
  return {
    selectionId: action.id,
    title: label,
    kind: label,
    rows: compactRows({
      semantic_label: label,
      raw_action_kind: action.kind,
      target,
      status: action.status,
      completeness: action.completeness,
      pid: action.process?.pid,
      started: action.start_time,
      ended: action.end_time,
      ...metricRows(action),
    }),
    attributes: action.attributes ?? {},
    evidence: action.evidence ?? [],
    raw: action,
  };
}

function metricRows(action) {
  const rows = {};
  for (const metric of WATERFALL_METRICS) {
    const value = metric.get(action);
    if (value !== undefined && value !== null && value !== '') {
      rows[metric.key] = value;
    }
  }
  return rows;
}

function rowFromNode(node, depth, expanded) {
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
    durationLabel: node.action.duration ?? null,
    live: node.live,
    startOffsetMs: node.startOffsetMs,
    durMs: node.durMs,
    startClock: shortTime(node.action.start_time) || '',
    startOffsetLabel: `+${formatOffset(node.startOffsetMs)}`,
    durationText: node.live
      ? 'running…'
      : node.action.duration ?? formatOffset(node.durMs ?? 0),
    metrics: tooltipMetrics(node.action),
    action: node.action,
  };
}

function tooltipMetrics(action) {
  return WATERFALL_METRICS.map((metric) => ({ label: metric.label, value: metric.get(action) }))
    .filter((item) => item.value !== undefined && item.value !== null && item.value !== '');
}

function actionNode(action, window) {
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
    target: semanticActionTarget(action) || '',
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

function computeWindow(actions) {
  let startNanos = null;
  let endNanos = null;
  let startIso = null;
  let endIso = null;
  for (const action of actions) {
    const start = toBigInt(action.start_time_unix_nanos);
    const end = action.end_time_unix_nanos ? toBigInt(action.end_time_unix_nanos) : start;
    if (startNanos === null || start < startNanos) {
      startNanos = start;
      startIso = action.start_time;
    }
    if (endNanos === null || end > endNanos) {
      endNanos = end;
      endIso = action.end_time ?? action.start_time;
    }
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
  return [node.label, node.target, node.kind, node.status, node.action.duration]
    .filter(Boolean)
    .join(' ')
    .toLowerCase()
    .includes(query);
}

function compareNodes(left, right) {
  if (left.startOffsetMs !== right.startOffsetMs) {
    return left.startOffsetMs - right.startOffsetMs;
  }
  return String(left.id).localeCompare(String(right.id));
}

function invalidatedAction(action) {
  return action?.attributes?.[ACTION_VALID_ATTR] === ACTION_VALID_FALSE;
}

function toBigInt(value) {
  if (value === undefined || value === null || value === '') {
    return 0n;
  }
  try {
    return BigInt(value);
  } catch {
    return 0n;
  }
}

function nanosDiffMs(later, earlier) {
  return Number(later - earlier) / 1_000_000;
}

function microsLabel(value) {
  if (value === undefined || value === null || value === '') {
    return null;
  }
  const micros = Number(value);
  if (!Number.isFinite(micros)) {
    return null;
  }
  if (micros < 1000) {
    return `${Math.round(micros)}µs`;
  }
  return `${(micros / 1000).toFixed(1)}ms`;
}

export function formatOffset(ms) {
  if (!Number.isFinite(ms)) {
    return '';
  }
  if (ms < 1) {
    return '0ms';
  }
  if (ms < 1000) {
    return `${Math.round(ms)}ms`;
  }
  if (ms < 60_000) {
    return `${(ms / 1000).toFixed(2)}s`;
  }
  const minutes = Math.floor(ms / 60_000);
  const seconds = ((ms % 60_000) / 1000).toFixed(1);
  return `${minutes}m${seconds}s`;
}

export function windowLabel(window) {
  if (!window.startIso) {
    return '';
  }
  return `${shortTime(window.startIso)} → ${shortTime(window.endIso)} · ${formatOffset(window.spanMs)}`;
}
