import {
  formatAttributionDuration,
  normalizeAttributionTarget,
} from '../../../components/time-attribution/model';

export function filterBreakdownRows(values, query) {
  return values.filter((row) =>
    matchesAttributionQuery([row.label, row.key, row.kind, ...(row.agent_tools ?? [])], query),
  );
}

export function matchesAttributionQuery(values, query) {
  if (!query) return true;
  return values
    .filter((value) => value !== null && value !== undefined)
    .join(' ')
    .toLowerCase()
    .includes(query);
}

export function commandCountLabel(row) {
  if (row.kind === 'tool_overhead') {
    return `${row.segment_count} intervals · Agent Tool self-time`;
  }
  return `${row.action_count} command processes · ${row.segment_count} intervals`;
}

export function openTraceEvent(row, filter) {
  return {
    traceId: row.trace.id,
    tabId: 'waterfall',
    focus: normalizeAttributionTarget(row.target, {
      source: 'Stats Time Attribution',
      dimension: filter?.dimension,
      key: filter?.key,
      label: filter?.label,
      description: `Longest contiguous interval in this Trace · ${formatAttributionDuration(row.contribution_duration_nanos)} total contribution`,
    }),
  };
}
