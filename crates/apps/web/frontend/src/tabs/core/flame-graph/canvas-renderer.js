import { formatOffset } from '../waterfall/model.js';

export const FLAME_BAR_HEIGHT = 24;
export const FLAME_DEPTH_PITCH = 28;
export const FLAME_BAR_TOP = 5;

const MIN_BAR_WIDTH = 3;
const DENSITY_BUCKET_WIDTH = 4;
const activityIndexes = new WeakMap();

export function buildFlameTrackFrame(track, viewport, pixelWidth) {
  const width = Math.max(Number(pixelWidth) || 0, 1);
  const startMs = Number(viewport?.startMs) || 0;
  const spanMs = Math.max(Number(viewport?.spanMs) || 0, 0.001);
  const entries = [];
  const densityBuckets = new Map();

  const viewportEnd = startMs + spanMs;
  for (const lane of activityLanes(track?.activities)) {
    const first = lowerBound(lane.ends, startMs);
    const last = upperBound(lane.starts, viewportEnd);
    for (let position = first; position < last; position += 1) {
      const activity = lane.activities[position];
      const activityStart = Number(activity.startOffsetMs) || 0;
      const activityEnd = flameActivityEndMs(activity);
      const left = ((activityStart - startMs) / spanMs) * width;
      const right = ((activityEnd - startMs) / spanMs) * width;
      if (right < 0 || left > width) {
        continue;
      }

      const projectedWidth = Math.max(right - left, width / spanMs * 0.001);
      const instant = projectedWidth < MIN_BAR_WIDTH;
      const entry = {
        activity,
        left,
        width: instant ? MIN_BAR_WIDTH : projectedWidth,
        top: FLAME_BAR_TOP + (Number(activity.depth) || 0) * FLAME_DEPTH_PITCH,
        height: FLAME_BAR_HEIGHT,
        instant,
        showLabel: projectedWidth >= 52
          || (Boolean(activity.backgroundKind) && projectedWidth >= 34),
      };

      if (track.synthetic || !instant) {
        entries.push(entry);
        continue;
      }

      const bucketIndex = Math.floor(Math.max(left, 0) / DENSITY_BUCKET_WIDTH);
      const statusKey = activity.status === 'error' ? 'error' : 'normal';
      const key = `${activity.depth ?? 0}:${bucketIndex}:${statusKey}`;
      const existing = densityBuckets.get(key);
      if (!existing) {
        densityBuckets.set(key, entry);
        entries.push(entry);
        continue;
      }

      const previous = existing.activity;
      const densityCount = activityVisualCount(previous) + activityVisualCount(activity);
      const densityStart = Math.min(previous.startOffsetMs, activity.startOffsetMs);
      const densityEnd = Math.max(flameActivityEndMs(previous), flameActivityEndMs(activity));
      existing.activity = {
        ...previous,
        id: `density:${track.id}:${key}`,
        kind: 'activity.density',
        label: `Dense activity ×${densityCount}`,
        startOffsetMs: densityStart,
        durMs: Math.max(densityEnd - densityStart, 0.001),
        density: true,
        densityCount,
        aggregate: true,
        aggregateCount: densityCount,
        synthetic: false,
      };
      existing.width = DENSITY_BUCKET_WIDTH;
      existing.showLabel = false;
    }
  }

  return entries;
}

function activityLanes(activities) {
  if (!activities?.length) {
    return [];
  }
  const cached = activityIndexes.get(activities);
  if (cached) {
    return cached;
  }
  const sorted = activities.every((activity, position) => (
    position === 0
      || activities[position - 1].startOffsetMs <= activity.startOffsetMs
  ))
    ? activities
    : [...activities].sort((left, right) => left.startOffsetMs - right.startOffsetMs);
  const activitiesByDepth = new Map();
  for (const activity of sorted) {
    const depth = Number(activity.depth) || 0;
    if (!activitiesByDepth.has(depth)) {
      activitiesByDepth.set(depth, []);
    }
    activitiesByDepth.get(depth).push(activity);
  }
  const lanes = [...activitiesByDepth]
    .sort(([left], [right]) => left - right)
    .map(([, laneActivities]) => ({
      activities: laneActivities,
      starts: Float64Array.from(
        laneActivities,
        (activity) => Number(activity.startOffsetMs) || 0,
      ),
      ends: Float64Array.from(laneActivities, flameActivityEndMs),
    }));
  activityIndexes.set(activities, lanes);
  return lanes;
}

function lowerBound(values, target) {
  let low = 0;
  let high = values.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (values[middle] < target) {
      low = middle + 1;
    } else {
      high = middle;
    }
  }
  return low;
}

function upperBound(values, target) {
  let low = 0;
  let high = values.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (values[middle] <= target) {
      low = middle + 1;
    } else {
      high = middle;
    }
  }
  return low;
}

export function hitTestFlameTrack(entries, x, y) {
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    const entry = entries[index];
    if (
      x >= entry.left
      && x <= entry.left + entry.width
      && y >= entry.top
      && y <= entry.top + entry.height
    ) {
      return entry.activity;
    }
  }
  return null;
}

export function flameActivityTitle(activity) {
  if (activity.density) {
    return [
      activity.label,
      `start +${formatOffset(activity.startOffsetMs)}`,
      `${activity.densityCount} activities share this viewport pixel`,
      'Zoom in to inspect them individually',
    ].join('\n');
  }
  return [
    activity.label,
    activity.target,
    activity.kind === 'llm.tool_call' || activity.kind === 'llm.tool_result' ? activity.statusLabel : '',
    `start +${formatOffset(activity.startOffsetMs)}`,
    activity.live ? 'running' : `duration ${formatOffset(activity.durMs ?? 0)}`,
    activity.backgroundKind ? `background ${activity.backgroundKind}` : '',
    activity.summaryMarker ? 'summary marker; raw interval is not continuous activity' : '',
    activity.aggregate
      ? `aggregate of ${activity.aggregateCount} activities; interval is first-to-last, not continuous`
      : '',
  ].filter(Boolean).join('\n');
}

export function flameActivityPaletteKey(activity) {
  if (activity.kind === 'llm.call') {
    return 'llmCall';
  }
  if (activity.kind === 'llm.request') {
    return 'llmRequest';
  }
  if (activity.kind === 'llm.response') {
    return 'llmResponse';
  }
  if (activity.kind === 'llm.tool_call') {
    return 'toolCall';
  }
  if (activity.kind === 'llm.tool_result') {
    return 'toolResult';
  }
  if (activity.group === 'process') {
    return 'process';
  }
  if (activity.group === 'protocol') {
    return 'protocol';
  }
  if (activity.group === 'tools') {
    return 'tool';
  }
  if (activity.group === 'filesystem' || activity.group === 'detail') {
    return 'file';
  }
  return activity.layer === 'harness' ? 'harness' : 'agent';
}

function flameActivityEndMs(activity) {
  return activity.live
    ? Number(activity.traceSpanMs) || Number(activity.startOffsetMs) || 0
    : (Number(activity.startOffsetMs) || 0) + Math.max(Number(activity.durMs) || 0, 0.001);
}

function activityVisualCount(activity) {
  return activity.densityCount ?? activity.aggregateCount ?? 1;
}
