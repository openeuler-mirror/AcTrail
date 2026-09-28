<template>
  <div class="trend-chart" :aria-busy="loading">
    <div class="trend-toolbar">
      <div class="toolbar-group">
        <span>Trend bucket</span>
        <TokenTimeBucketControl v-model="activeBucket" :buckets="TOKEN_TIME_BUCKETS" />
      </div>
      <MultiSelectFilter
        title="Chart"
        align="end"
        :options="chartModes"
        :model-value="activeModeSelection"
        :default-selected-ids="defaultChartModeIds"
        @update:model-value="activeModeSelection = $event"
      />
    </div>
    <FacetTrendChart
      :points="chartPoints"
      :facets="facets"
      :modes="activeModes"
      :format-value="formatNumber"
    />
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';

import FacetTrendChart from '../charts/FacetTrendChart.vue';
import MultiSelectFilter from './filters/MultiSelectFilter.vue';
import TokenTimeBucketControl from '../visualizations/TokenTimeBucketControl.vue';
import {
  TOKEN_TIME_BUCKETS,
  buildTokenSeriesByBucket,
  categoryFlags,
  formatNumber,
} from '../tokenModel';

const props = defineProps({
  requests: {
    type: Array,
    required: true,
  },
  selectedCategories: {
    type: Array,
    required: true,
  },
  loading: {
    type: Boolean,
    default: false,
  },
});

const chartModes = Object.freeze([
  { id: 'line', label: 'Line' },
  { id: 'bar', label: 'Histogram' },
  { id: 'kde', label: 'KDE' },
]);
const defaultChartModeIds = Object.freeze(['line', 'bar']);

const facetDefinitions = Object.freeze([
  { key: 'total', label: 'Total', field: 'total_tokens', color: 'var(--ui-chart-total)' },
  { key: 'input', label: 'Input / Prompt', field: 'prompt_tokens', color: 'var(--ui-chart-input)' },
  {
    key: 'output',
    label: 'Output / Completion',
    field: 'completion_tokens',
    color: 'var(--ui-chart-output)',
  },
  {
    key: 'reasoning',
    label: 'Reasoning',
    field: 'reasoning_tokens',
    color: 'var(--ui-chart-reasoning)',
  },
]);

const activeBucket = ref(TOKEN_TIME_BUCKETS[0].id);
const activeModeSelection = ref({});
const series = computed(() => buildTokenSeriesByBucket(props.requests, activeBucket.value));
const activeModes = computed(() => {
  const selection = activeChartModeSelection.value;
  return chartModes.filter((mode) => Boolean(selection[mode.id])).map((mode) => mode.id);
});
const activeChartModeSelection = computed(() => {
  const explicit = chartModes.some((mode) =>
    Object.prototype.hasOwnProperty.call(activeModeSelection.value, mode.id),
  );
  if (explicit) {
    return activeModeSelection.value;
  }
  return Object.fromEntries(chartModes.map((mode) => [mode.id, defaultChartModeIds.includes(mode.id)]));
});
const facets = computed(() => {
  const flags = categoryFlags(props.selectedCategories);
  return facetDefinitions.filter((facet) => facet.key === 'total' || flags[facet.key]);
});
const chartPoints = computed(() =>
  series.value.map((row) => ({
    key: row.bucket_key,
    label: chartLabel(row),
    total_tokens: row.total_tokens,
    prompt_tokens: row.prompt_tokens,
    completion_tokens: row.completion_tokens,
    reasoning_tokens: row.reasoning_tokens,
  })),
);

function chartLabel(row) {
  if (activeBucket.value === 'request' && row.bucket_detail) {
    return `${row.bucket_label} ${row.bucket_detail}`;
  }
  return row.bucket_label || row.bucket_detail || '';
}
</script>

<style scoped>
.trend-chart {
  min-width: 0;
  min-height: 0;
  height: 100%;
  padding: var(--ui-space-3xl) var(--ui-space-3xl) var(--ui-space-2xl);
  background: var(--ui-surface-soft);
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  gap: var(--ui-space-2xl);
}

.trend-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-lg);
}

.toolbar-group {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ui-space-md);
}

.toolbar-group span {
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  font-weight: var(--ui-weight-medium);
  text-transform: uppercase;
}
</style>
