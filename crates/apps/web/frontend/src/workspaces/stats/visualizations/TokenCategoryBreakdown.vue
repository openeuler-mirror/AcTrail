<template>
  <div class="category-breakdown" :aria-busy="loading">
    <template v-if="rows.length">
      <div class="category-bars" aria-label="Token usage by pricing axis">
        <div class="category-chart-toolbar">
          <div class="category-chart-title">
            <span>Token type charts</span>
            <strong>{{ visibleChartDefinitions.length }} views</strong>
          </div>
          <MultiSelectFilter
            class="chart-picker"
            title="Charts"
            :options="chartOptions"
            :model-value="chartSelectionForControl"
            :show-bulk-actions="true"
            align="end"
            @update:model-value="setChartSelection"
          />
          <ChartModeControl
            v-model="activeMode"
            :modes="chartModes"
            label="Token type chart mode"
          />
        </div>
        <p class="pricing-note">
          Charts show token counts by pricing axis. Cost is intentionally not calculated.
        </p>
        <div class="chart-grid">
          <section
            v-for="chart in visibleChartDefinitions"
            :key="chart.id"
            class="chart-card"
          >
            <header>
              <span>{{ chart.kicker }}</span>
              <strong>{{ chart.title }}</strong>
            </header>
            <CategoricalDistributionChart
              :items="chart.items"
              :mode="activeMode"
              :format-value="formatNumber"
            />
          </section>
        </div>
      </div>

      <table class="category-table">
        <thead>
          <tr>
            <th>Category</th>
            <th>Scope</th>
            <th class="numeric">Tokens</th>
            <th class="numeric">Share</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in rows" :key="`${row.key}:table`" :class="{ child: row.level > 0 }">
            <td>
              <span class="category-name">{{ row.label }}</span>
            </td>
            <td>{{ row.scope }}</td>
            <td class="numeric">{{ formatNumber(row.tokens) }}</td>
            <td class="numeric">{{ formatPercent(row.share) }} {{ shareSuffix(row) }}</td>
          </tr>
        </tbody>
      </table>
    </template>
    <div v-else class="category-empty">No token category usage in this date range</div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';

import CategoricalDistributionChart from '../charts/CategoricalDistributionChart.vue';
import ChartModeControl from '../charts/ChartModeControl.vue';
import MultiSelectFilter from '../token/filters/MultiSelectFilter.vue';
import { formatNumber } from '../tokenModel';

const props = defineProps({
  rows: {
    type: Array,
    required: true,
  },
  modelRows: {
    type: Array,
    default: () => [],
  },
  loading: {
    type: Boolean,
    default: false,
  },
});

const chartModes = Object.freeze([
  { id: 'donut', label: 'Donut' },
  { id: 'pie', label: 'Pie' },
  { id: 'bar', label: 'Bar' },
]);

const categoryColors = Object.freeze({
  input: 'var(--stats-chart-input)',
  output: 'var(--stats-chart-output)',
  reasoning: 'var(--stats-chart-reasoning)',
  cache_hit: 'var(--stats-chart-total)',
  cache_miss: 'var(--stats-chart-output)',
});

