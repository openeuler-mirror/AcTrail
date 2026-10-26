<template>
  <div
    v-if="segments.length"
    class="timeline-plot"
    :class="`timeline-${mode}`"
    :style="{ '--plot-columns': plotColumns }"
    role="img"
    :aria-label="ariaLabel"
  >
    <div class="plot-labels" aria-hidden="true">
      <span v-for="lane in lanes" :key="lane.key" :title="lane.label">{{ lane.label }}</span>
    </div>
    <div class="plot-track">
      <div v-for="lane in lanes" :key="lane.key" class="plot-lane">
        <span
          v-for="segment in lane.segments"
          :key="segment.id"
          class="plot-span"
          :class="[`span-${lane.key}`, { 'span-concurrent': segment.concurrent }]"
          :style="{ left: segment.left, width: segment.width }"
          :title="segment.title"
        />
      </div>
    </div>
    <div class="plot-axis">
      <span>{{ axisStartLabel }}</span>
      <span>{{ axisEndLabel }}</span>
    </div>
  </div>
  <div v-else class="empty">{{ emptyLabel }}</div>
</template>

<script setup>
import { computed } from 'vue';

import { formatDurationNanos } from './model';

const props = defineProps({
  segments: { type: Array, default: () => [] },
  laneLabels: { type: Object, required: true },
  ariaLabel: { type: String, required: true },
  emptyLabel: { type: String, required: true },
  /**
   * `compact` gives every recorded segment one equal slot so the plot reads as
   * a regular grid, `actual` keeps the recorded start and duration.
   */
  mode: { type: String, default: 'compact' },
  /** Fold idle gaps between segments; only meaningful in `actual` mode. */
  foldIdle: { type: Boolean, default: true },
  totalLabel: { type: String, default: '' },
});

const LANE_KEYS = Object.freeze(['model_side', 'agent_side', 'unattributed']);
const MIN_WIDTH_PERCENT = 0.35;

/** Recorded spans, optionally with the idle gaps between them removed. */
const projected = computed(() => {
  const timed = props.segments
    .map((row, index) => {
      const start = Number(row.start_unix_nanos);
      const end = Number(row.end_unix_nanos);
      return {
        index,
        row,
        start: Number.isFinite(start) ? start : null,
        end: Number.isFinite(end) && Number.isFinite(start) ? Math.max(end, start) : null,
      };
    })
    .filter((entry) => entry.start !== null && entry.end !== null);

  if (props.mode !== 'actual') {
    return { entries: timed, folded: false };
  }
  if (!props.foldIdle) {
    return { entries: timed, folded: false };
  }

  const ordered = [...timed].sort((left, right) => left.start - right.start || left.end - right.end);
  const removedById = new Map();
  let removed = 0;
  let coveredUntil = null;
  for (const entry of ordered) {
    if (coveredUntil !== null && entry.start > coveredUntil) {
      removed += entry.start - coveredUntil;
    }
    removedById.set(entry.index, removed);
    coveredUntil = coveredUntil === null ? entry.end : Math.max(coveredUntil, entry.end);
  }
  return {
    entries: timed.map((entry) => {
      const offset = removedById.get(entry.index) ?? 0;
      return { ...entry, start: entry.start - offset, end: entry.end - offset };
    }),
    folded: removed > 0,
  };
});

const domain = computed(() => {
  const entries = projected.value.entries;
  if (!entries.length) {
    return null;
  }
  if (props.mode !== 'actual') {
    return { start: 0, end: entries.length, span: entries.length };
  }
  const start = Math.min(...entries.map((entry) => entry.start));
  const end = Math.max(...entries.map((entry) => entry.end));
  return { start, end, span: Math.max(end - start, 1) };
});

