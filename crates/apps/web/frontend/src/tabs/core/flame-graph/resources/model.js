// Project /resource-timeline responses onto the flame graph's offset-based time axis.
//
// The API speaks Unix milliseconds; the flame graph speaks milliseconds from the
// timeline origin (`timelineWindow.startNanos`). Everything below the fetch uses offsets.

/** Tracks shown in each group, top to bottom. */
export const GROUP_TRACKS = Object.freeze({
  host: Object.freeze(['hostCpu', 'hostMemory', 'hostLoad']),
  cgroup: Object.freeze(['cpu', 'memory', 'io', 'pids']),
});

const MAX_POINTS_LIMIT = 2000;
const MIN_POINTS = 100;

/** Unix milliseconds of the flame graph's offset 0. */
export function timelineOriginMs(timelineWindow) {
  const nanos = timelineWindow?.startNanos;
  if (nanos == null) return null;
  return Number(BigInt(nanos) / 1_000_000n);
}

/**
 * Plans a fetch around the viewport: one viewport of margin on each side, so
 * small pans reuse the response, and about one bucket per visible pixel.
 */
export function planResourceQuery(originMs, viewport, bounds, widthPx) {
  const span = Math.max(viewport.spanMs, 1);
  const lowest = bounds.startMs;
  const highest = bounds.startMs + bounds.spanMs;
  const startMs = Math.max(lowest, viewport.startMs - span);
  const endMs = Math.min(highest, viewport.startMs + span * 2);
  const fetchedSpan = Math.max(endMs - startMs, 1);
  const pixelsPerMs = Math.max(widthPx, 1) / span;
  const maxPoints = Math.round(Math.min(MAX_POINTS_LIMIT, Math.max(MIN_POINTS, fetchedSpan * pixelsPerMs)));
  return {
    startMs,
    endMs,
    fromMs: Math.floor(originMs + startMs),
    toMs: Math.max(Math.ceil(originMs + endMs), Math.floor(originMs + startMs) + 1),
    maxPoints,
  };
}

/**
 * Whether a loaded plan still serves the viewport: it covers it and its buckets
 * are at most four pixels wide at the current zoom.
 */
export function planCoversViewport(plan, bucketMs, viewport, widthPx) {
  if (!plan) return false;
  const end = viewport.startMs + viewport.spanMs;
  const tolerance = Math.max(bucketMs, 1);
  if (viewport.startMs < plan.startMs - tolerance || end > plan.endMs + tolerance) return false;
  const pixelsPerBucket = (bucketMs / Math.max(viewport.spanMs, 1)) * Math.max(widthPx, 1);
  return pixelsPerBucket <= 4;
}

