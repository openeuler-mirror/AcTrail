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

export function commandCountLabel(row, t) {
  if (row.kind === 'tool_overhead') {
    return t('commands.toolOverheadLabel', { intervals: row.segment_count });
  }
  return t('commands.countLabel', { processes: row.action_count, intervals: row.segment_count });
}

export function openTraceEvent(row, filter, t) {
  return {
    traceId: row.trace.id,
    tabId: 'waterfall',
    focus: normalizeAttributionTarget(row.target, {
      source: t('source'),
      dimension: filter?.dimension,
      key: filter?.key,
      label: filter?.label,
      description: t('traces.contribution', {
        duration: formatAttributionDuration(row.contribution_duration_nanos),
      }),
    }),
  };
}
