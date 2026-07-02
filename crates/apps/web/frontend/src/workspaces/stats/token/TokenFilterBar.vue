<template>
  <section class="token-filter-bar">
    <header class="filter-heading">
      <span>Filters</span>
      <strong>Token usage query</strong>
    </header>
    <div class="filter-grid">
      <TokenDateFilter
        :from-date="fromDate"
        :to-date="toDate"
        :disabled="disabled"
        @update-range="$emit('update-range', $event)"
      />
      <MultiSelectFilter
        title="Models"
        align="stretch"
        :options="modelFilterOptions"
        :model-value="modelSelection"
        :show-bulk-actions="true"
        empty-label="No models yet"
        :disabled="disabled"
        @update:model-value="$emit('update-model-selection', $event)"
      />
      <MultiSelectFilter
        title="Categories"
        align="stretch"
        :options="categories"
        :model-value="categorySelection"
        :show-bulk-actions="true"
        :disabled="disabled"
        @update:model-value="$emit('update-category-selection', $event)"
      />
      <MultiSelectFilter
        title="Validity"
        align="stretch"
        :options="validityOptions"
        :model-value="validitySelection"
        :disabled="disabled"
        @update:model-value="$emit('update-validity-selection', $event)"
      />
      <div class="filter-actions">
        <button
          type="button"
          :class="{ loading }"
          :title="loading ? 'Abort query' : 'Run query'"
          @click="$emit(loading ? 'abort-query' : 'query')"
        >
          <X v-if="loading" :size="15" aria-hidden="true" />
          <Search v-else :size="15" aria-hidden="true" />
          <span>{{ loading ? 'Stop' : 'Query' }}</span>
        </button>
      </div>
    </div>
  </section>
</template>

<script setup>
import { computed } from 'vue';
import { Search, X } from '@lucide/vue';

import TokenDateFilter from './TokenDateFilter.vue';
import MultiSelectFilter from './filters/MultiSelectFilter.vue';

const props = defineProps({
  fromDate: {
    type: String,
    required: true,
  },
  toDate: {
    type: String,
    required: true,
  },
  modelOptions: {
    type: Array,
    required: true,
  },
  modelSelection: {
    type: Object,
    required: true,
  },
  categories: {
    type: Array,
    required: true,
  },
  categorySelection: {
    type: Object,
    required: true,
  },
  validitySelection: {
    type: Object,
    required: true,
  },
  loading: {
    type: Boolean,
    default: false,
  },
  disabled: {
    type: Boolean,
    default: false,
  },
});

const VALID_LLM_FILTER_ID = 'valid_llm';
const validityOptions = Object.freeze([{ id: VALID_LLM_FILTER_ID, label: 'Valid LLM' }]);

const modelFilterOptions = computed(() =>
  props.modelOptions.map((model) => ({
    id: model,
    label: model,
  })),
);

defineEmits([
  'query',
  'update-range',
  'update-model-selection',
  'update-category-selection',
  'update-validity-selection',
  'abort-query',
]);
</script>

<style scoped>
.token-filter-bar {
  min-width: 0;
  padding: var(--stats-panel-padding);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-lg);
  background: var(--stats-surface);
  box-shadow:
    var(--stats-highlight),
    var(--stats-shadow);
  backdrop-filter: var(--stats-glass-filter);
}

.filter-heading {
  margin-bottom: var(--stats-space-2xl);
}

.filter-heading span {
  display: block;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}

.filter-heading strong {
  display: block;
  margin-top: var(--stats-heading-kicker-gap);
  color: var(--stats-text);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-lg);
  font-weight: var(--stats-weight-medium);
  line-height: var(--stats-line-height-tight);
}

.filter-grid {
  display: grid;
  grid-template-columns: var(--stats-filter-grid);
  gap: var(--stats-space-2xl);
  align-items: end;
}

.filter-actions {
  display: flex;
  justify-content: flex-end;
}

.filter-actions button {
  height: var(--stats-control-height-lg);
  display: inline-flex;
  align-items: center;
  gap: var(--stats-space-xs);
  padding: 0 var(--stats-action-padding-x);
  border: 0;
  border-radius: var(--stats-radius-md);
  background: var(--stats-accent);
  color: var(--stats-on-accent);
  cursor: pointer;
  font-weight: var(--stats-weight-medium);
}

.filter-actions button.loading {
  background: var(--stats-danger);
  color: var(--stats-on-danger);
}

@media (max-width: 980px) {
  .filter-actions {
    justify-content: flex-start;
  }
}
</style>
