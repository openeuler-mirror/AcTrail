const LANE_GAP = 92;
const ROW_GAP = 78;
const TOP = 48;
const LEFT = 52;
const LABEL_GAP = 28;

const TRAJECTORY_COLORS = [
  '#2563eb',
  '#16a34a',
  '#9333ea',
  '#ea580c',
  '#0891b2',
  '#db2777',
  '#4f46e5',
  '#a16207',
];

export function buildTrajectoryLayout(graph) {
  const nodes = [...(graph?.nodes ?? [])].sort(compareNodes);
  const trajectories = buildTrajectories(nodes, graph?.edges ?? []);
  const trajectoryById = new Map(
    trajectories.map((trajectory) => [trajectory.id, trajectory]),
  );
  const previousByTrajectory = new Map();
  const stepByTrajectory = new Map();

  const positionedNodes = nodes.map((node, index) => {
    const trajectory = trajectoryById.get(node.trajectory_id);
    const fallbackStep = stepByTrajectory.get(node.trajectory_id) ?? 0;
    const previous = previousByTrajectory.get(node.trajectory_id) ?? null;
    stepByTrajectory.set(node.trajectory_id, fallbackStep + 1);
    previousByTrajectory.set(node.trajectory_id, node);
    return {
      ...node,
      lane: trajectory?.lane ?? 0,
      trajectory_label: trajectory?.label ?? 'T?',
      x: LEFT + (trajectory?.lane ?? 0) * LANE_GAP,
      y: TOP + index * ROW_GAP,
      color: trajectory?.color ?? trajectoryColor(node.trajectory_id),
      tool_result_delta: countDifference(
        node.tool_result_count,
        previous?.tool_result_count,
      ),
      label: nodeLabel(
        node,
        trajectory?.label ?? 'T?',
        previous,
        fallbackStep,
      ),
    };
  });
  const byId = new Map(positionedNodes.map((node) => [node.id, node]));
  const edges = (graph?.edges ?? [])
    .map((edge) => {
      const source = byId.get(edge.source);
      const target = byId.get(edge.target);
      if (!source || !target) {
        return null;
      }
      return {
        ...edge,
        color: target.color,
        path: edgePath(source, target, edge.kind),
      };
    })
    .filter(Boolean);

  const laneCount = Math.max(
    trajectories.reduce(
      (count, trajectory) => Math.max(count, trajectory.lane + 1),
      0,
    ),
    1,
  );
  return {
    nodes: positionedNodes,
    edges,
    trajectories,
    laneCount,
    width: Math.max(960, LEFT + laneCount * LANE_GAP + 720),
    height: Math.max(240, TOP * 2 + Math.max(nodes.length - 1, 0) * ROW_GAP),
    labelX: LEFT + laneCount * LANE_GAP + LABEL_GAP,
  };
}

export function trajectoryColor(trajectoryId, ordinal = null) {
  const colorIndex = Number.isInteger(ordinal)
    ? ordinal
    : stableHash(trajectoryId) % TRAJECTORY_COLORS.length;
  if (colorIndex < TRAJECTORY_COLORS.length) {
    return TRAJECTORY_COLORS[colorIndex];
  }
  const hue = Math.round((colorIndex * 137.508) % 360);
  return `hsl(${hue} 68% 42%)`;
}

export function nodeLabel(
  node,
  trajectoryLabel = 'T?',
  previous = null,
  fallbackStep = 0,
) {
  const position = Number.isInteger(node.trajectory_position)
    ? node.trajectory_position
    : fallbackStep;
  const model = node.model || node.classifier_id || 'unknown model';
  return {
    title: `${trajectoryLabel} · Step ${position + 1} · ${model}`,
    metadata: [
      countLabel(node.block_count, 'block', previous?.block_count),
      countLabel(
        node.user_message_count,
        'user message',
        previous?.user_message_count,
      ),
      countLabel(
        node.tool_result_count,
        'tool result',
        previous?.tool_result_count,
      ),
    ].join(' · '),
  };
}

