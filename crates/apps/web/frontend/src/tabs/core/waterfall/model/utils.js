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
