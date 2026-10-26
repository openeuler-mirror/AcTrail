<template>
  <section class="token-visualization">
    <div class="visualization-header">
      <div class="visualization-tabs" aria-label="Token visualizations">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          type="button"
          :class="{ active: activeTab === tab.id }"
          @click="activeTab = tab.id"
        >
          {{ tab.label }}
        </button>
      </div>
    </div>

    <TokenTrendChart
      v-if="activeTab === TAB_IDS.line"
      :requests="requests"
      :selected-categories="selectedCategories"
      :loading="loading"
    />
    <TokenCategoryBreakdown
      v-else-if="activeTab === TAB_IDS.category"
      :rows="breakdownByCategory"
      :model-rows="breakdownByModel"
      :loading="loading"
    />
    <TokenByModelTable
      v-else-if="activeTab === TAB_IDS.model"
      :rows="breakdownByModel"
      :selected-categories="selectedCategories"
      :loading="loading"
    />
    <TokenTimeBucketTable
      v-else
      :requests="requests"
      :selected-categories="selectedCategories"
      :loading="loading"
    />
  </section>
</template>

<script setup>
import { ref } from 'vue';

import TokenCategoryBreakdown from '../visualizations/TokenCategoryBreakdown.vue';
import TokenTimeBucketTable from '../visualizations/TokenTimeBucketTable.vue';
import TokenByModelTable from './TokenByModelTable.vue';
import TokenTrendChart from './TokenTrendChart.vue';

const TAB_IDS = Object.freeze({
  line: 'line',
  category: 'category',
  model: 'model',
  date: 'date',
});

const tabs = Object.freeze([
  { id: TAB_IDS.line, label: 'Trend' },
  { id: TAB_IDS.category, label: 'By Token Type' },
  { id: TAB_IDS.model, label: 'By Model' },
  { id: TAB_IDS.date, label: 'By Time Bucket' },
]);

const props = defineProps({
  requests: {
    type: Array,
    required: true,
  },
  breakdownByModel: {
    type: Array,
    required: true,
  },
  breakdownByCategory: {
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

const activeTab = ref(TAB_IDS.line);
</script>

<style scoped>
.token-visualization {
  min-width: 0;
  min-height: 0;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-lg);
  background: var(--ui-surface);
  box-shadow:
    var(--ui-highlight),
    var(--ui-shadow);
  backdrop-filter: var(--ui-glass-filter);
  overflow: hidden;
}

.visualization-header {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: flex-start;
  gap: var(--ui-space-md);
  padding: var(--ui-space-lg) var(--ui-space-xl);
  border-bottom: 1px solid var(--ui-border);
  background: var(--ui-surface-bar);
}

.visualization-tabs {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ui-space-2xs);
}

.visualization-tabs button {
  height: calc(var(--ui-control-height-md) - 2px);
  padding: 0 var(--ui-space-lg);
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: var(--ui-muted);
  cursor: pointer;
  font-size: var(--ui-font-ui);
  font-weight: var(--ui-weight-medium);
}

.visualization-tabs button:hover,
.visualization-tabs button.active {
  border-color: var(--ui-accent-soft);
  background: var(--ui-accent-muted);
  color: var(--ui-text);
}

@media (max-width: 900px) {
  .visualization-header {
    align-items: flex-start;
    flex-direction: column;
  }
}
</style>
