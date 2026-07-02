<template>
  <div class="request-table-shell" :aria-busy="loading">
    <template v-if="requests.length">
      <table class="request-table">
        <thead>
          <tr>
            <th>Time</th>
            <th>Trace</th>
            <th>Model</th>
            <th>Provider</th>
            <th v-if="visible.input" class="numeric">Input</th>
            <th v-if="visible.output" class="numeric">Output</th>
            <th v-if="visible.reasoning" class="numeric">Reasoning</th>
            <th class="numeric">Selected Total</th>
            <th>Response Action</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="request in visibleRequests"
            :key="`${request.trace_id}:${request.response_action_id}`"
            tabindex="0"
            @click="open(request)"
            @keydown.enter.prevent="open(request)"
            @keydown.space.prevent="open(request)"
          >
            <td>{{ formatTime(request.started_at_ms) }}</td>
            <td>
              <span class="cell-primary">{{ request.trace_name }}</span>
              <small>{{ request.trace_id }}</small>
            </td>
            <td>{{ request.model || '-' }}</td>
            <td>{{ request.provider_id || '-' }}</td>
            <td v-if="visible.input" class="numeric">{{ formatOptionalNumber(request.prompt_tokens) }}</td>
            <td v-if="visible.output" class="numeric">
              {{ formatOptionalNumber(request.completion_tokens) }}
            </td>
            <td v-if="visible.reasoning" class="numeric">
              {{ formatOptionalNumber(request.reasoning_tokens) }}
            </td>
            <td class="numeric">{{ formatOptionalNumber(request.total_tokens) }}</td>
            <td><code>{{ request.response_action_id }}</code></td>
          </tr>
        </tbody>
      </table>
      <div class="request-table-footer">
        <span>Showing {{ visibleRequests.length }} of {{ requests.length }}</span>
        <button v-if="hasMore" type="button" @click="showMore">Load more</button>
      </div>
    </template>
    <div v-else class="request-table-empty">No token requests in this date range</div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

import { categoryFlags, formatOptionalNumber, formatTime } from '../tokenModel';

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
  pageSize: {
    type: Number,
    default: 100,
  },
});

const emit = defineEmits(['open-trace']);
const visibleCount = ref(props.pageSize);
const visible = computed(() => categoryFlags(props.selectedCategories));
const visibleRequests = computed(() => props.requests.slice(0, visibleCount.value));
const hasMore = computed(() => visibleCount.value < props.requests.length);

watch(
  () => [props.requests, props.pageSize],
  () => {
    visibleCount.value = props.pageSize;
  },
);

function open(request) {
  emit('open-trace', {
    traceId: request.trace_id,
  });
}

function showMore() {
  visibleCount.value = Math.min(props.requests.length, visibleCount.value + props.pageSize);
}
</script>

<style scoped>
.request-table-shell {
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: auto;
}

.request-table {
  width: 100%;
  min-width: var(--stats-request-table-min-width);
  border-collapse: separate;
  border-spacing: 0;
  font-size: var(--stats-font-md);
}

.request-table th,
.request-table td {
  padding: var(--stats-table-cell-padding);
  border-bottom: 1px solid var(--stats-border);
  text-align: left;
  vertical-align: top;
}

.request-table th {
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

.request-table tbody tr {
  cursor: pointer;
}

.request-table tbody tr:hover td,
.request-table tbody tr:focus td {
  background: var(--stats-accent-faint);
}

.request-table tbody tr:focus {
  outline: none;
}

.request-table code {
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: var(--stats-font-sm);
  overflow-wrap: anywhere;
}

.cell-primary {
  display: block;
  max-width: var(--stats-request-name-max-width);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.request-table small {
  display: block;
  margin-top: var(--stats-space-2xs);
  color: var(--stats-muted);
}

.numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.request-table-footer {
  min-width: var(--stats-request-table-min-width);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-lg) var(--stats-space-xl);
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.request-table-footer button {
  height: var(--stats-control-height-sm);
  padding: 0 var(--stats-space-lg);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  cursor: pointer;
  font-weight: var(--stats-weight-medium);
}

.request-table-footer button:hover {
  border-color: var(--stats-accent-soft);
  background: var(--stats-accent-muted);
}

.request-table-empty {
  min-height: var(--stats-empty-min-height);
  display: grid;
  place-items: center;
  color: var(--stats-muted);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-regular);
}
</style>