const activeMode = ref(chartModes[0].id);
const chartSelection = ref({});
const topLevelRows = computed(() => props.rows.filter((row) => Number(row.level ?? 0) === 0));
const chartItems = computed(() =>
  topLevelRows.value
    .filter((row) => Number(row.tokens ?? 0) > 0)
    .map((row) => ({
      key: row.key,
      label: row.label,
      value: row.tokens,
      color: categoryColors[row.key] ?? 'var(--stats-accent)',
    })),
);
const modelRowsWithUsage = computed(() =>
  (props.modelRows ?? []).filter((row) => Number(row.total_tokens ?? 0) > 0),
);
const hasMultipleModels = computed(() => modelRowsWithUsage.value.length > 1);
const modelDistributionItems = computed(() =>
  modelRowsWithUsage.value.map((row, index) => ({
    key: `model:${row.model}`,
    label: row.model,
    value: row.total_tokens,
    color: seriesColor(index),
  })),
);
const totalCacheItems = computed(() =>
  cacheItemsFromTotals({
    keyPrefix: 'total',
    hit: cacheHitTokens({
      prompt_cache_hit_tokens: rowTokens('cache_hit'),
      cached_prompt_tokens: rowTokens('cache_hit'),
    }),
    miss: rowTokens('cache_miss'),
  }),
);
const modelTokenTypeCharts = computed(() =>
  hasMultipleModels.value
    ? modelRowsWithUsage.value
        .map((row) => ({
          id: `model-token-type:${row.model}`,
          title: row.model,
          kicker: 'Input / Output / Reasoning',
          items: tokenTypeItemsFromModel(row),
        }))
        .filter((chart) => chart.items.length > 0)
    : [],
);
const modelCacheCharts = computed(() =>
  hasMultipleModels.value
    ? modelRowsWithUsage.value
        .map((row, index) => ({
          id: `model-cache:${row.model}`,
          title: row.model,
          kicker: 'Cache Hit / Miss',
          items: cacheItemsFromTotals({
            keyPrefix: `model:${row.model}`,
            hit: cacheHitTokens(row),
            miss: row.prompt_cache_miss_tokens,
            hitColor: seriesColor(index),
          }),
        }))
        .filter((chart) => chart.items.length > 0)
    : [],
);
const chartGroups = computed(() =>
  [
    chartItems.value.length
      ? {
          id: 'total_token_type',
          label: 'Total Token Type',
          charts: [
            {
              id: 'total-token-type',
              title: 'All selected responses',
              kicker: 'Input / Output / Reasoning',
              items: chartItems.value,
            },
          ],
        }
      : null,
    hasMultipleModels.value && modelDistributionItems.value.length > 1
      ? {
          id: 'model_distribution',
          label: 'Model Distribution',
          charts: [
            {
              id: 'model-distribution',
              title: 'Model token share',
              kicker: 'Models',
              items: modelDistributionItems.value,
            },
          ],
        }
      : null,
    modelTokenTypeCharts.value.length
      ? {
          id: 'model_token_type',
          label: 'Type by Model',
          charts: modelTokenTypeCharts.value,
        }
      : null,
    totalCacheItems.value.length
      ? {
          id: 'cache_total',
          label: 'Total Cache',
          charts: [
            {
              id: 'total-cache',
              title: 'All selected responses',
              kicker: 'Prompt Cache Hit / Miss',
              items: totalCacheItems.value,
            },
          ],
        }
      : null,
    modelCacheCharts.value.length
      ? {
          id: 'model_cache',
          label: 'Cache by Model',
          charts: modelCacheCharts.value,
        }
      : null,
  ].filter(Boolean),
);
const chartOptions = computed(() =>
  chartGroups.value.map((group) => ({
    id: group.id,
    label: group.label,
  })),
);
const normalizedChartSelection = computed(() => {
  const current = chartSelection.value ?? {};
  const options = chartOptions.value;
  if (!options.length) {
    return {};
  }
  const selectedCount = options.filter((option) => current[option.id]).length;
  if (Object.keys(current).length === 0 || selectedCount === 0) {
    return Object.fromEntries(options.map((option) => [option.id, true]));
  }
  return Object.fromEntries(options.map((option) => [option.id, Boolean(current[option.id])]));
});
const chartSelectionForControl = computed(() => normalizedChartSelection.value);
const visibleChartDefinitions = computed(() =>
  chartGroups.value
    .filter((group) => normalizedChartSelection.value[group.id])
    .flatMap((group) => group.charts),
);

function formatPercent(value) {
  return `${(Number(value ?? 0) * 100).toFixed(1)}%`;
}

function shareSuffix(row) {
  return row.shareBasis === 'input' ? 'of input' : 'of total';
}

function setChartSelection(selection) {
  chartSelection.value = selection;
}

