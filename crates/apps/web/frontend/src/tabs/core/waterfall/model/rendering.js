import { projectTimeInterval } from '../../time-navigation/model.js';
import { kindGroup } from './main.js';
import { formatOffset } from './utils.js';

export function llmBarSegments(row, axisWindow) {
  const { startMs, spanMs } = axisWindow;
  if (!spanMs) {
    return [];
  }
  if (row.kind === 'llm.call' && row.llmPhases) {
    const segments = [];
    const { request, response, gap } = row.llmPhases;
    if (request) {
      segments.push(phaseSegment('request', request, startMs, spanMs, row.live));
    }
    if (gap?.durMs > 0.05) {
      segments.push(phaseSegment('ttft', gap, startMs, spanMs, false));
    }
    if (response) {
      segments.push(phaseSegment('response', response, startMs, spanMs, row.live && !request));
    }
    return segments.filter(Boolean);
  }
  if (row.kind === 'llm.request') {
    return [phaseSegment('request', { startOffsetMs: row.startOffsetMs, durMs: row.durMs, live: row.live }, startMs, spanMs, row.live)].filter(Boolean);
  }
  if (row.kind === 'llm.response') {
    return [phaseSegment('response', { startOffsetMs: row.startOffsetMs, durMs: row.durMs, live: row.live }, startMs, spanMs, row.live)].filter(Boolean);
  }
  return [];
}

function barInstantRow(row, axisWindow) {
  if (row.live || row.durMs === null) {
    return false;
  }
  const { spanMs } = axisWindow;
  if (!spanMs) {
    return false;
  }
  return (row.durMs / spanMs) * 100 < 1.5;
}

function barStyleForRow(row, axisWindow) {
  const { startMs, spanMs } = axisWindow;
  const endMs = row.live ? startMs + spanMs : row.startOffsetMs + (row.durMs ?? 0);
  if (!row.live && endMs === row.startOffsetMs) {
    if (row.startOffsetMs < startMs || row.startOffsetMs > startMs + spanMs) {
      return null;
    }
    return {
      left: `${((row.startOffsetMs - startMs) / spanMs) * 100}%`,
      width: '3px',
    };
  }
  const projected = projectTimeInterval(row.startOffsetMs, endMs, axisWindow);
  if (!projected) {
    return null;
  }
  if (barInstantRow(row, axisWindow)) {
    return { left: `${projected.leftPct}%`, width: '3px' };
  }
  return { left: `${projected.leftPct}%`, width: `${projected.widthPct}%` };
}

function barClassForRow(row) {
  if (row.kind === 'llm.request') {
    return 'wf-bar-request';
  }
  if (row.kind === 'llm.response') {
    return 'wf-bar-response';
  }
  return `wf-group-${row.kindGroup}`;
}

function barTitleForRow(row) {
  const lines = [row.label];
  if (row.target) {
    lines.push(row.target);
  }
  if (row.llmRequestPreview) {
    lines.push(`request: ${row.llmMessages?.requestFull ?? row.llmRequestPreview}`);
  }
  if (row.llmResponsePreview) {
    lines.push(`response: ${row.llmMessages?.responseFull ?? row.llmResponsePreview}`);
  }
  if (row.llmScope) {
    lines.push(`scope: ${row.llmScope}`);
  }
  if (row.agentContext) {
    lines.push(`parent: ${row.agentContext}`);
  }
  if (row.llmPhases?.gap?.durMs) {
    lines.push(`ttft: ${formatOffset(row.llmPhases.gap.durMs)}`);
  }
  lines.push(`start +${formatOffset(row.startOffsetMs)}`);
  for (const metric of row.metrics) {
    lines.push(`${metric.label}: ${metric.value}`);
  }
  lines.push(`status: ${row.statusLabel ?? row.status}`);
  return lines.join('\n');
}

export function decorateWaterfallRows(rows, axisWindow) {
  return rows.map((row) => {
    const segments = llmBarSegments(row, axisWindow).map((segment) => ({
      ...segment,
      instant: segment.kind !== 'ttft' && isInstantBarSegment(segment),
    }));
    return {
      ...row,
      barSegments: segments,
      barStyle: barStyleForRow(row, axisWindow),
      barClass: barClassForRow(row),
      barInstant: barInstantRow(row, axisWindow),
      barTitle: barTitleForRow(row),
    };
  });
}

function isInstantBarSegment(segment) {
  const width = Number.parseFloat(String(segment.style.width));
  return Number.isFinite(width) && width < 1.5;
}

function phaseSegment(kind, phase, startMs, spanMs, live) {
  if (!phase) {
    return null;
  }
  const endMs = live && kind !== 'ttft' ? startMs + spanMs : phase.startOffsetMs + (phase.durMs ?? 0);
  const projected = projectTimeInterval(phase.startOffsetMs, endMs, { startMs, spanMs });
  if (!projected) {
    return null;
  }
  return {
    kind,
    style: {
      left: `${projected.leftPct}%`,
      width: `${projected.widthPct}%`,
    },
  };
}
