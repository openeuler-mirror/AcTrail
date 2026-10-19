// Format waterfall values and calculate stable time coordinates and ordering.
import { shortTime } from '../../action-tree/common.js';

export function toBigInt(value) {
  if (value === undefined || value === null || value === '') {
    return 0n;
  }
  try {
    return BigInt(value);
  } catch {
    return 0n;
  }
}

export function nanosDiffMs(later, earlier) {
  return Number(later - earlier) / 1_000_000;
}

export function microsLabel(value) {
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

export function computeWindow(actions, intervals = [], axisEndNanos = null) {
  let startNanos = null;
  let endNanos = null;
  let startIso = null;
  let endIso = null;
  const consider = (start, end, isoStart, isoEnd) => {
    if (startNanos === null || start < startNanos) {
      startNanos = start;
      startIso = isoStart;
    }
    if (endNanos === null || end > endNanos) {
      endNanos = end;
      endIso = isoEnd;
    }
  };
  for (const action of actions) {
    const start = toBigInt(action.start_time_unix_nanos);
    const end = action.end_time_unix_nanos ? toBigInt(action.end_time_unix_nanos) : start;
    consider(start, end, action.start_time, action.end_time ?? action.start_time);
  }
  for (const interval of intervals) {
    consider(
      interval.startNanos,
      interval.endNanos ?? interval.startNanos,
      null,
      null,
    );
  }
  if (axisEndNanos) {
    const axisEnd = toBigInt(axisEndNanos);
    if (endNanos === null || axisEnd > endNanos) {
      endNanos = axisEnd;
    }
  }
  if (startNanos === null) {
    return emptyWindow();
  }
  const spanMs = Math.max(nanosDiffMs(endNanos, startNanos), 1);
  return { startNanos, endNanos, spanMs, startIso, endIso };
}

export function emptyWindow() {
  return { startNanos: 0n, endNanos: 0n, spanMs: 1, startIso: null, endIso: null };
}

export function compareNodes(left, right) {
  if (left.startOffsetMs !== right.startOffsetMs) {
    return left.startOffsetMs - right.startOffsetMs;
  }
  return String(left.id).localeCompare(String(right.id));
}
