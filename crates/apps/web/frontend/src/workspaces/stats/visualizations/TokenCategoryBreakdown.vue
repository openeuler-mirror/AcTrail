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
  input: 'var(--ui-chart-input)',
  output: 'var(--ui-chart-output)',
  reasoning: 'var(--ui-chart-reasoning)',
  cache_hit: 'var(--ui-chart-total)',
  cache_miss: 'var(--ui-chart-output)',
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
      color: categoryColors[row.key] ?? 'var(--ui-accent)',
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
    'var(--ui-chart-total)',
    'var(--ui-chart-input)',
    'var(--ui-chart-output)',
    'var(--ui-chart-reasoning)',
    'var(--ui-accent)',
    'var(--ui-danger)',
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
  gap: var(--ui-space-xl);
  padding: var(--ui-space-2xl);
  border-bottom: 1px solid var(--ui-border);
  background: var(--ui-surface-soft);
}

.category-chart-toolbar {
  display: grid;
  grid-template-columns: minmax(180px, 0.8fr) minmax(320px, 1.4fr) auto;
  gap: var(--ui-space-lg);
  align-items: start;
}

.category-chart-title span,
.chart-card header span {
  display: block;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  font-weight: var(--ui-weight-medium);
  text-transform: uppercase;
}

.category-chart-title strong,
.chart-card header strong {
  display: block;
  margin-top: var(--ui-heading-kicker-gap);
  color: var(--ui-text);
  font-family: var(--ui-heading-font);
  font-size: var(--ui-font-display-sm);
  font-weight: var(--ui-weight-medium);
}

.chart-picker {
  min-width: 0;
}

.pricing-note {
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.chart-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--ui-space-xl);
}

.chart-card {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-lg);
  padding: var(--ui-panel-padding);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-lg);
  background: var(--ui-surface);
}

.chart-card :deep(.distribution-svg) {
  min-height: 180px;
  max-height: 220px;
}

.chart-card :deep(.legend text),
.chart-card :deep(.bar-label),
.chart-card :deep(.bar-value) {
  font-size: var(--ui-font-xs);
}

.category-table {
  width: 100%;
  min-width: var(--ui-category-table-min-width);
  border-collapse: separate;
  border-spacing: 0;
  font-size: var(--ui-font-md);
}

.category-table th,
.category-table td {
  padding: var(--ui-table-cell-padding);
  border-bottom: 1px solid var(--ui-border);
  text-align: left;
}

.category-table tr.child td {
  background: var(--ui-accent-muted);
}

.category-table tr.child .category-name {
  position: relative;
  display: inline-block;
  padding-left: var(--ui-table-child-indent);
  color: var(--ui-muted);
}

.category-table tr.child .category-name::before {
  position: absolute;
  left: var(--ui-table-child-marker-left);
  color: var(--ui-accent);
  content: "-";
}

.category-table th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--ui-surface-strong);
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-medium);
  text-transform: uppercase;
  backdrop-filter: var(--ui-control-filter);
}

.numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.category-empty {
  min-height: var(--ui-empty-min-height);
  display: grid;
  place-items: center;
  color: var(--ui-muted);
  font-family: var(--ui-heading-font);
  font-size: var(--ui-font-display-sm);
  font-weight: var(--ui-weight-regular);
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
