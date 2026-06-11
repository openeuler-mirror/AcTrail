import { compactRows, formatTime, row, valuesMatchQuery } from '../../tableModel';
import { semanticActionLabel, semanticActionTarget } from '../../actionLabels';

export const COMMAND_COLUMNS = Object.freeze([
  { key: 'title', label: 'Command', tree: true },
  { key: 'time', label: 'Time', align: 'numeric' },
  { key: 'duration', label: 'Duration', align: 'numeric' },
  { key: 'pid', label: 'PID', align: 'numeric' },
  { key: 'kind', label: 'Kind', badge: 'kind' },
  { key: 'status', label: 'Status', badge: 'status' },
]);

export function buildCommandTree(actions = [], links = []) {
  const nodes = new Map(actions.map((action) => [action.id, { action, parent: null, children: [] }]));
  const assigned = new Set();
  for (const link of links) {
    const parent = nodes.get(link.parent);
    const child = nodes.get(link.child);
    if (!parent || !child || parent === child || assigned.has(child.action.id)) {
      continue;
    }
    if (wouldCycle(parent, child)) {
      continue;
    }
    child.parent = parent;
    parent.children.push(child);
    assigned.add(child.action.id);
  }
  const roots = actions.filter((action) => !assigned.has(action.id)).map((action) => nodes.get(action.id));
  sortNodes(roots);
  return roots;
}

export function collectParentIds(roots) {
  const ids = [];
  walkNodes(roots, (node) => {
    if (node.children.length) {
      ids.push(node.action.id);
    }
  });
  return ids;
}

export function flattenVisibleCommands(roots, expandedIds) {
  const out = [];
  const visited = new Set();
  const walk = (nodes, depth) => {
    for (const node of nodes) {
      if (visited.has(node.action.id)) {
        continue;
      }
      visited.add(node.action.id);
      const hasChildren = node.children.length > 0;
      const expanded = hasChildren && expandedIds.has(node.action.id);
      out.push(commandRow(node, depth, hasChildren, expanded));
      if (hasChildren && expanded) {
        walk(node.children, depth + 1);
      }
    }
  };
  walk(roots, 0);
  return out;
}

export function flattenMatchingCommands(roots, query) {
  const out = [];
  const visited = new Set();
  const walk = (nodes, depth) => {
    for (const node of nodes) {
      if (visited.has(node.action.id)) {
        continue;
      }
      visited.add(node.action.id);
      if (commandMatchesQuery(node.action, query)) {
        out.push(commandRow(node, depth, node.children.length > 0, node.children.length > 0));
      }
      walk(node.children, depth + 1);
    }
  };
  walk(roots, 0);
  return out;
}

function commandRow(node, depth, hasChildren, expanded) {
  const { action } = node;
  const label = semanticActionLabel(action);
  const target = semanticActionTarget(action);
  const title = target || action.title || label;
  return row(
    action.id,
    {
      title: { text: title, indent: depth, hasChildren, expanded },
      time: formatTime(action.start_time),
      duration: action.duration,
      pid: action.process?.pid,
      kind: label,
      status: action.status,
    },
    {
      title,
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
        duration: action.duration,
      }),
      attributes: action.attributes,
      raw: action,
    },
  );
}

function commandMatchesQuery(action, query) {
  return valuesMatchQuery(
    [
      formatTime(action.start_time),
      action.duration,
      action.process?.pid,
      action.kind,
      semanticActionLabel(action),
      semanticActionTarget(action),
      action.status,
      action.title,
      action.completeness,
    ],
    query,
  );
}

function sortNodes(nodes) {
  nodes.sort(compareNodes);
  for (const node of nodes) {
    sortNodes(node.children);
  }
}

function compareNodes(left, right) {
  const leftTime = Number(left.action.start_time);
  const rightTime = Number(right.action.start_time);
  if (Number.isFinite(leftTime) && Number.isFinite(rightTime) && leftTime !== rightTime) {
    return leftTime - rightTime;
  }
  return String(left.action.id).localeCompare(String(right.action.id));
}

function walkNodes(nodes, visit) {
  for (const node of nodes) {
    visit(node);
    walkNodes(node.children, visit);
  }
}

function wouldCycle(parent, child) {
  let current = parent;
  while (current) {
    if (current === child) {
      return true;
    }
    current = current.parent;
  }
  return false;
}
