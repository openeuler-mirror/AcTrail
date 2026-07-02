<template>
  <div class="token-table-shell" :aria-busy="loading">
    <div class="bucket-table-toolbar">
      <span>Table bucket</span>
      <TokenTimeBucketControl v-model="activeBucket" :buckets="TOKEN_TIME_BUCKETS" />
    </div>
    <table v-if="rows.length" class="token-table">
      <thead>
        <tr>
          <th>{{ bucketLabel }}</th>
          <th class="numeric">Responses</th>
          <th class="numeric">Models</th>
          <th class="numeric">Traces</th>
          <th v-if="visible.input" class="numeric">Input</th>
          <th v-if="visible.output" class="numeric">Output</th>
          <th v-if="visible.reasoning" class="numeric">Reasoning</th>
          <th class="numeric">Selected Total</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="row in rows" :key="row.bucket_key">
          <td>
            <span class="bucket-main">{{ row.bucket_label }}</span>
            <span v-if="row.bucket_detail && row.bucket_detail !== row.bucket_label" class="bucket-detail">
              {{ row.bucket_detail }}
            </span>
          </td>
          <td class="numeric">{{ formatNumber(row.response_count) }}</td>
          <td class="numeric">{{ formatNumber(row.model_count) }}</td>
          <td class="numeric">{{ formatNumber(row.trace_count) }}</td>
          <td v-if="visible.input" class="numeric">{{ formatNumber(row.prompt_tokens) }}</td>
          <td v-if="visible.output" class="numeric">{{ formatNumber(row.completion_tokens) }}</td>
          <td v-if="visible.reasoning" class="numeric">{{ formatNumber(row.reasoning_tokens) }}</td>
          <td class="numeric">{{ formatNumber(row.total_tokens) }}</td>
        </tr>
      </tbody>
    </table>
    <div v-else class="token-table-empty">No token usage in this bucket selection</div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';

import TokenTimeBucketControl from './TokenTimeBucketControl.vue';
import {
  TOKEN_TIME_BUCKETS,
  buildTokenBreakdownByBucket,
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

const activeBucket = ref(TOKEN_TIME_BUCKETS[0].id);
const rows = computed(() => buildTokenBreakdownByBucket(props.requests, activeBucket.value));
const visible = computed(() => categoryFlags(props.selectedCategories));
const bucketLabel = computed(() => {
  const bucket = TOKEN_TIME_BUCKETS.find((item) => item.id === activeBucket.value);
  return bucket?.label ?? 'Bucket';
});
</script>

<style scoped>
.token-table-shell {
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: auto;
}

.bucket-table-toolbar {
  position: sticky;
  top: 0;
  z-index: 2;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-lg) var(--stats-space-xl);
  border-bottom: 1px solid var(--stats-border);
  background: var(--stats-surface-bar);
  backdrop-filter: var(--stats-control-filter);
}

.bucket-table-toolbar span {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}

.token-table {
  width: 100%;
  min-width: var(--stats-bucket-table-min-width);
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

.bucket-main {
  display: block;
  font-weight: var(--stats-weight-medium);
}

.bucket-detail {
  display: block;
  margin-top: var(--stats-table-subtext-gap);
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
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