function buildTrajectories(nodes, edges) {
  const byId = new Map();
  const trajectoryByNode = new Map();
  for (const [index, node] of nodes.entries()) {
    trajectoryByNode.set(node.id, node.trajectory_id);
    const existing = byId.get(node.trajectory_id);
    if (existing) {
      existing.lastRow = index;
      continue;
    }
    byId.set(node.trajectory_id, {
      id: node.trajectory_id,
      firstRow: index,
      lastRow: index,
      model: node.model || node.classifier_id || 'unknown model',
    });
  }

  const linkedTrajectories = new Map();
  for (const edge of edges) {
    const source = trajectoryByNode.get(edge.source);
    const target = trajectoryByNode.get(edge.target);
    if (!source || !target || source === target) {
      continue;
    }
    addLink(linkedTrajectories, source, target);
    addLink(linkedTrajectories, target, source);
  }

  const laneEnds = [];
  const assigned = new Map();
  return [...byId.values()].map((trajectory, ordinal) => {
    const forbidden = new Set(
      [...(linkedTrajectories.get(trajectory.id) ?? [])]
        .map((id) => assigned.get(id)?.lane)
        .filter(Number.isInteger),
    );
    let lane = laneEnds.findIndex(
      (lastRow, candidate) =>
        lastRow < trajectory.firstRow && !forbidden.has(candidate),
    );
    const reusedLane = lane >= 0;
    if (lane < 0) {
      lane = laneEnds.length;
    }
    laneEnds[lane] = trajectory.lastRow;
    const result = {
      ...trajectory,
      ordinal,
      label: `T${ordinal + 1}`,
      color: trajectoryColor(trajectory.id, ordinal),
      lane,
      reusedLane,
      x: LEFT + lane * LANE_GAP,
      startY: TOP + trajectory.firstRow * ROW_GAP,
      reuseY: TOP + (trajectory.firstRow - 0.5) * ROW_GAP,
    };
    assigned.set(trajectory.id, result);
    return result;
  });
}

function addLink(links, source, target) {
  if (!links.has(source)) {
    links.set(source, new Set());
  }
  links.get(source).add(target);
}

function countLabel(value, singular, previousValue) {
  const numeric = optionalNumber(value);
  const count = numeric == null ? '—' : numeric;
  const noun = numeric === 1 ? singular : `${singular}s`;
  const delta = countDelta(numeric, optionalNumber(previousValue));
  return `${count} ${noun}${delta}`;
}

function countDelta(value, previous) {
  const delta = countDifference(value, previous);
  if (delta == null || delta === 0) {
    return '';
  }
  return ` (${delta > 0 ? '+' : ''}${delta})`;
}

function countDifference(value, previous) {
  const numeric = optionalNumber(value);
  const previousNumeric = optionalNumber(previous);
  if (numeric == null || previousNumeric == null) {
    return null;
  }
  return numeric - previousNumeric;
}

function optionalNumber(value) {
  if (value == null || value === '') {
    return null;
  }
  const numeric = Number(value);
  return Number.isFinite(numeric) ? numeric : null;
}

function stableHash(value) {
  let hash = 2166136261;
  for (const char of String(value ?? '')) {
    hash ^= char.codePointAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return Math.abs(hash >>> 0);
}

function compareNodes(left, right) {
  const time = compareIntegerStrings(
    left.start_time_unix_nanos,
    right.start_time_unix_nanos,
  );
  return time || String(left.id).localeCompare(String(right.id));
}

function compareIntegerStrings(left, right) {
  const a = String(left ?? '0').replace(/^0+/, '') || '0';
  const b = String(right ?? '0').replace(/^0+/, '') || '0';
  return a.length - b.length || a.localeCompare(b);
}

function edgePath(source, target, kind) {
  if (kind === 'append' && source.x === target.x) {
    return `M ${source.x} ${source.y + 11} L ${target.x} ${target.y - 11}`;
  }
  const middleY = source.y + (target.y - source.y) * 0.52;
  return `M ${source.x} ${source.y + 11} C ${source.x} ${middleY}, ${target.x} ${middleY}, ${target.x} ${target.y - 11}`;
}
