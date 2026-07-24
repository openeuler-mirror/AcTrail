import { GRAPH_LANES, TREE_NODE_TYPES, UI_LIMITS } from './config';
import { compactMeta, compactRows, kindClass, shortTime } from './common';
import { groupActionNodes, mergeActionTreeChildren } from './actionGroups';
import { semanticActionLabel, semanticActionTarget } from '../../actionLabels';

export { mergeActionTreeChildren };

const NODE_ID_AGENT = 'agent-process';
const AGENT_ACTION_SEQUENCE_ATTR = 'agent.performed_action.sequence';
const ACTION_VALID_ATTR = 'actrail.action.valid';
const ACTION_VALID_FALSE = 'false';
const LINK_VALID_ATTR = 'actrail.link.valid';
const LINK_VALID_FALSE = 'false';
const PROCESS_PARENT_IDENTITY_STATE_ATTR = 'process.parent.identity_state';
const PROCESS_PARENT_IDENTITY_STATE_CONFLICT = 'conflict';
const PARENT_IDENTITY_LINK_ROLES = new Set([
  'agent.performed_action',
  'command.contains_command_invocation',
]);

export function buildActionTreeRootNode({ traceDetail, rootData }) {
  const observedAgent = rootData?.root?.observed_agent ?? null;
  const root = agentNode(traceDetail, observedAgent);
  applyLazyState(root, {
    hasChildren: Boolean(rootData?.root?.has_children),
    childrenLoaded: false,
  });
  return root;
}

export function buildActionTreeChildNodes({ parentNode, childData }) {
  const actions = (childData?.actions ?? []).filter((action) => !invalidatedAction(action));
  const links = childData?.links ?? [];
  const childState = childStateByActionId(childData?.child_state ?? []);
  const actionChildren = displayActions(actions, links)
    .sort(parentNode.nodeType === TREE_NODE_TYPES.agent ? sortAgentDisplayActions : sortDisplayActionByTime)
    .map(({ action }) => actionTreeNode(action, childState))
    .filter(Boolean);
  return groupActionNodes(actionChildren);
}

export function buildVisibleActionTreeModel({ root, query }) {
  const normalizedQuery = query.trim().toLowerCase();
  return {
    lanes: laneTitles(maxDepth(root)),
    root: filterTree(root, normalizedQuery, true) ?? root,
    queryActive: normalizedQuery.length > 0,
  };
}

function actionTreeNode(action, childState) {
  const node = actionNode(action);
  const state = childState.get(action.id);
  applyLazyState(node, {
    hasChildren: Boolean(state?.hasChildren),
    childrenLoaded: false,
  });
  return node;
}

function agentNode(traceDetail, observedAgent) {
  const trace = traceDetail?.trace;
  const attrs = observedAgent?.attributes ?? {};
  const title = observedAgent?.title || trace?.name || 'Agent process';
  const pid = observedAgent?.process?.pid ?? trace?.root_pid;
  return {
    id: NODE_ID_AGENT,
    nodeType: TREE_NODE_TYPES.agent,
    kind: 'agent.process',
    kindClass: 'agent-process',
    title: `AgentProcess:${title}`,
    meta: compactMeta([pidLabel(pid), trace?.state, trace?.health]),
    metaItems: compactMetaItems([
      metaItem('pid', pidLabel(pid)),
      statusMetaItem(trace?.state),
      statusMetaItem(trace?.health),
    ]),
    status: trace?.health ?? trace?.state,
    children: [],
    hasChildren: false,
    childrenLoaded: true,
    loading: false,
    error: '',
    detail: {
      selectionId: NODE_ID_AGENT,
      title: `AgentProcess:${title}`,
      kind: 'agent.process',
      rows: compactRows({
        pid,
        executable: attrs['process.executable'] ?? attrs.executable,
        command_line: attrs.command_line,
        identity_status: attrs['agent.identity.status'],
        profile: trace?.profile,
        state: trace?.state,
        health: trace?.health,
      }),
      raw: { trace, observedAgent },
    },
  };
}

function actionNode(action) {
  const display = actionDisplay(action);
  return {
    id: action.id,
    nodeType: TREE_NODE_TYPES.action,
    kind: action.kind,
    semanticLabel: display.label,
    kindClass: kindClass(action.kind),
    visualClass: actionVisualClass(action),
    title: display.label,
    durationBadge: display.durationBadge,
    meta: display.meta,
    metaItems: display.metaItems,
    status: action.status,
    children: [],
    hasChildren: false,
    childrenLoaded: true,
    loading: false,
    error: '',
    detail: {
      selectionId: action.id,
      title: display.label,
      kind: display.label,
      rows: compactRows({
        semantic_label: display.label,
        raw_action_kind: action.kind,
        target: display.target,
        status: action.status,
        completeness: action.completeness,
        pid: action.process?.pid,
        evidence: action.evidence?.length,
        started: shortTime(action.start_time),
        duration: action.duration,
      }),
      attributes: previewAttributes(action.kind, action.attributes),
      evidence: action.evidence ?? [],
      raw: action,
    },
  };
}

