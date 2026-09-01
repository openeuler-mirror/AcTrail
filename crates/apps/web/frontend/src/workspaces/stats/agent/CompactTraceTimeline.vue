<template>
  <div v-if="segments.length" class="timeline" role="img" :aria-label="ariaLabel">
    <div v-for="lane in lanes" :key="lane.key" class="timeline-lane">
      <span class="timeline-lane-label">{{ lane.label }}</span>
      <div class="timeline-track">
        <span
          v-for="window in normalizedWindows"
          :key="window.id"
          class="timeline-window"
          :style="{ left: `${window.left}%`, width: `${window.width}%` }"
        />
        <span
          v-for="segment in lane.segments"
          :key="segment.id"
          class="timeline-segment"
          :class="[`segment-${lane.key}`, { 'segment-concurrent': segment.concurrent }]"
          :style="{ left: `${segment.left}%`, width: `${segment.width}%` }"
          :title="segment.title"
        />
      </div>
    </div>
    <div class="timeline-axis"><span>0</span><span>{{ formatDurationNanos(duration) }}</span></div>
  </div>
  <div v-else class="empty">{{ emptyLabel }}</div>
</template>

<script setup>
import { computed } from 'vue';
import { formatDurationNanos } from './model';

const props = defineProps({
  segments: { type: Array, default: () => [] },
  windows: { type: Array, default: () => [] },
  laneLabels: { type: Object, required: true },
  ariaLabel: { type: String, required: true },
  emptyLabel: { type: String, required: true },
});

const bounds = computed(() => {
  const rows = props.windows.length ? props.windows : props.segments;
  const starts = rows.map((row) => Number(row.start_unix_nanos));
  const ends = rows.map((row) => Number(row.end_unix_nanos));
  return { start: Math.min(...starts), end: Math.max(...ends) };
});
const duration = computed(() => Math.max(0, bounds.value.end - bounds.value.start));
const normalized = computed(() => props.segments.map((row, index) => normalizeRow(row, index, true)));
const normalizedWindows = computed(() => props.windows.map((row, index) => normalizeRow(row, index, false)));
const lanes = computed(() => ['model_side', 'agent_side', 'unattributed'].map((key) => ({
  key,
  label: props.laneLabels[key],
  segments: normalized.value.filter((segment) => segment.category === key),
})));

function normalizeRow(row, index, includeDetails) {
  const start = Number(row.start_unix_nanos);
  const rowDuration = Number(row.duration_nanos ?? Number(row.end_unix_nanos) - start);
  const total = duration.value || 1;
  return {
    id: row.id ?? `${index}-${row.start_unix_nanos}`,
    category: row.category ?? row.category_key ?? 'unattributed',
    concurrent: Boolean(row.concurrent),
    left: ((start - bounds.value.start) / total) * 100,
    width: Math.max(0, (rowDuration / total) * 100),
    title: includeDetails
      ? `${row.label ?? row.category ?? ''} · ${formatDurationNanos(rowDuration)}`
      : '',
  };
}
</script>

<style scoped>
.timeline { min-width: 0; }
.timeline-lane { display: grid; grid-template-columns: minmax(110px, 22%) 1fr; align-items: center; gap: 10px; margin-bottom: 7px; }
.timeline-lane-label { min-width: 0; color: var(--stats-muted); font-size: var(--stats-font-xs); overflow-wrap: anywhere; }
.timeline-track { position: relative; height: 28px; overflow: hidden; border: 1px solid var(--stats-border); border-radius: var(--stats-radius-sm); background: var(--stats-surface-soft); }
.timeline-window { position: absolute; inset-block: 0; min-width: 1px; background: var(--stats-surface-strong); opacity: .7; }
.timeline-segment { position: absolute; top: 4px; height: 18px; min-width: 2px; border-radius: 3px; background: var(--stats-chart-7); box-shadow: var(--stats-highlight); }
.segment-model_side { background: var(--stats-chart-1); }
.segment-agent_side { background: var(--stats-chart-2); }
.segment-unattributed { background: var(--stats-chart-7); opacity: .7; }
.segment-concurrent { border: 2px solid var(--stats-surface-strong); background: repeating-linear-gradient(135deg, var(--stats-chart-2) 0 5px, var(--stats-chart-4) 5px 10px); }
.timeline-axis { display: flex; justify-content: space-between; margin: var(--stats-space-xs) 0 0 calc(22% + 10px); color: var(--stats-muted); font-size: var(--stats-font-xs); }
.empty { padding: var(--stats-space-3xl); text-align: center; color: var(--stats-muted); }
</style>
