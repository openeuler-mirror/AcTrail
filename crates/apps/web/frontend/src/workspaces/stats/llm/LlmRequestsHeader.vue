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
  gap: var(--ui-space-xl);
}

.llm-header h2 {
  margin: 0;
  color: var(--ui-text);
  font-size: var(--ui-font-display-lg);
  font-weight: var(--ui-weight-medium);
  line-height: var(--ui-line-height-tight);
}

.llm-header p {
  margin: 6px 0 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.controls {
  min-width: 0;
  display: flex;
  align-items: flex-end;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: var(--ui-space-sm);
}

.quick-ranges {
  display: inline-flex;
  gap: var(--ui-space-2xs);
  padding: var(--ui-space-2xs);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
}

.quick-ranges button,
.icon-button {
  min-height: var(--ui-control-height-md);
  border: 0;
  border-radius: var(--ui-radius-sm);
  background: transparent;
  color: var(--ui-text);
  cursor: pointer;
}

.quick-ranges button {
  padding: 0 var(--ui-segment-padding-x);
  font-size: var(--ui-font-sm);
}

.quick-ranges button:hover,
.icon-button:hover {
  background: var(--ui-accent-muted);
}

label {
  display: grid;
  gap: var(--ui-space-2xs);
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

input {
  height: var(--ui-control-height-md);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  font: inherit;
}

label:not(.search) input {
  width: 150px;
  padding: 0 var(--ui-space-sm);
}

.search {
  height: var(--ui-control-height-md);
  min-width: 190px;
  display: flex;
  align-items: center;
  gap: var(--ui-space-xs);
  padding: 0 var(--ui-space-sm);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
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
  width: var(--ui-control-height-md);
  display: inline-grid;
  place-items: center;
  border: 1px solid var(--ui-border);
  background: var(--ui-surface);
}
</style>
