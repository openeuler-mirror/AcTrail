import { TREE_NODE_TYPES, UI_LIMITS } from './config';
import { compactMeta, compactRows, kindClass, shortTime } from './common';

const GROUP_KIND = 'action.group';
const SAME_KIND_RULE = 'same-kind';
const NON_GROUPABLE_KINDS = new Set(['llm.call', 'llm.request', 'llm.response']);

export function groupActionNodes(nodes) {
  const minActions = UI_LIMITS.actionGroupMinActions;
  if (!Number.isInteger(minActions) || minActions < 1) {
    throw new Error('invalid UI_LIMITS.actionGroupMinActions');
  }

  const grouped = [];
  let run = [];
  for (const node of nodes) {
    if (!sameKindCandidate(node)) {
      flushRun(grouped, run, minActions);
      run = [];
      grouped.push(node);
      continue;
    }
    if (run.length && run[0].kind !== node.kind) {
      flushRun(grouped, run, minActions);
      run = [];
    }
    run.push(node);
  }
  flushRun(grouped, run, minActions);
  return grouped;
}

export function mergeActionTreeChildren(existing, next) {
  const merged = [];
  const seen = new Set();
  for (const child of existing) {
    merged.push(child);
    for (const action of groupCandidateActions(child)) {
      seen.add(action.id);
    }
  }
  for (const child of next) {
    const childActions = groupCandidateActions(child);
    if (childActions.length && childActions.every((action) => seen.has(action.id))) {
      continue;
    }
    const lastIndex = merged.length - 1;
    const last = merged[lastIndex];
    if (sameGroupKey(last, child)) {
      const combined = foldRun(groupCandidateActions(last).concat(childActions));
      merged.splice(lastIndex, 1, ...combined);
      for (const action of childActions) {
        seen.add(action.id);
      }
      continue;
    }
    merged.push(child);
    for (const action of childActions) {
      seen.add(action.id);
    }
  }
  return merged;
}

function flushRun(target, run, minActions) {
  if (!run.length) {
    return;
  }
  if (run.length < minActions) {
    target.push(...run);
    return;
  }
  target.push(actionGroupNode(run));
}

function foldRun(run) {
  const minActions = UI_LIMITS.actionGroupMinActions;
  if (run.length < minActions) {
    return run;
  }
  return [actionGroupNode(run)];
}

function actionGroupNode(children) {
  const first = children[0];
  const last = children[children.length - 1];
  const childKind = first.kind;
  const started = nodeStartTime(first);
  const ended = nodeEndTime(last);
  const status = statusSummary(children);
  const label = first.semanticLabel || first.title || childKind;
  const title = `${label} (${children.length})`;
  const id = `group:${SAME_KIND_RULE}:${childKind}:${first.id}:${last.id}`;
  return {
    id,
    nodeType: TREE_NODE_TYPES.actionGroup,
    kind: GROUP_KIND,
    semanticLabel: title,
    kindClass: kindClass(GROUP_KIND),
    visualClass: 'action-group',
    groupRule: SAME_KIND_RULE,
    groupKey: childKind,
    title,
    meta: compactMeta([childKind, timeRange(started, ended), status]),
    status,
    children,
    hasChildren: true,
    totalChildren: children.length,
    childrenLoaded: true,
    nextChildOffset: children.length,
    hasMoreChildren: false,
    loading: false,
    loadingMore: false,
    error: '',
    detail: {
      selectionId: id,
      title,
      kind: GROUP_KIND,
      rows: compactRows({
        group_rule: SAME_KIND_RULE,
        child_kind: childKind,
        actions: children.length,
        started,
        ended,
        status,
      }),
      raw: {
        kind: GROUP_KIND,
        group_rule: SAME_KIND_RULE,
        child_kind: childKind,
        start_time: started,
        end_time: ended,
        action_ids: children.map((child) => child.id),
      },
    },
  };
}

function sameKindCandidate(node) {
  return (
    node.nodeType === TREE_NODE_TYPES.action &&
    node.kind !== GROUP_KIND &&
    !NON_GROUPABLE_KINDS.has(node.kind)
  );
}

function sameGroupKey(left, right) {
  const leftKey = groupCandidateKey(left);
  return leftKey && leftKey === groupCandidateKey(right);
}

function groupCandidateKey(node) {
  if (node?.nodeType === TREE_NODE_TYPES.actionGroup && node.groupRule === SAME_KIND_RULE) {
    return node.groupKey;
  }
  if (sameKindCandidate(node)) {
    return node.kind;
  }
  return '';
}

function groupCandidateActions(node) {
  if (node?.nodeType === TREE_NODE_TYPES.actionGroup && node.groupRule === SAME_KIND_RULE) {
    return node.children;
  }
  if (sameKindCandidate(node)) {
    return [node];
  }
  return [];
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
