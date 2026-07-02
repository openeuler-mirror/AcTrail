<template>
  <section class="token-summary" :aria-busy="loading">
    <div v-for="metric in metrics" :key="metric.label" class="summary-metric">
      <span>{{ metric.label }}</span>
      <strong>{{ metric.value }}</strong>
    </div>
  </section>
</template>

<script setup>
import { computed } from 'vue';

import { categoryFlags, formatNumber } from '../tokenModel';

const props = defineProps({
  summary: {
    type: Object,
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

const metrics = computed(() => {
  const flags = categoryFlags(props.selectedCategories);
  return [
    { label: 'Selected total', value: formatNumber(props.summary.total_tokens) },
    flags.input ? { label: 'Input', value: formatNumber(props.summary.prompt_tokens) } : null,
    flags.output ? { label: 'Output', value: formatNumber(props.summary.completion_tokens) } : null,
    flags.reasoning ? { label: 'Reasoning', value: formatNumber(props.summary.reasoning_tokens) } : null,
    { label: 'Responses', value: formatNumber(props.summary.response_count) },
    { label: 'Missing usage', value: formatNumber(props.summary.missing_usage_count) },
  ].filter(Boolean);
});
</script>

<style scoped>
.token-summary {
  display: grid;
  grid-template-columns: repeat(6, minmax(0, 1fr));
  gap: var(--stats-space-xl);
}

.summary-metric {
  min-width: 0;
  padding: var(--stats-space-2xl) var(--stats-space-2xl) var(--stats-panel-padding);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-lg);
  background: var(--stats-surface);
  box-shadow:
    var(--stats-highlight),
    var(--stats-shadow);
  backdrop-filter: var(--stats-glass-filter);
}

.summary-metric span {
  display: block;
  color: var(--stats-muted);
  font-size: var(--stats-font-md);
  font-weight: var(--stats-weight-regular);
}

.summary-metric strong {
  display: block;
  margin-top: var(--stats-space-xs);
  overflow-wrap: anywhere;
  color: var(--stats-text);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-lg);
  font-weight: var(--stats-weight-medium);
  line-height: var(--stats-line-height-tight);
}

@media (max-width: 1100px) {
  .token-summary {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}

@media (max-width: 760px) {
  .token-summary {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
