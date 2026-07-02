<template>
  <div class="category-breakdown" :aria-busy="loading">
    <template v-if="rows.length">
      <div class="category-bars" aria-label="Token usage by pricing category">
        <div class="category-chart-toolbar">
          <span>Token type chart</span>
          <ChartModeControl
            v-model="activeMode"
            :modes="chartModes"
            label="Token type chart mode"
          />
        </div>
        <CategoricalDistributionChart
          :items="chartItems"
          :mode="activeMode"
          :format-value="formatNumber"
        />
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
import { formatNumber } from '../tokenModel';

const props = defineProps({
  rows: {
    type: Array,
    required: true,
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
});

const activeMode = ref(chartModes[0].id);
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

function formatPercent(value) {
  return `${(Number(value ?? 0) * 100).toFixed(1)}%`;
}

function shareSuffix(row) {
  return row.shareBasis === 'input' ? 'of input' : 'of total';
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
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
}

.category-chart-toolbar span {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
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
</style>
