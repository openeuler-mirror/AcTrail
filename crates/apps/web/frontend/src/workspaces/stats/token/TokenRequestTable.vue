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
  min-width: var(--ui-request-table-min-width);
  border-collapse: separate;
  border-spacing: 0;
  font-size: var(--ui-font-md);
}

.request-table th,
.request-table td {
  padding: var(--ui-table-cell-padding);
  border-bottom: 1px solid var(--ui-border);
  text-align: left;
  vertical-align: top;
}

.request-table th {
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

.request-table tbody tr {
  cursor: pointer;
}

.request-table tbody tr:hover td,
.request-table tbody tr:focus td {
  background: var(--ui-accent-faint);
}

.request-table tbody tr:focus {
  outline: none;
}

.request-table code {
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: var(--ui-font-sm);
  overflow-wrap: anywhere;
}

.cell-primary {
  display: block;
  max-width: var(--ui-request-name-max-width);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.request-table small {
  display: block;
  margin-top: var(--ui-space-2xs);
  color: var(--ui-muted);
}

.numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.request-table-footer {
  min-width: var(--ui-request-table-min-width);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-lg);
  padding: var(--ui-space-lg) var(--ui-space-xl);
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.request-table-footer button {
  height: var(--ui-control-height-sm);
  padding: 0 var(--ui-space-lg);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  cursor: pointer;
  font-weight: var(--ui-weight-medium);
}

.request-table-footer button:hover {
  border-color: var(--ui-accent-soft);
  background: var(--ui-accent-muted);
}

.request-table-empty {
  min-height: var(--ui-empty-min-height);
  display: grid;
  place-items: center;
  color: var(--ui-muted);
  font-family: var(--ui-heading-font);
  font-size: var(--ui-font-display-sm);
  font-weight: var(--ui-weight-regular);
}
</style>