function actionVisualClass(action) {
  if (action.kind === 'command.invocation' && action.attributes?.['invocation.kind'] === 'agent') {
    return 'agent-call';
  }
  return '';
}

function displayActions(actions, links) {
  const linkByChild = displayLinkByChild(actions, links);
  return actions.map((action) => ({
    action,
    link: linkByChild.get(action.id) ?? null,
  }));
}

function displayLinkByChild(actions, links) {
  const actionById = new Map(actions.map((action) => [action.id, action]));
  const seen = new Set();
  const linkByChild = new Map();
  for (const link of links) {
    const action = actionById.get(link.child);
    if (!action || invalidatedParentIdentityLink(link, action)) {
      continue;
    }
    if (!seen.has(action.id)) {
      seen.add(action.id);
      linkByChild.set(action.id, link);
    }
  }
  return linkByChild;
}

function sortDisplayActionByTime(left, right) {
  return compareActionOrder(left.action, right.action);
}

function sortAgentDisplayActions(left, right) {
  return (
    compareActionTime(left.action, right.action) ||
    compareOptionalDecimalStrings(
      left.link?.attributes?.[AGENT_ACTION_SEQUENCE_ATTR],
      right.link?.attributes?.[AGENT_ACTION_SEQUENCE_ATTR],
      AGENT_ACTION_SEQUENCE_ATTR,
    ) ||
    compareActionId(left.action, right.action)
  );
}

function applyLazyState(node, state) {
  node.hasChildren = state.hasChildren;
  node.totalChildren = state.childCount ?? (state.hasChildren ? null : 0);
  node.childrenLoaded = state.childrenLoaded;
  node.nextChildOffset = 0;
  node.hasMoreChildren = false;
  node.loading = false;
  node.loadingMore = false;
  node.error = '';
}

function childStateByActionId(rows) {
  return new Map(
    rows.map((row) => [
      row.id,
      { hasChildren: Boolean(row.has_children), childCount: row.child_count ?? 0 },
    ]),
  );
}

function invalidatedAction(action) {
  return action.attributes?.[ACTION_VALID_ATTR] === ACTION_VALID_FALSE;
}

function invalidatedParentIdentityLink(link, action) {
  return (
    link.attributes?.[LINK_VALID_ATTR] === LINK_VALID_FALSE ||
    (PARENT_IDENTITY_LINK_ROLES.has(link.role) &&
      action.attributes?.[PROCESS_PARENT_IDENTITY_STATE_ATTR] ===
        PROCESS_PARENT_IDENTITY_STATE_CONFLICT)
  );
}

function filterTree(node, query, keepRoot = false) {
  if (!query) {
    return node;
  }
  const children = node.children
    .map((child) => filterTree(child, query, false))
    .filter(Boolean);
  const matches = nodeMatchesQuery(node, query);
  if (!keepRoot && !matches && children.length === 0) {
    return null;
  }
  return {
    ...node,
    queryMatch: matches,
    children,
  };
}

function nodeMatchesQuery(node, query) {
  return [
    node.title,
    node.kind,
    node.meta,
    node.status,
    JSON.stringify(node.detail?.rows ?? {}),
    JSON.stringify(node.detail?.attributes ?? {}),
  ]
    .join(' ')
    .toLowerCase()
    .includes(query);
}

function actionDisplay(action) {
  const label = actionCardLabel(action);
  const target = semanticActionTarget(action);
  const time = shortTime(action.start_time);
  return {
    label,
    target,
    durationBadge: action.duration ?? null,
    meta: compactMeta([target, time, action.status]),
    metaItems: compactMetaItems([
      metaItem(metaKind(action), target),
      metaItem('time', time),
      statusMetaItem(action.status),
    ]),
  };
}

function actionCardLabel(action) {
  if (action?.kind === 'llm.call') {
    return 'LLM Call';
  }
  if (action?.kind === 'llm.request') {
    return 'LLM Request';
  }
  if (action?.kind === 'llm.response') {
    return 'LLM Response';
  }
  return semanticActionLabel(action);
}

function metaKind(action) {
  if (action?.kind === 'llm.call' || action?.kind === 'llm.request' || action?.kind === 'llm.response') {
    return 'model';
  }
  if (action?.kind?.startsWith('file.') || action?.kind === 'fs.enumerate') {
    return 'path';
  }
  if (action?.kind === 'command.invocation' || action?.kind === 'agent.invocation') {
    return 'command';
  }
  return 'target';
}

function compactMetaItems(items) {
  return items.filter((item) => item && item.label);
}

