<template>
  <header class="llm-header">
    <div>
      <h2>{{ t('stats.llm.header.title') }}</h2>
      <p>{{ rangeLabel }}</p>
    </div>

    <div class="controls">
      <div class="quick-ranges" :aria-label="t('stats.llm.header.quickRanges')">
        <button v-for="range in quickRanges" :key="range.id" type="button" @click="$emit('quick-range', range.days)">
          {{ range.label }}
        </button>
      </div>
      <label>
        <span>{{ t('stats.llm.header.from') }}</span>
        <input type="date" :value="fromDate" @input="$emit('update-range', { fromDate: $event.target.value, toDate })" />
      </label>
      <label>
        <span>{{ t('stats.llm.header.to') }}</span>
        <input type="date" :value="toDate" @input="$emit('update-range', { fromDate, toDate: $event.target.value })" />
      </label>
      <label class="search">
        <Search :size="15" />
        <input
          :value="query"
          type="search"
          :placeholder="t('stats.llm.header.searchRows')"
          @input="$emit('update-query', $event.target.value)"
        />
      </label>
      <button class="icon-button" type="button" :title="t('stats.llm.common.refresh')" :disabled="loading" @click="$emit('refresh')">
        <RefreshCw :size="16" />
      </button>
      <button class="icon-button" type="button" :title="t('stats.llm.common.exportCsv')" :disabled="loading" @click="$emit('export')">
        <Download :size="16" />
      </button>
    </div>
  </header>
</template>

<script setup>
import { computed } from 'vue';
import { Download, RefreshCw, Search } from '@lucide/vue';

import { useLocale } from '../../../locale';
import { QUICK_RANGES } from './model';

const props = defineProps({
  fromDate: {
    type: String,
    required: true,
  },
  toDate: {
    type: String,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
  loading: {
    type: Boolean,
    default: false,
  },
});

defineEmits(['update-range', 'update-query', 'quick-range', 'refresh', 'export']);

const quickRanges = QUICK_RANGES;
const { t } = useLocale();
const rangeLabel = computed(() => t('stats.llm.header.dateRange', { from: props.fromDate, to: props.toDate }));
</script>

<style scoped>
.llm-header {
  min-width: 0;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--stats-space-xl);
}

.llm-header h2 {
  margin: 0;
  color: var(--stats-text);
  font-size: var(--stats-font-display-lg);
  font-weight: var(--stats-weight-medium);
  line-height: var(--stats-line-height-tight);
}

.llm-header p {
  margin: 6px 0 0;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.controls {
  min-width: 0;
  display: flex;
  align-items: flex-end;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: var(--stats-space-sm);
}

.quick-ranges {
  display: inline-flex;
  gap: var(--stats-space-2xs);
  padding: var(--stats-space-2xs);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
}

.quick-ranges button,
.icon-button {
  min-height: var(--stats-control-height-md);
  border: 0;
  border-radius: var(--stats-radius-sm);
  background: transparent;
  color: var(--stats-text);
  cursor: pointer;
}

.quick-ranges button {
  padding: 0 var(--stats-segment-padding-x);
  font-size: var(--stats-font-sm);
}

.quick-ranges button:hover,
.icon-button:hover {
  background: var(--stats-accent-muted);
}

label {
  display: grid;
  gap: var(--stats-space-2xs);
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}

input {
  height: var(--stats-control-height-md);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  font: inherit;
}

label:not(.search) input {
  width: 150px;
  padding: 0 var(--stats-space-sm);
}

.search {
  height: var(--stats-control-height-md);
  min-width: 190px;
  display: flex;
  align-items: center;
  gap: var(--stats-space-xs);
  padding: 0 var(--stats-space-sm);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
}

.search input {
  min-width: 0;
  width: 100%;
  height: auto;
  padding: 0;
  border: 0;
  background: transparent;
}

.icon-button {
  width: var(--stats-control-height-md);
  display: inline-grid;
  place-items: center;
  border: 1px solid var(--stats-border);
  background: var(--stats-surface);
}
</style>