const lanes = computed(() => {
  const entries = projected.value.entries;
  const bounds = domain.value;
  const slots = Math.max(entries.length, 1);
  // Compact mode gives every recorded segment one slot in recording order, so
  // the lanes share a single grid instead of each packing its spans left.
  return LANE_KEYS.map((key) => ({
    key,
    label: props.laneLabels[key],
    segments: entries
      .map((entry, slot) => ({ entry, slot }))
      .filter(({ entry }) => categoryOf(entry.row) === key)
      .map(({ entry, slot }) => {
        const position = props.mode === 'actual'
          ? (entry.start - bounds.start) / bounds.span
          : slot / slots;
        const length = props.mode === 'actual'
          ? Math.max((entry.end - entry.start) / bounds.span, MIN_WIDTH_PERCENT / 100)
          : 1 / slots;
        return {
          id: entry.row.id ?? `${entry.index}-${entry.start}`,
          concurrent: Boolean(entry.row.concurrent),
          left: `${position * 100}%`,
          width: `max(2px, calc(${length * 100}% - 2px))`,
          title: `${entry.row.label ?? entry.row.category ?? ''} · ${formatDurationNanos(entry.end - entry.start)}`,
        };
      }),
  }));
});

/** Columns drawn behind the compact grid; one per recorded segment. */
const plotColumns = computed(() => Math.max(projected.value.entries.length, 1));

const axisStartLabel = computed(() => (props.mode === 'actual' ? '0' : ''));
const axisEndLabel = computed(() => {
  if (props.mode !== 'actual') {
    return `${props.segments.length}${props.totalLabel ? ` ${props.totalLabel}` : ''}`;
  }
  const bounds = domain.value;
  return bounds ? formatDurationNanos(bounds.span) : '';
});

function categoryOf(row) {
  return row.category ?? row.category_key ?? 'unattributed';
}
</script>

<style scoped>
.timeline-plot {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(96px, 20%) minmax(0, 1fr);
  grid-template-rows: 42px auto;
  column-gap: 10px;
}

.plot-labels {
  display: grid;
  grid-template-rows: repeat(3, 14px);
  align-content: start;
  padding-top: 3px;
}

.plot-labels span {
  min-width: 0;
  overflow: hidden;
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
  line-height: 14px;
  text-align: right;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plot-track {
  position: relative;
  display: grid;
  grid-template-rows: repeat(3, 14px);
  align-content: start;
  border-radius: 2px;
  background: var(--ui-surface-soft);
}

.plot-lane {
  position: relative;
}

.plot-lane + .plot-lane {
  box-shadow: inset 0 0.5px 0 color-mix(in srgb, var(--ui-border) 70%, transparent);
}

.timeline-compact .plot-lane {
  background-image: repeating-linear-gradient(
    90deg,
    transparent 0,
    transparent calc(100% / var(--plot-columns, 24) - 1px),
    color-mix(in srgb, var(--ui-border) 55%, transparent) calc(100% / var(--plot-columns, 24) - 1px),
    color-mix(in srgb, var(--ui-border) 55%, transparent) calc(100% / var(--plot-columns, 24))
  );
}

.plot-span {
  position: absolute;
  top: 3px;
  height: 8px;
  min-width: 2px;
  border-radius: 1px;
  background: var(--ui-chart-7);
  opacity: 0.78;
}

.plot-span:hover {
  opacity: 1;
  filter: brightness(1.1);
}

.span-model_side {
  background: var(--ui-chart-3);
}

.span-agent_side {
  background: var(--ui-chart-4);
}

.span-unattributed {
  background: var(--ui-chart-7);
  opacity: 0.5;
}

.span-concurrent {
  background: repeating-linear-gradient(
    135deg,
    var(--ui-chart-4) 0 4px,
    color-mix(in srgb, var(--ui-chart-4) 45%, var(--ui-surface-strong)) 4px 8px
  );
  opacity: 1;
}

.plot-axis {
  grid-column: 2;
  display: flex;
  justify-content: space-between;
  margin-top: 4px;
  color: var(--ui-muted);
  font-family: var(--ui-value-font);
  font-size: var(--ui-font-xs);
}

.empty {
  padding: var(--ui-space-3xl);
  color: var(--ui-muted);
  text-align: center;
}
</style>