function tokenTypeItemsFromModel(row) {
  return [
    {
      key: `${row.model}:input`,
      label: 'Input',
      value: row.prompt_tokens,
      color: categoryColors.input,
    },
    {
      key: `${row.model}:output`,
      label: 'Output',
      value: row.completion_tokens,
      color: categoryColors.output,
    },
    {
      key: `${row.model}:reasoning`,
      label: 'Reasoning',
      value: row.reasoning_tokens,
      color: categoryColors.reasoning,
    },
  ].filter((item) => Number(item.value ?? 0) > 0);
}

function cacheItemsFromTotals({ keyPrefix, hit, miss, hitColor = categoryColors.cache_hit }) {
  return [
    {
      key: `${keyPrefix}:cache_hit`,
      label: 'Cache Hit',
      value: hit,
      color: hitColor,
    },
    {
      key: `${keyPrefix}:cache_miss`,
      label: 'Cache Miss',
      value: miss,
      color: categoryColors.cache_miss,
    },
  ].filter((item) => Number(item.value ?? 0) > 0);
}

function cacheHitTokens(row) {
  return Number(row?.prompt_cache_hit_tokens || row?.cached_prompt_tokens || 0);
}

function rowTokens(key) {
  return (props.rows ?? [])
    .filter((row) => row.key === key)
    .reduce((sum, row) => sum + Number(row.tokens ?? 0), 0);
}

function seriesColor(index) {
  const colors = [
    'var(--stats-chart-total)',
    'var(--stats-chart-input)',
    'var(--stats-chart-output)',
    'var(--stats-chart-reasoning)',
    'var(--stats-accent)',
    'var(--stats-danger)',
  ];
  return colors[index % colors.length];
}
</script>

<style scoped>
.category-breakdown {
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: auto;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
}

.category-bars {
  display: grid;
  gap: var(--stats-space-xl);
  padding: var(--stats-space-2xl);
  border-bottom: 1px solid var(--stats-border);
  background: var(--stats-surface-soft);
}

.category-chart-toolbar {
  display: grid;
  grid-template-columns: minmax(180px, 0.8fr) minmax(320px, 1.4fr) auto;
  gap: var(--stats-space-lg);
  align-items: start;
}

.category-chart-title span,
.chart-card header span {
  display: block;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}

.category-chart-title strong,
.chart-card header strong {
  display: block;
  margin-top: var(--stats-heading-kicker-gap);
  color: var(--stats-text);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-medium);
}

.chart-picker {
  min-width: 0;
}

.pricing-note {
  margin: 0;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.chart-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--stats-space-xl);
}

.chart-card {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-lg);
  padding: var(--stats-panel-padding);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-lg);
  background: var(--stats-surface);
}

.chart-card :deep(.distribution-svg) {
  min-height: 180px;
  max-height: 220px;
}

.chart-card :deep(.legend text),
.chart-card :deep(.bar-label),
.chart-card :deep(.bar-value) {
  font-size: var(--stats-font-xs);
}

.category-table {
  width: 100%;
  min-width: var(--stats-category-table-min-width);
  border-collapse: separate;
  border-spacing: 0;
  font-size: var(--stats-font-md);
}

.category-table th,
.category-table td {
  padding: var(--stats-table-cell-padding);
  border-bottom: 1px solid var(--stats-border);
  text-align: left;
}

.category-table tr.child td {
  background: var(--stats-accent-muted);
}

.category-table tr.child .category-name {
  position: relative;
  display: inline-block;
  padding-left: var(--stats-table-child-indent);
  color: var(--stats-muted);
}

.category-table tr.child .category-name::before {
  position: absolute;
  left: var(--stats-table-child-marker-left);
  color: var(--stats-accent);
  content: "-";
}

.category-table th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--stats-surface-strong);
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
  backdrop-filter: var(--stats-control-filter);
}

.numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.category-empty {
  min-height: var(--stats-empty-min-height);
  display: grid;
  place-items: center;
  color: var(--stats-muted);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-regular);
}

@media (max-width: 1180px) {
  .chart-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 980px) {
  .category-chart-toolbar,
  .chart-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
