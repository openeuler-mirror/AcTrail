import { formatOffset } from '../model';
import { formatAttributionDuration } from '../../../../components/time-attribution/model';

export function rowOverlapsWindow(row, targetWindow) {
  const targetEnd = targetWindow.startMs + targetWindow.spanMs;
  const rowEnd = row.live
    ? Number.POSITIVE_INFINITY
    : row.startOffsetMs + Math.max(row.durMs ?? 0, 0);
  return row.startOffsetMs <= targetEnd && rowEnd >= targetWindow.startMs;
}

export function contextualFocusWindow(targetWindow, globalSpanMs) {
  const minimumPadding = Math.min(Math.max(globalSpanMs * 0.01, 2), 25);
  const padding = Math.max(targetWindow.spanMs * 0.06, minimumPadding);
  const startMs = Math.max(targetWindow.startMs - padding, 0);
  const endMs = Math.min(
    targetWindow.startMs + targetWindow.spanMs + padding,
    globalSpanMs,
  );
  return {
    startMs,
    spanMs: Math.max(endMs - startMs, targetWindow.spanMs, 0.001),
  };
}

export function attributionOccurrences(attribution, dimension, key) {
  if (!attribution || !dimension || !key) {
    return [];
  }
  let segments = [];
  if (dimension === 'command') {
    segments = (attribution.command_segments ?? []).filter(
      (segment) => segment.key === key,
    );
  } else if (dimension === 'model') {
    segments = (attribution.segments ?? []).filter(
      (segment) => segment.category === 'model_side' && segment.key === key,
    );
  } else if (dimension === 'tool') {
    segments = (attribution.segments ?? []).filter(
      (segment) => segment.category === 'agent_side' && segment.key === key,
    );
  } else if (dimension === 'category') {
    segments = (attribution.segments ?? []).filter(
      (segment) => segment.category === key,
    );
  } else if (dimension === 'model_request') {
    segments = (attribution.bottlenecks?.model_requests?.items ?? []).filter(
      (segment) => segment.key === key,
    );
  } else if (dimension === 'command_occurrence') {
    segments = (attribution.bottlenecks?.commands?.items ?? []).filter(
      (segment) => segment.key === key,
    );
  } else if (dimension === 'unattributed_gap') {
    segments = attribution.bottlenecks?.unattributed_gaps?.items ?? [];
  }
  return segments
    .map((segment) => ({
      startNanos: segment.start_unix_nanos,
      endNanos: segment.end_unix_nanos,
      actionIds: Array.isArray(segment.action_ids) ? segment.action_ids : [],
    }))
    .sort((left, right) => compareNanos(left.startNanos, right.startNanos));
}

export function decorateBottleneckItems(items, traceDurationNanos, windowStartNanos) {
  const rows = items ?? [];
  const durations = rows.map((item) => parseNanos(item.duration_nanos));
  const longest = durations.reduce((current, duration) => (
    duration > current ? duration : current
  ), 0n);
  const traceDuration = parseNanos(traceDurationNanos);
  return rows.map((item, index) => {
    const duration = durations[index];
    return {
      ...item,
      offsetLabel: `+${formatOffset(nanosOffsetMs(item.start_unix_nanos, windowStartNanos))}`,
      relativeWidth: ratioPercent(duration, longest, 1),
      traceShare: `${ratioPercent(duration, traceDuration, 2).toFixed(2)}% Trace`,
    };
  });
}

export function bottleneckStatusLabel(status) {
  return {
    in_progress: 'Live',
    partial: 'Partial',
    provisional: 'Provisional',
    error: 'Error',
  }[status] ?? status;
}

export function bottleneckStatusDescription(status) {
  return status && status !== 'complete'
    ? `${bottleneckStatusLabel(status)} interval`
    : '';
}

export function parseNanos(value) {
  try {
    return BigInt(value ?? 0);
  } catch {
    return 0n;
  }
}

export function ratioPercent(value, total, decimals) {
  if (value <= 0n || total <= 0n) {
    return 0;
  }
  const scale = 10n ** BigInt(decimals);
  const scaled = (value * 100n * scale + total / 2n) / total;
  return Number(scaled) / Number(scale);
}

export function projectAttributionLane(rows, laneKind, targetWindow, windowStartNanos, t) {
  const axisEnd = targetWindow.startMs + targetWindow.spanMs;
  return (rows ?? [])
    .map((row, index) => {
      const startMs = nanosOffsetMs(row.start_unix_nanos, windowStartNanos);
      const endMs = nanosOffsetMs(row.end_unix_nanos, windowStartNanos);
      const clippedStart = Math.max(startMs, targetWindow.startMs);
      const clippedEnd = Math.min(endMs, axisEnd);
      if (clippedEnd <= clippedStart) {
        return null;
      }
      const left = ((clippedStart - targetWindow.startMs) / targetWindow.spanMs) * 100;
      const width = ((clippedEnd - clippedStart) / targetWindow.spanMs) * 100;
      const context = laneTargetContext(row, laneKind);
      return {
        id: `${laneKind}-${row.id ?? row.key ?? index}-${index}`,
        label: row.label,
        className: laneKind === 'command'
          ? `lane-command-${row.kind}`
          : `lane-category-${row.category}`,
        style: {
          left: `${left}%`,
          width: `${width}%`,
        },
        showLabel: width >= 7,
        title: `${row.label} · ${formatAttributionDuration(row.duration_nanos)}`,
        target: {
          startNanos: row.start_unix_nanos,
          endNanos: row.end_unix_nanos,
          actionIds: Array.isArray(row.action_ids) ? row.action_ids : [],
          source: t('source.lane'),
          ...context,
        },
      };
    })
    .filter(Boolean);
}

export function laneTargetContext(row, laneKind) {
  if (laneKind === 'command') {
    return {
      dimension: 'command',
      key: row.key,
      label: row.label,
    };
  }
  if (row.category === 'model_side') {
    return {
      dimension: 'model',
      key: row.key,
      label: row.label,
    };
  }
  if (row.category === 'agent_side' && row.subcategory !== 'orchestration') {
    return {
      dimension: 'tool',
      key: row.key,
      label: row.label,
    };
  }
  return {
    dimension: 'category',
    key: row.category,
    label: row.label,
  };
}

export function nanosOffsetMs(value, origin) {
  try {
    return Number(BigInt(value) - BigInt(origin)) / 1_000_000;
  } catch {
    return 0;
  }
}

export function compareNanos(left, right) {
  try {
    const leftNanos = BigInt(left);
    const rightNanos = BigInt(right);
    return leftNanos < rightNanos ? -1 : leftNanos > rightNanos ? 1 : 0;
  } catch {
    return String(left).localeCompare(String(right));
  }
}

export function contextChip(row) {
  const context = String(row.agentContext ?? '').trim();
  if (!context || context === String(row.target ?? '').trim()) {
    return '';
  }
  return context.length > 32 ? `${context.slice(0, 31)}…` : context;
}


export function nanosClock(nanos) {
  const millis = Number(BigInt(nanos) / 1000000n);
  return new Date(millis).toLocaleString();
}

