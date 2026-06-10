import { GRAPH_LANES, TREE_NODE_TYPES, UI_LIMITS } from './config';
import { compactMeta, compactRows, kindClass, shortTime } from './common';
import { groupAgentRootActions } from './rootGroups';
import { semanticActionLabel, semanticActionTarget } from '../../actionLabels';

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

export function buildActionTreeChildNodes({ parentNode, childData, traceDetail }) {
  const actions = (childData?.actions ?? []).filter((action) => !invalidatedAction(action));
  const links = childData?.links ?? [];
  const actionById = new Map(actions.map((action) => [action.id, action]));
  const childState = childStateByActionId(childData?.child_state ?? []);
  const actionChildren = linkedActions(actionById, links)
    .sort(parentNode.nodeType === TREE_NODE_TYPES.agent ? sortAgentLinkedActions : sortLinkedActionByTime)
    .map(({ action }) => actionTreeNode(action, childState))
    .filter(Boolean);
  if (parentNode.nodeType === TREE_NODE_TYPES.agent) {
    return groupAgentRootActions(actionChildren);
  }
  if (parentNode.nodeType !== TREE_NODE_TYPES.action) {
    return actionChildren;
  }
  const evidenceChildren = evidenceNodes(
    parentNode.detail.raw,
    traceDetail,
    coveredEvidenceKeys(actions),
  );
  return actionChildren.concat(evidenceChildren).sort(sortTreeNodeByTime);
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
    hasChildren: Boolean(state?.hasChildren ?? action.evidence?.length),
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
    title: display.label,
    meta: display.meta,
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
        started: action.start_time,
        duration: action.duration,
      }),
      attributes: previewAttributes(action.kind, action.attributes),
      evidence: action.evidence ?? [],
      raw: action,
    },
  };
}

function evidenceNodes(action, traceDetail, excludedEvidence = new Set()) {
  return (action.evidence ?? [])
    .filter((evidence) => !excludedEvidence.has(evidenceKey(evidence)))
    .map((evidence) => evidenceNode(evidence, traceDetail));
}

function evidenceNode(evidence, traceDetail) {
  const event = evidence.kind === 'event' ? findById(traceDetail?.events, evidence.id) : null;
  const payload =
    evidence.kind === 'payload_segment' ? findById(traceDetail?.payloads, evidence.id) : null;
  const title = event
    ? `${event.domain}:${event.operation}`
    : payload
      ? `${payload.display_id}:${payload.direction}`
      : `${evidence.kind}:${evidence.id}`;
  return {
    id: `evidence:${evidence.kind}:${evidence.id}:${evidence.role}`,
    nodeType: TREE_NODE_TYPES.evidence,
    kind: evidence.kind,
    kindClass: kindClass(evidence.kind),
    title,
    meta: evidence.role,
    status: event?.operation ?? payload?.status,
    children: [],
    hasChildren: false,
    childrenLoaded: true,
    loading: false,
    error: '',
    detail: {
      selectionId: `evidence:${evidence.kind}:${evidence.id}:${evidence.role}`,
      title,
      kind: evidence.kind,
      rows: compactRows({
        role: evidence.role,
        id: evidence.id,
        pid: event?.pid ?? payload?.pid,
        time: event?.observed_at ?? payload?.observed_at,
      }),
      attributes: event?.metadata ?? payload,
      raw: { evidence, event, payload },
      payloadId: payload?.id,
    },
  };
}

function linkedActions(actionById, links) {
  const seen = new Set();
  return links
    .map((link) => ({ link, action: actionById.get(link.child) }))
    .filter(({ action }) => action)
    .filter(({ link, action }) => !invalidatedParentIdentityLink(link, action))
    .filter(({ action }) => {
      if (seen.has(action.id)) {
        return false;
      }
      seen.add(action.id);
      return true;
    });
}

function coveredEvidenceKeys(actions) {
  const keys = new Set();
  for (const action of actions) {
    for (const evidence of action.evidence ?? []) {
      keys.add(evidenceKey(evidence));
    }
  }
  return keys;
}

function evidenceKey(evidence) {
  return `${evidence.kind}:${evidence.id}`;
}

function applyLazyState(node, state) {
  node.hasChildren = state.hasChildren;
  node.childrenLoaded = state.childrenLoaded;
  node.loading = false;
  node.error = '';
}

function childStateByActionId(rows) {
  return new Map(
    rows.map((row) => [
      row.id,
      {
        hasChildren: Boolean(row.has_children),
        childCount: row.child_count ?? 0,
      },
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
  const label = semanticActionLabel(action);
  const target = semanticActionTarget(action);
  const time = shortTime(action.start_time);
  const duration = action.duration ? `(${action.duration})` : null;
  return {
    label,
    target,
    meta: compactMeta([target, time, duration, action.status]),
  };
}

function pidLabel(pid) {
  return pid === undefined || pid === null ? null : `pid ${pid}`;
}

function laneTitles(depth) {
  const baseTitles = [GRAPH_LANES.agent, GRAPH_LANES.actions, GRAPH_LANES.details];
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
  if (kind === 'llm.response') {
    return [
      'llm.response.content_text',
      'llm.response.reasoning_text',
      'llm.response.output_text',
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
      'llm.response.delta.content_text',
      'llm.response.delta.reasoning_text',
      'llm.response.delta.tool_calls_json',
      'llm.response.finish_reason',
      'sse.done',
      'sse.data_json_state',
      'sse.data_text',
    ];
  }
  return [];
}

function sortLinkedActionByTime(left, right) {
  return compareActionOrder(left.action, right.action);
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

function sortAgentLinkedActions(left, right) {
  return (
    compareActionTime(left.action, right.action) ||
    compareOptionalDecimalStrings(
      left.link.attributes?.[AGENT_ACTION_SEQUENCE_ATTR],
      right.link.attributes?.[AGENT_ACTION_SEQUENCE_ATTR],
      AGENT_ACTION_SEQUENCE_ATTR,
    ) ||
    compareActionId(left.action, right.action)
  );
}

function sortTreeNodeByTime(left, right) {
  return compareOptionalIsoTime(nodeTime(left), nodeTime(right)) || compareNodeId(left, right);
}

function nodeTime(node) {
  return node.detail?.raw?.start_time ?? node.detail?.rows?.time ?? null;
}

function compareOptionalIsoTime(left, right) {
  if (!left && !right) {
    return 0;
  }
  if (!left) {
    return 1;
  }
  if (!right) {
    return -1;
  }
  return String(left).localeCompare(String(right));
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

function compareNodeId(left, right) {
  return String(left.id ?? '').localeCompare(String(right.id ?? ''));
}

function findById(items, id) {
  return (items ?? []).find((item) => item.id === id);
}