function metaItem(kind, label) {
  const text = String(label ?? '').trim();
  return text ? { kind, label: text } : null;
}

function statusMetaItem(status) {
  const text = String(status ?? '').trim();
  if (!text || successStatus(text)) {
    return null;
  }
  return { kind: 'status', label: text };
}

function successStatus(status) {
  return ['success', 'healthy', 'completed', 'complete', 'ok'].includes(
    String(status ?? '')
      .trim()
      .toLowerCase()
      .replace(/[\s-]+/g, '_'),
  );
}

function pidLabel(pid) {
  return pid === undefined || pid === null ? null : `pid ${pid}`;
}

function laneTitles(depth) {
  const baseTitles = [GRAPH_LANES.agent, GRAPH_LANES.actions];
  return Array.from({ length: depth }, (_, index) => baseTitles[index] ?? `L${index + 1}`);
}

function maxDepth(node) {
  if (!node.children.length) {
    return 1;
  }
  return 1 + Math.max(...node.children.map(maxDepth));
}

function previewAttributes(kind, attributes) {
  if (!attributes) {
    return {};
  }
  return Object.fromEntries(
    prioritizedAttributeEntries(kind, attributes).slice(0, UI_LIMITS.inlineAttributeCount),
  );
}

function prioritizedAttributeEntries(kind, attributes) {
  const entries = Object.entries(attributes);
  const priority = attributePriority(kind);
  if (!priority.length) {
    return entries;
  }
  const used = new Set();
  const prioritized = priority
    .filter((key) => Object.prototype.hasOwnProperty.call(attributes, key))
    .map((key) => {
      used.add(key);
      return [key, attributes[key]];
    });
  return prioritized.concat(entries.filter(([key]) => !used.has(key)));
}

function attributePriority(kind) {
  if (kind === 'llm.call') {
    return [
      'llm.call.model',
      'llm.call.request_action_id',
      'llm.call.response_action_id',
      'payload.stream_key',
      'payload.operation_id',
      'http.request.stream_id',
    ];
  }
  if (kind === 'llm.response') {
    return [
      'llm.response.prompt_tokens',
      'llm.response.completion_tokens',
      'llm.response.total_tokens',
      'llm.response.cached_prompt_tokens',
      'llm.response.reasoning_tokens',
      'llm.response.tool_calls_json',
      'llm.response.model',
      'llm.response.done',
      'llm.response.chunk_count',
      'llm.response.stream',
      'llm.response.body_format',
      'http.response.body_format',
      'http.response.body_json_state',
    ];
  }
  if (kind === 'sse.stream') {
    return ['sse.event_count', 'sse.done', 'llm.response.model', 'payload.stream_key'];
  }
  if (kind === 'sse.event') {
    return [
      'llm.response.finish_reason',
      'sse.done',
      'sse.data_json_state',
    ];
  }
  if (kind === 'command.invocation') {
    return [
      'command.failure.summary',
      'command.exit_code',
      'command.line',
      'process.executable',
      'cwd',
    ];
  }
  if (kind === 'process.exec') {
    return [
      'process.failure.summary',
      'process.exit_code',
      'process.executable',
      'command_line',
      'cwd',
    ];
  }
  return [];
}

function compareActionOrder(left, right) {
  return compareActionTime(left, right) || compareActionId(left, right);
}

function compareActionTime(left, right) {
  return compareDecimalStrings(
    requiredDecimalString(left.start_time_unix_nanos, 'start_time_unix_nanos'),
    requiredDecimalString(right.start_time_unix_nanos, 'start_time_unix_nanos'),
  );
}

function compareOptionalDecimalStrings(left, right, fieldName) {
  const leftText = optionalDecimalString(left, fieldName);
  const rightText = optionalDecimalString(right, fieldName);
  if (leftText === null && rightText === null) {
    return 0;
  }
  if (leftText === null) {
    return 1;
  }
  if (rightText === null) {
    return -1;
  }
  return compareDecimalStrings(leftText, rightText);
}

function requiredDecimalString(value, fieldName) {
  const normalized = optionalDecimalString(value, fieldName);
  if (normalized === null) {
    throw new Error(`missing ${fieldName}`);
  }
  return normalized;
}

function optionalDecimalString(value, fieldName) {
  if (value === undefined || value === null || value === '') {
    return null;
  }
  const text = String(value);
  if (!/^(0|[1-9][0-9]*)$/.test(text)) {
    throw new Error(`invalid ${fieldName}: ${text}`);
  }
  return text;
}

function compareDecimalStrings(left, right) {
  if (left.length !== right.length) {
    return left.length - right.length;
  }
  return left.localeCompare(right);
}

function compareActionId(left, right) {
  return String(left.id ?? '').localeCompare(String(right.id ?? ''));
}
