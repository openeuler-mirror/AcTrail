import { TREE_NODE_TYPES, UI_LIMITS } from './config';
import { compactMeta, compactRows, kindClass, shortTime } from './common';

const FILE_ACTIVITY_KIND = 'file.activity';
const FILE_ACTION_KINDS = new Set(['file.read', 'file.write', 'file.modify']);

export function groupAgentRootActions(nodes) {
  const minActions = UI_LIMITS.fileActivityGroupMinActions;
  if (!Number.isInteger(minActions) || minActions < 1) {
    throw new Error('invalid UI_LIMITS.fileActivityGroupMinActions');
  }

  const grouped = [];
  let fileRun = [];
  for (const node of nodes) {
    if (isFileActionNode(node)) {
      fileRun.push(node);
      continue;
    }
    flushFileRun(grouped, fileRun, minActions);
    fileRun = [];
    grouped.push(node);
  }
  flushFileRun(grouped, fileRun, minActions);
  return grouped;
}

function flushFileRun(target, fileRun, minActions) {
  if (!fileRun.length) {
    return;
  }
  if (fileRun.length < minActions) {
    target.push(...fileRun);
    return;
  }
  target.push(fileActivityNode(fileRun));
}

function fileActivityNode(children) {
  const first = children[0];
  const last = children[children.length - 1];
  const counts = actionCounts(children);
  const title = `tool.call:file.activity (${children.length})`;
  const started = nodeStartTime(first);
  const ended = nodeEndTime(last);
  const id = `group:file-activity:${first.id}:${last.id}`;
  return {
    id,
    nodeType: TREE_NODE_TYPES.action,
    kind: FILE_ACTIVITY_KIND,
    semanticLabel: title,
    kindClass: kindClass(FILE_ACTIVITY_KIND),
    title,
    meta: compactMeta([timeRange(started, ended), statusSummary(children)]),
    status: statusSummary(children),
    children,
    hasChildren: true,
    childrenLoaded: true,
    loading: false,
    error: '',
    detail: {
      selectionId: id,
      title,
      kind: FILE_ACTIVITY_KIND,
      rows: compactRows({
        actions: children.length,
        read: counts.get('file.read'),
        write: counts.get('file.write'),
        modify: counts.get('file.modify'),
        started,
        ended,
      }),
      raw: {
        kind: FILE_ACTIVITY_KIND,
        actions: children.map((child) => child.detail?.raw).filter(Boolean),
      },
    },
  };
}

function actionCounts(nodes) {
  const counts = new Map();
  for (const node of nodes) {
    counts.set(node.kind, (counts.get(node.kind) ?? 0) + 1);
  }
  return counts;
}

function isFileActionNode(node) {
  return node.nodeType === TREE_NODE_TYPES.action && FILE_ACTION_KINDS.has(node.kind);
}

function statusSummary(nodes) {
  const statuses = new Set(nodes.map((node) => node.status).filter(Boolean));
  if (!statuses.size) {
    return '';
  }
  if (statuses.size === 1) {
    return statuses.values().next().value;
  }
  return 'mixed';
}

function timeRange(started, ended) {
  if (!started) {
    return '';
  }
  if (!ended || ended === started) {
    return shortTime(started);
  }
  return `${shortTime(started)}-${shortTime(ended)}`;
}

function nodeStartTime(node) {
  return node.detail?.raw?.start_time ?? node.detail?.rows?.started ?? null;
}

function nodeEndTime(node) {
  return node.detail?.raw?.end_time ?? node.detail?.rows?.ended ?? null;
}
