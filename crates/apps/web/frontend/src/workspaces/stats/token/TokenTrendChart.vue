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

const facetDefinitions = Object.freeze([
  { key: 'total', label: 'Total', field: 'total_tokens', color: 'var(--stats-chart-total)' },
  { key: 'input', label: 'Input / Prompt', field: 'prompt_tokens', color: 'var(--stats-chart-input)' },
  {
    key: 'output',
    label: 'Output / Completion',
    field: 'completion_tokens',
    color: 'var(--stats-chart-output)',
  },
  {
    key: 'reasoning',
    label: 'Reasoning',
    field: 'reasoning_tokens',
    color: 'var(--stats-chart-reasoning)',
  },
]);

const activeBucket = ref(TOKEN_TIME_BUCKETS[0].id);
const activeModeSelection = ref({ line: true, bar: false, kde: false });
const series = computed(() => buildTokenSeriesByBucket(props.requests, activeBucket.value));
const activeModes = computed(() =>
  chartModes.filter((mode) => Boolean(activeModeSelection.value[mode.id])).map((mode) => mode.id),
);
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
  padding: var(--stats-space-3xl) var(--stats-space-3xl) var(--stats-space-2xl);
  background: var(--stats-bg-gradient), var(--stats-surface-soft);
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  gap: var(--stats-space-2xl);
}

.trend-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
}

.toolbar-group {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--stats-space-md);
}

.toolbar-group span {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}
</style>
