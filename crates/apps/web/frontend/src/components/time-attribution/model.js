export const ATTRIBUTION_COLORS = Object.freeze({
  agent_side: 'var(--stats-chart-cache-hit, #48b89f)',
  model_side: 'var(--stats-chart-output, #7b8cff)',
  unattributed: 'var(--stats-chart-reasoning, #9aa0aa)',
});

export function formatAttributionDuration(value) {
  let nanos;
  try {
    nanos = BigInt(value ?? 0);
  } catch {
    return '—';
  }
  const sign = nanos < 0n ? '-' : '';
  const absolute = nanos < 0n ? -nanos : nanos;
  if (absolute < 1_000n) {
    return `${sign}${absolute} ns`;
  }
  if (absolute < 1_000_000n) {
    return `${sign}${decimalUnits(absolute, 1_000n, 1)} μs`;
  }
  if (absolute < 1_000_000_000n) {
    return `${sign}${decimalUnits(absolute, 1_000_000n, 1)} ms`;
  }
  if (absolute < 60_000_000_000n) {
    return `${sign}${decimalUnits(absolute, 1_000_000_000n, 2)} s`;
  }
  if (absolute < 3_600_000_000_000n) {
    return `${sign}${decimalUnits(absolute, 60_000_000_000n, 1)} min`;
  }
  return `${sign}${decimalUnits(absolute, 3_600_000_000_000n, 1)} h`;
}

export function formatAttributionPercent(value) {
  const bps = Number(value ?? 0);
  return Number.isFinite(bps) ? `${(bps / 100).toFixed(2)}%` : '—';
}

export function targetFromInterval(row, context = {}) {
  if (!row?.start_unix_nanos || !row?.end_unix_nanos) {
    return null;
  }
  return {
    startNanos: row.start_unix_nanos,
    endNanos: row.end_unix_nanos,
    actionIds: Array.isArray(row.action_ids) ? row.action_ids : [],
    ...attributionFocusContext(context),
  };
}

export function normalizeAttributionTarget(target, context = {}) {
  if (!target?.start_unix_nanos || !target?.end_unix_nanos) {
    return null;
  }
  return {
    startNanos: target.start_unix_nanos,
    endNanos: target.end_unix_nanos,
    actionIds: Array.isArray(target.action_ids) ? target.action_ids : [],
    ...attributionFocusContext(context),
  };
}

function attributionFocusContext(context) {
  return Object.fromEntries(
    Object.entries({
      source: context.source,
      dimension: context.dimension,
      key: context.key,
      label: context.label,
      description: context.description,
    }).filter(([, value]) => value !== undefined && value !== null && value !== ''),
  );
}

export function attributionStatusLabel(status) {
  switch (status) {
    case 'complete':
      return 'Complete';
    case 'provisional':
      return 'Provisional';
    case 'partial':
      return 'Partial';
    case 'invalid':
      return 'Invalid';
    default:
      return status || 'Unknown';
  }
}

function decimalUnits(value, unit, decimals) {
  const scale = 10n ** BigInt(decimals);
  const rounded = (value * scale + unit / 2n) / unit;
  const whole = rounded / scale;
  const fraction = rounded % scale;
  if (fraction === 0n) {
    return whole.toString();
  }
  return `${whole}.${fraction.toString().padStart(decimals, '0').replace(/0+$/, '')}`;
}