export function buildResourceTracks(timeline, originMs) {
  const bucketMs = timeline?.bucket_ms ?? 1;
  const workload = timeline?.workload ?? {};
  const host = timeline?.host ?? {};
  const offset = (ms) => ms + bucketMs / 2 - originMs;
  const gapsOf = (track) => (track.gaps ?? []).map((gap) => [gap.from_ms - originMs, gap.to_ms - originMs]);
  const workloadGaps = gapsOf(workload);
  const hostGaps = gapsOf(host);
  const workloadBreak = breakDistance(bucketMs, workload.sample_interval_ms);
  const hostBreak = breakDistance(bucketMs, host.sample_interval_ms);
  const series = (points, gaps, gapMs, pick) => segments(
    points.map((point) => [offset(point.t_ms), pick(point)]),
    gaps,
    gapMs,
  );
  const workloadPoints = workload.points ?? [];
  const hostPoints = host.points ?? [];
  const memoryTotal = host.memory_total_bytes ?? null;
  const cpuCount = host.cpu_count > 0 ? host.cpu_count : null;
  // Workload CPU is percent of one core; with the core count it becomes a share
  // of the whole machine, like the host track. Without it, fall back to cores.
  const cpuValue = cpuCount ? (percent) => machinePercent(percent, cpuCount) : cores;

  const cgroupTracks = {
    cpu: track('cpu', cpuCount ? 'percent' : 'cores', workloadGaps, [
      line('cpuAvg', 'area', '--trace-flame-res-cpu',
        series(workloadPoints, workloadGaps, workloadBreak, (p) => cpuValue(p.cpu_percent_avg))),
      line('cpuMax', 'line', '--trace-flame-res-cpu-max',
        series(workloadPoints, workloadGaps, workloadBreak, (p) => cpuValue(p.cpu_percent_max))),
    ], cpuCount ? { floor: 1, cap: 100 } : { floor: 1 }),
    memory: track('memory', 'bytes', workloadGaps, [
      line('memory', 'area', '--trace-flame-res-memory',
        series(workloadPoints, workloadGaps, workloadBreak, (p) => p.memory_bytes_max)),
      line('anon', 'line', '--trace-flame-res-memory-anon',
        series(workloadPoints, workloadGaps, workloadBreak, (p) => p.memory_anon_bytes_max)),
    ]),
    io: track('io', 'rate', workloadGaps, [
      line('ioRead', 'line', '--trace-flame-res-io-read',
        series(workloadPoints, workloadGaps, workloadBreak, (p) => p.io_read_bytes_per_sec)),
      line('ioWrite', 'line', '--trace-flame-res-io-write',
        series(workloadPoints, workloadGaps, workloadBreak, (p) => p.io_write_bytes_per_sec)),
    ]),
    pids: track('pids', 'count', workloadGaps, [
      line('pids', 'line', '--trace-flame-res-pids',
        series(workloadPoints, workloadGaps, workloadBreak, (p) => p.pids_max)),
    ], { floor: 1 }),
  };
  const hostTracks = {
    hostCpu: track('hostCpu', 'percent', hostGaps, [
      line('hostCpuAvg', 'area', '--trace-flame-res-host',
        series(hostPoints, hostGaps, hostBreak, (p) => p.cpu_percent_avg)),
    ], { ceiling: 100 }),
    hostMemory: track('hostMemory', 'bytes', hostGaps, [
      line('hostMemory', 'area', '--trace-flame-res-host',
        series(hostPoints, hostGaps, hostBreak, (p) => p.memory_used_bytes_max)),
    ], memoryTotal ? { ceiling: memoryTotal } : {}),
    hostLoad: track('hostLoad', 'load', hostGaps, [
      line('load1', 'line', '--trace-flame-res-host',
        series(hostPoints, hostGaps, hostBreak, (p) => p.load1_max)),
    ], { floor: 1 }),
  };
  return {
    host: {
      available: Boolean(host.available),
      reasons: host.reasons ?? [],
      memoryTotal,
      cpuCount,
      tracks: hostTracks,
    },
    cgroup: {
      available: Boolean(workload.available),
      reasons: workload.reasons ?? [],
      source: workload.source ?? null,
      sourcePath: workload.source_path ?? null,
      method: workload.accounting_method ?? null,
      coverage: workload.accounting_coverage ?? null,
      summary: workload.summary ?? null,
      tracks: cgroupTracks,
      // The binding and procfs paths never report these counters.
      missing: ['io', 'pids'].filter((id) => !workloadPoints.some((p) => (
        id === 'io' ? p.io_read_bytes_per_sec != null || p.io_write_bytes_per_sec != null : p.pids_max != null
      ))),
    },
  };
}

function breakDistance(bucketMs, intervalMs) {
  return Math.max(bucketMs * 2.5, (intervalMs ?? 0) * 3);
}

/** Splits a polyline wherever a gap separates two points or points are too far apart. */
export function segments(points, gaps, breakMs) {
  const result = [];
  let current = [];
  let previous = null;
  for (const [x, y] of points) {
    if (y == null || !Number.isFinite(y)) {
      if (current.length) result.push(current);
      current = [];
      previous = null;
      continue;
    }
    if (previous !== null && (x - previous > breakMs
      || gaps.some(([from, to]) => from < x && to > previous))) {
      if (current.length) result.push(current);
      current = [];
    }
    current.push([x, y]);
    previous = x;
  }
  if (current.length) result.push(current);
  return result;
}

function line(key, style, colorVar, parts) {
  return { key, style, colorVar, segments: parts };
}

