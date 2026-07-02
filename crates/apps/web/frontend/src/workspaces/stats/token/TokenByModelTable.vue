<template>
  <div class="token-table-shell" :aria-busy="loading">
    <table v-if="rows.length" class="token-table">
      <thead>
        <tr>
          <th>Model</th>
          <th class="numeric">Responses</th>
          <th class="numeric">Traces</th>
          <th v-if="visible.input" class="numeric">Input</th>
          <th v-if="visible.output" class="numeric">Output</th>
          <th v-if="visible.reasoning" class="numeric">Reasoning</th>
          <th class="numeric">Selected Total</th>
          <th class="numeric">Missing</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.model">
          <td>{{ row.model }}</td>
          <td class="numeric">{{ formatNumber(row.response_count) }}</td>
          <td class="numeric">{{ formatNumber(row.trace_count) }}</td>
          <td v-if="visible.input" class="numeric">{{ formatNumber(row.prompt_tokens) }}</td>
          <td v-if="visible.output" class="numeric">{{ formatNumber(row.completion_tokens) }}</td>
          <td v-if="visible.reasoning" class="numeric">{{ formatNumber(row.reasoning_tokens) }}</td>
          <td class="numeric">{{ formatNumber(row.total_tokens) }}</td>
          <td class="numeric">{{ formatNumber(row.missing_usage_count) }}</td>
        </tr>
      </tbody>
    </table>
    <div v-else class="token-table-empty">No model usage in this date range</div>
  </div>
</template>

<script setup>
import { computed } from 'vue';

import { categoryFlags, formatNumber } from '../tokenModel';

const props = defineProps({
  rows: {
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

const visible = computed(() => categoryFlags(props.selectedCategories));
</script>

<style scoped>
.token-table-shell {
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: auto;
}

.token-table {
  width: 100%;
  min-width: var(--stats-model-table-min-width);
  border-collapse: separate;
  border-spacing: 0;
  font-size: var(--stats-font-md);
}

.token-table th,
.token-table td {
  padding: var(--stats-table-cell-padding);
  border-bottom: 1px solid var(--stats-border);
  text-align: left;
  vertical-align: top;
}

.token-table th {
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

.token-table-empty {
  min-height: var(--stats-empty-min-height);
  display: grid;
  place-items: center;
  color: var(--stats-muted);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-regular);
}
</style>
