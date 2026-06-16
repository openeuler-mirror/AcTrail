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
    get: (action, llmMessages) =>
      llmMessages?.model ??
      action.attributes?.['llm.call.model'] ??
      action.attributes?.['llm.response.model'],
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

// Default Waterfall legend selection: keep noisy groups off until the user
// explicitly enables them on large traces.
export const WATERFALL_DEFAULT_ACTIVE_GROUPS = Object.freeze(['command', 'llm']);

export function defaultActiveGroups(groups) {
  const available = new Set((groups ?? []).map((group) => group.group));
  return new Set(WATERFALL_DEFAULT_ACTIVE_GROUPS.filter((group) => available.has(group)));
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

export function actionDetail(action, llmMessages = null) {
  const label = semanticActionLabel(action);
  const target = semanticActionTarget(action);
  const messages = llmMessages ?? llmMessagesFromAction(action);
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
  const messages = ensureLlmMessages(node);
  const llmText = [
    messages?.requestFull,
    messages?.responseFull,
    messages?.requestPreview,
    messages?.responsePreview,
    node.llmContext?.parentLabel,
    node.llmContext?.scopeLabel,
  ];
  return [node.label, node.target, node.kind, node.status, node.action.duration, ...llmText]
    .filter(Boolean)
    .join(' ')
    .toLowerCase()
    .includes(query);
}

function attachLlmCallDetails(node, nodeById, parentByChild, window) {
  const requestAction =
    node.children.find((child) => child.kind === 'llm.request')?.action ??
    actionById(nodeById, node.action.attributes?.['llm.call.request_action_id']);
  const responseAction =
    node.children.find((child) => child.kind === 'llm.response')?.action ??
    actionById(nodeById, node.action.attributes?.['llm.call.response_action_id']);
  node.llmRequestAction = requestAction;
  node.llmResponseAction = responseAction;
  node.llmPhases = buildLlmPhases(requestAction, responseAction, window);
  node.llmContext = resolveLlmCallContext(node, nodeById, parentByChild);
}

function ensureLlmMessages(node) {
  if (Object.prototype.hasOwnProperty.call(node, 'llmMessages')) {
    return node.llmMessages;
  }
  let messages = null;
  if (node.kind === 'llm.call') {
    messages = buildLlmMessages(node.llmRequestAction, node.llmResponseAction);
  } else if (node.kind === 'llm.request') {
    const requestFull = llmRequestMessage(node.action);
    messages = buildLlmMessages(node.action, null, requestFull, '');
  } else if (node.kind === 'llm.response') {
    const responseFull = llmResponseMessage(node.action);
    messages = buildLlmMessages(null, node.action, '', responseFull);
  }
  node.llmMessages = messages;
  return messages;
}

function buildLlmPhases(requestAction, responseAction, window) {
  const request = requestAction ? phaseFromAction(requestAction, window) : null;
  const response = responseAction ? phaseFromAction(responseAction, window) : null;
  if (!request && !response) {
    return null;
  }
  let gap = null;
  if (request && response && request.durMs !== null) {
    const requestEndMs = request.startOffsetMs + request.durMs;
    if (response.startOffsetMs > requestEndMs) {
      gap = {
        startOffsetMs: requestEndMs,
        durMs: response.startOffsetMs - requestEndMs,
      };
    }
  }
  return { request, response, gap };
}

function phaseFromAction(action, window) {
  const startNanos = toBigInt(action.start_time_unix_nanos);
  const endNanos = action.end_time_unix_nanos ? toBigInt(action.end_time_unix_nanos) : null;
  return {
    startOffsetMs: nanosDiffMs(startNanos, window.startNanos),
    durMs: endNanos === null ? null : nanosDiffMs(endNanos, startNanos),
    live: endNanos === null,
  };
}

function resolveLlmCallContext(node, nodeById, parentByChild) {
  const pid = node.action.process?.pid;
  const ancestors = walkAncestorNodes(node.id, nodeById, parentByChild);
  const commandAncestors = ancestors.filter(
    (ancestor) => ancestor.kind === 'command.invocation' || ancestor.kind === 'agent.invocation',
  );
  const parentCommand = commandAncestors[0] ?? null;
  const trigger = parentCommand?.action.attributes?.['agent.invocation.trigger'];
  const invocationKind = parentCommand?.action.attributes?.['invocation.kind'];
  let scope = 'primary';
  if (trigger === 'child_llm_request') {
    scope = 'subagent';
  } else if (commandAncestors.length > 1) {
    scope = 'nested';
  } else if (invocationKind === 'agent' && commandAncestors.length) {
    scope = 'agent';
  }
  const parentLabel = parentCommand ? commandContextLabel(parentCommand.action) : '';
  return {
    scope,
    pid,
    parentLabel,
    scopeLabel: llmScopeLabel(scope, pid),
  };
}

function walkAncestorNodes(nodeId, nodeById, parentByChild) {
  const ancestors = [];
  const seen = new Set();
  let currentId = parentByChild.get(nodeId);
  while (currentId && !seen.has(currentId)) {
    seen.add(currentId);
    const ancestor = nodeById.get(currentId);
    if (!ancestor) {
      break;
    }
    ancestors.push(ancestor);
    currentId = parentByChild.get(currentId);
  }
  return ancestors;
}

function commandContextLabel(action) {
  const attrs = action.attributes ?? {};
  return previewText(
    attrs['agent.child.command_line'] ?? attrs['command.line'] ?? action.title ?? '',
    72,
  );
}

function llmScopeLabel(scope, pid) {
  const pidLabel = pid === undefined || pid === null ? '' : `pid ${pid}`;
  switch (scope) {
    case 'subagent':
      return pidLabel ? `subagent · ${pidLabel}` : 'subagent';
    case 'nested':
      return pidLabel ? `nested agent · ${pidLabel}` : 'nested agent';
    case 'agent':
      return pidLabel ? `agent · ${pidLabel}` : 'agent';
    default:
      return pidLabel || '';
  }
}

export function llmBarSegments(row, axisWindow) {
  const { startMs, spanMs } = axisWindow;
  if (!spanMs) {
    return [];
  }
  if (row.kind === 'llm.call' && row.llmPhases) {
    const segments = [];
    const { request, response, gap } = row.llmPhases;
    if (request) {
      segments.push(phaseSegment('request', request, startMs, spanMs, row.live));
    }
    if (gap?.durMs > 0.05) {
      segments.push(phaseSegment('ttft', gap, startMs, spanMs, false));
    }
    if (response) {
      segments.push(phaseSegment('response', response, startMs, spanMs, row.live && !request));
    }
    return segments.filter(Boolean);
  }
  if (row.kind === 'llm.request') {
    return [phaseSegment('request', { startOffsetMs: row.startOffsetMs, durMs: row.durMs, live: row.live }, startMs, spanMs, row.live)].filter(Boolean);
  }
  if (row.kind === 'llm.response') {
    return [phaseSegment('response', { startOffsetMs: row.startOffsetMs, durMs: row.durMs, live: row.live }, startMs, spanMs, row.live)].filter(Boolean);
  }
  return [];
}

function barInstantRow(row, axisWindow) {
  if (row.live || row.durMs === null) {
    return false;
  }
  const { spanMs } = axisWindow;
  if (!spanMs) {
    return false;
  }
  return (row.durMs / spanMs) * 100 < 1.5;
}

function barStyleForRow(row, axisWindow) {
  const { startMs, spanMs } = axisWindow;
  const left = clampPct(((row.startOffsetMs - startMs) / spanMs) * 100);
  if (barInstantRow(row, axisWindow)) {
    return { left: `${left}%`, width: '3px' };
  }
  const endMs = row.live ? startMs + spanMs : row.startOffsetMs + (row.durMs ?? 0);
  const width = Math.max(((endMs - row.startOffsetMs) / spanMs) * 100, 0.5);
  return { left: `${left}%`, width: `${Math.min(width, 100 - left)}%` };
}

function barClassForRow(row) {
  if (row.kind === 'llm.request') {
    return 'wf-bar-request';
  }
  if (row.kind === 'llm.response') {
    return 'wf-bar-response';
  }
  return `wf-group-${row.kindGroup}`;
}

function barTitleForRow(row) {
  const lines = [row.label];
  if (row.target) {
    lines.push(row.target);
  }
  if (row.llmRequestPreview) {
    lines.push(`request: ${row.llmMessages?.requestFull ?? row.llmRequestPreview}`);
  }
  if (row.llmResponsePreview) {
    lines.push(`response: ${row.llmMessages?.responseFull ?? row.llmResponsePreview}`);
  }
  if (row.llmScope) {
    lines.push(`scope: ${row.llmScope}`);
  }
  if (row.agentContext) {
    lines.push(`parent: ${row.agentContext}`);
  }
  if (row.llmPhases?.gap?.durMs) {
    lines.push(`ttft: ${formatOffset(row.llmPhases.gap.durMs)}`);
  }
  lines.push(`start +${formatOffset(row.startOffsetMs)}`);
  for (const metric of row.metrics) {
    lines.push(`${metric.label}: ${metric.value}`);
  }
  lines.push(`status: ${row.status}`);
  return lines.join('\n');
}

export function decorateWaterfallRows(rows, axisWindow) {
  return rows.map((row) => {
    const segments = llmBarSegments(row, axisWindow).map((segment) => ({
      ...segment,
      instant: segment.kind !== 'ttft' && isInstantBarSegment(segment),
    }));
    return {
      ...row,
      barSegments: segments,
      barStyle: barStyleForRow(row, axisWindow),
      barClass: barClassForRow(row),
      barInstant: barInstantRow(row, axisWindow),
      barTitle: barTitleForRow(row),
    };
  });
}

function isInstantBarSegment(segment) {
  const width = Number.parseFloat(String(segment.style.width));
  return Number.isFinite(width) && width < 1.5;
}

export function emptyWaterfallModel() {
  return { roots: [], window: emptyWindow(), groups: [], totalActions: 0 };
}

function phaseSegment(kind, phase, startMs, spanMs, live) {
  if (!phase) {
    return null;
  }
  const left = clampPct(((phase.startOffsetMs - startMs) / spanMs) * 100);
  const endMs = live && kind !== 'ttft' ? startMs + spanMs : phase.startOffsetMs + (phase.durMs ?? 0);
  const widthPct = kind === 'ttft'
    ? Math.max(((phase.durMs ?? 0) / spanMs) * 100, 0.35)
    : Math.max(((endMs - phase.startOffsetMs) / spanMs) * 100, 0.5);
  if (widthPct <= 0) {
    return null;
  }
  return {
    kind,
    style: {
      left: `${left}%`,
      width: `${Math.min(widthPct, 100 - left)}%`,
    },
  };
}

function clampPct(value) {
  return Math.min(Math.max(value, 0), 100);
}

function llmMessagesFromAction(action) {
  if (action?.kind !== 'llm.call') {
    if (action?.kind === 'llm.request') {
      const requestFull = llmRequestMessage(action);
      return buildLlmMessages(action, null, requestFull, '');
    }
    if (action?.kind === 'llm.response') {
      const responseFull = llmResponseMessage(action);
      return buildLlmMessages(null, action, '', responseFull);
    }
    return null;
  }
  return null;
}

function actionById(nodeById, actionId) {
  if (!actionId) {
    return null;
  }
  return nodeById.get(actionId)?.action ?? null;
}

function buildLlmMessages(requestAction, responseAction, requestOverride = null, responseOverride = null) {
  const requestFull = requestOverride ?? llmRequestMessage(requestAction, { preview: false });
  const responseFull = responseOverride ?? llmResponseMessage(responseAction);
  if (!requestFull && !responseFull) {
    return null;
  }
  const requestPreview =
    requestOverride !== null
      ? previewText(requestOverride, 160)
      : previewText(llmRequestMessage(requestAction, { preview: true }) || requestFull, 160);
  const model =
    requestAction?.attributes?.['llm.request.model'] ??
    responseAction?.attributes?.['llm.response.model'] ??
    null;
  return {
    model,
    requestFull,
    responseFull,
    requestPreview,
    responsePreview: previewText(responseFull, 160),
  };
}

function llmRequestMessage(action, { preview = false } = {}) {
  if (!action) {
    return '';
  }
  const attrs = action.attributes ?? {};
  const raw = attrs['llm.request.body_json'] || attrs['llm.request.body_text'] || '';
  return extractLlmRequestMessage(raw, preview);
}

function llmResponseMessage(action) {
  if (!action) {
    return '';
  }
  const attrs = action.attributes ?? {};
  const parts = [
    attrs['llm.response.reasoning_text'],
    attrs['llm.response.content_text'],
  ].filter((value) => String(value ?? '').trim().length > 0);
  if (parts.length > 0) {
    return Array.from(new Set(parts)).join('\n\n');
  }
  return '';
}

function extractLlmRequestMessage(raw, preview) {
  const text = String(raw ?? '').trim();
  if (!text) {
    return '';
  }
  if (text.startsWith('{') || text.startsWith('[')) {
    try {
      const parsed = JSON.parse(text);
      if (preview) {
        const userText = messagesTextByRoles(parsed?.messages, USER_MESSAGE_ROLES);
        if (userText) {
          return userText;
        }
        if (typeof parsed?.input === 'string') {
          return parsed.input.trim();
        }
        if (typeof parsed?.prompt === 'string') {
          return parsed.prompt.trim();
        }
      }
      const fromMessages = messagesText(parsed?.messages ?? parsed?.input);
      if (fromMessages) {
        return fromMessages;
      }
      if (typeof parsed?.prompt === 'string') {
        return parsed.prompt.trim();
      }
      if (typeof parsed?.input === 'string') {
        return parsed.input.trim();
      }
    } catch {
      // Fall through to raw text when JSON is truncated or invalid.
    }
  }
  return text;
}

function extractLlmAssistantMessage(raw) {
  const text = String(raw ?? '').trim();
  if (!text) {
    return '';
  }
  if (text.startsWith('{') || text.startsWith('[')) {
    try {
      const parsed = JSON.parse(text);
      const fromChoices = openAiChoicesText(parsed);
      if (fromChoices) {
        return fromChoices;
      }
      const fromOutput = openAiResponsesOutputText(parsed);
      if (fromOutput) {
        return fromOutput;
      }
      const fromAnthropic = anthropicResponseText(parsed);
      if (fromAnthropic) {
        return fromAnthropic;
      }
      const fromMessages = messagesTextByRoles(parsed?.messages, ASSISTANT_MESSAGE_ROLES);
      if (fromMessages) {
        return fromMessages;
      }
    } catch {
      // Fall through to raw text when JSON is truncated or invalid.
    }
  }
  return text;
}

const USER_MESSAGE_ROLES = ['user', 'human'];
const ASSISTANT_MESSAGE_ROLES = ['assistant'];

function messagesTextByRoles(messages, roles) {
  if (!Array.isArray(messages)) {
    return '';
  }
  const allowed = new Set(roles);
  return messages
    .map((message) => formatMessageLine(message, allowed))
    .filter(Boolean)
    .join('\n');
}

function formatMessageLine(message, allowedRoles = null) {
  if (!message || typeof message !== 'object') {
    return '';
  }
  const role = String(message.role ?? '').toLowerCase();
  if (allowedRoles && role && !allowedRoles.has(role)) {
    return '';
  }
  const prefix = message.role ? `[${message.role}] ` : '';
  const content = messageContentText(message.content ?? message.text ?? message.input);
  if (!content) {
    return '';
  }
  return `${prefix}${content}`.trim();
}

function messageContentText(content) {
  if (typeof content === 'string') {
    return content.trim();
  }
  if (Array.isArray(content)) {
    return content
      .map((part) => {
        if (typeof part === 'string') {
          return part;
        }
        if (typeof part?.text === 'string') {
          return part.text;
        }
        if (part?.type === 'text' && typeof part?.text === 'string') {
          return part.text;
        }
        return '';
      })
      .filter(Boolean)
      .join(' ')
      .trim();
  }
  return '';
}

function openAiChoicesText(parsed) {
  const choices = parsed?.choices;
  if (!Array.isArray(choices)) {
    return '';
  }
  return choices
    .map((choice) => {
      const message = choice?.message ?? choice?.delta;
      if (!message) {
        return choice?.text ?? '';
      }
      return messageContentText(message.content ?? message.text) || String(message.content ?? message.text ?? '').trim();
    })
    .filter(Boolean)
    .join('\n');
}

function openAiResponsesOutputText(parsed) {
  const output = parsed?.output;
  if (!Array.isArray(output)) {
    return '';
  }
  return output
    .flatMap((item) => {
      if (typeof item?.content === 'string') {
        return [item.content];
      }
      if (Array.isArray(item?.content)) {
        return item.content
          .map((part) => (typeof part?.text === 'string' ? part.text : ''))
          .filter(Boolean);
      }
      return [];
    })
    .join('');
}

function anthropicResponseText(parsed) {
  if (Array.isArray(parsed?.content)) {
    const text = parsed.content
      .map((block) => (typeof block?.text === 'string' ? block.text : ''))
      .filter(Boolean)
      .join('');
    if (text) {
      return text;
    }
  }
  const message = parsed?.message ?? parsed?.delta;
  if (message) {
    return messageContentText(message.content ?? message.text);
  }
  return '';
}

function extractLlmUserMessage(raw) {
  return extractLlmRequestMessage(raw, false);
}

function messagesText(messages) {
  if (Array.isArray(messages)) {
    return messages
      .map((message) => formatMessageLine(message))
      .filter(Boolean)
      .join('\n');
  }
  if (typeof messages === 'string') {
    return messages.trim();
  }
  return '';
}

function previewText(text, maxLen) {
  const normalized = String(text ?? '').replace(/\s+/g, ' ').trim();
  if (!normalized) {
    return '';
  }
  if (normalized.length <= maxLen) {
    return normalized;
  }
  return `${normalized.slice(0, maxLen - 1)}…`;
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