function track(id, unit, gaps, series, { floor = 0, ceiling = null, cap = Infinity } = {}) {
  const values = series.flatMap((s) => s.segments.flat().map(([, y]) => y));
  const peak = values.length ? Math.max(...values) : 0;
  const yMax = ceiling ?? Math.min(niceCeiling(Math.max(peak * 1.15, floor)), cap);
  return { id, unit, gaps, series, yMax, empty: values.length === 0 };
}

/** Rounds up to 1, 2 or 5 times a power of ten. */
export function niceCeiling(value) {
  if (!(value > 0)) return 1;
  const power = 10 ** Math.floor(Math.log10(value));
  const scaled = value / power;
  const step = scaled <= 1 ? 1 : scaled <= 2 ? 2 : scaled <= 5 ? 5 : 10;
  return step * power;
}

function cores(percent) {
  return percent == null ? null : percent / 100;
}

function machinePercent(percent, cpuCount) {
  return percent == null ? null : percent / cpuCount;
}

/** The value of each series at the point nearest to `x`, within half a break distance. */
export function valuesAt(resourceTrack, x, maxDistanceMs) {
  return resourceTrack.series.map((s) => {
    let best = null;
    for (const segment of s.segments) {
      for (const point of segment) {
        const distance = Math.abs(point[0] - x);
        if (distance <= maxDistanceMs && (best === null || distance < best.distance)) {
          best = { distance, value: point[1] };
        }
      }
    }
    return { key: s.key, value: best?.value ?? null };
  });
}

export function formatResourceValue(unit, value, t) {
  if (value == null) return '—';
  switch (unit) {
    case 'cores': {
      const shown = trim(value, value < 10 ? 2 : 1);
      return t(shown === '1' ? 'resources.units.core' : 'resources.units.cores', { value: shown });
    }
    case 'percent':
      return formatPercent(value);
    case 'bytes':
      return formatBytes(value);
    case 'rate':
      return `${formatBytes(value)}/s`;
    case 'load':
      return trim(value, 2);
    default:
      return String(Math.round(value));
  }
}

export function formatBytes(bytes) {
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let value = Math.max(bytes, 0);
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${trim(value, value < 10 && index > 0 ? 2 : value < 100 && index > 0 ? 1 : 0)} ${units[index]}`;
}

/** Small shares keep two decimals: one agent on a large machine is often under 1%. */
function formatPercent(value) {
  return `${trim(value, value < 1 ? 2 : value < 10 ? 1 : 0)}%`;
}

function trim(value, digits) {
  return Number(value.toFixed(digits)).toString();
}

/** Detail-panel rows summarizing a window, from a `max_points=1` timeline. */
export function resourceSummaryRows(timeline, t) {
  const workload = timeline?.workload;
  if (!workload?.available) return {};
  const summary = workload.summary ?? {};
  const rows = {};
  const source = workload.source ? t(`resources.source.${workload.source}`) : '';
  const cpuCount = timeline.host?.cpu_count > 0 ? timeline.host.cpu_count : null;
  // A `max_points=1` timeline has one point covering the whole window.
  const [point] = workload.points ?? [];
  if (cpuCount && point?.cpu_percent_avg != null) {
    const cpuShare = {
      avg: formatPercent(point.cpu_percent_avg / cpuCount),
      max: formatPercent((point.cpu_percent_max ?? point.cpu_percent_avg) / cpuCount),
      count: cpuCount,
    };
    rows.resource_cpu = summary.cpu_seconds == null
      ? t('resources.summary.cpuShareNoTotal', cpuShare)
      : t('resources.summary.cpuShare', {
        ...cpuShare,
        seconds: trim(summary.cpu_seconds, 2),
      });
  } else if (summary.cpu_seconds != null) {
    rows.resource_cpu = t('resources.summary.cpu', { value: trim(summary.cpu_seconds, 2) });
  }
  if (summary.memory_peak_bytes != null) {
    rows.resource_memory_peak = formatBytes(summary.memory_peak_bytes);
  }
  if (summary.io_read_bytes != null || summary.io_write_bytes != null) {
    rows.resource_io = t('resources.summary.io', {
      read: formatBytes(summary.io_read_bytes ?? 0),
      write: formatBytes(summary.io_write_bytes ?? 0),
    });
  }
  if (Object.keys(rows).length && source) {
    rows.resource_source = source;
  }
  return rows;
}
