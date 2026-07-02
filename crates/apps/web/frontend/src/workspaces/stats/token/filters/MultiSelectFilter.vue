<template>
  <section class="multi-select-filter" :class="alignmentClass">
    <header>
      <div class="filter-title">
        <span>{{ title }}</span>
        <strong>{{ selectedSummary }}</strong>
      </div>
      <div v-if="showBulkActions" class="bulk-actions" aria-label="Bulk selection">
        <button
          type="button"
          :disabled="disabled || !options.length"
          title="Select all"
          aria-label="Select all"
          @click="selectAll"
        >
          <CheckCheck :size="14" aria-hidden="true" />
        </button>
        <button
          type="button"
          :disabled="disabled || !options.length"
          title="Clear selection"
          aria-label="Clear selection"
          @click="clearSelection"
        >
          <X :size="14" aria-hidden="true" />
        </button>
      </div>
    </header>

    <div v-if="options.length" class="option-list">
      <label
        v-for="option in options"
        :key="option.id"
        class="option-pill"
        :class="{ selected: selectedSet.has(option.id) }"
      >
        <input
          type="checkbox"
          :checked="selectedSet.has(option.id)"
          :disabled="disabled"
          @change="toggleOption(option.id, $event.target.checked)"
        />
        <span class="option-check" aria-hidden="true">
          <Check :size="12" />
        </span>
        <span class="option-label">{{ option.label }}</span>
      </label>
    </div>
    <div v-else class="filter-empty">{{ emptyLabel }}</div>
  </section>
</template>

<script setup>
import { computed } from 'vue';
import { Check, CheckCheck, X } from '@lucide/vue';

const props = defineProps({
  title: {
    type: String,
    required: true,
  },
  options: {
    type: Array,
    required: true,
  },
  modelValue: {
    type: Object,
    required: true,
  },
  showBulkActions: {
    type: Boolean,
    default: false,
  },
  emptyLabel: {
    type: String,
    default: 'No options yet',
  },
  disabled: {
    type: Boolean,
    default: false,
  },
  align: {
    type: String,
    default: 'start',
    validator: (value) => ['start', 'end', 'stretch'].includes(value),
  },
});

const emit = defineEmits(['update:modelValue']);
const selectedSet = computed(
  () =>
    new Set(
      props.options
        .filter((option) => Boolean(props.modelValue?.[option.id]))
        .map((option) => option.id),
    ),
);
const alignmentClass = computed(() => `align-${props.align}`);
const selectedSummary = computed(() => {
  if (!props.options.length) {
    return '0';
  }
  return `${selectedSet.value.size}/${props.options.length}`;
});

function toggleOption(optionId, checked) {
  const next = selectionFromCurrentOptions();
  next[optionId] = checked;
  emit('update:modelValue', next);
}

function selectAll() {
  emit('update:modelValue', selectionFromCurrentOptions(true));
}

function clearSelection() {
  emit('update:modelValue', selectionFromCurrentOptions(false));
}

function selectionFromCurrentOptions(forceValue = null) {
  return Object.fromEntries(
    props.options.map((option) => [
      option.id,
      forceValue === null ? Boolean(props.modelValue?.[option.id]) : forceValue,
    ]),
  );
}
</script>

<style scoped>
.multi-select-filter {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-sm);
}

header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-md);
}

.filter-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: var(--stats-space-sm);
}

.filter-title span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--stats-text);
  font-size: var(--stats-font-ui);
  font-weight: var(--stats-weight-medium);
}

.filter-title strong {
  flex: 0 0 auto;
  padding: var(--stats-filter-count-padding);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
  font-variant-numeric: tabular-nums;
}

.bulk-actions {
  display: inline-flex;
  flex: 0 0 auto;
  gap: var(--stats-space-2xs);
  padding: var(--stats-space-2xs);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
}

.bulk-actions button {
  width: var(--stats-control-height-sm);
  height: calc(var(--stats-control-height-sm) - 4px);
  display: inline-grid;
  place-items: center;
  border: 0;
  border-radius: calc(var(--stats-radius-sm) - 2px);
  background: transparent;
  color: var(--stats-muted);
  cursor: pointer;
}

.bulk-actions button:hover:not(:disabled) {
  background: var(--stats-accent-muted);
  color: var(--stats-accent);
}

.bulk-actions button:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}

.option-list {
  max-height: var(--stats-model-list-max-height);
  overflow: auto;
  display: flex;
  flex-wrap: wrap;
  gap: var(--stats-space-sm);
}

.align-start .option-list {
  justify-content: flex-start;
}

.align-end .option-list {
  justify-content: flex-end;
}

.align-stretch .option-list {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(var(--stats-filter-option-min-width), 1fr));
}

.option-pill {
  min-width: 0;
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: var(--stats-space-xs);
  padding: var(--stats-chip-padding);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  cursor: pointer;
  font-size: var(--stats-font-md);
}

.align-stretch .option-pill {
  width: 100%;
}

.option-pill.selected {
  border-color: var(--stats-accent-soft);
  background: var(--stats-accent-muted);
}

.option-pill:focus-within {
  box-shadow:
    0 0 0 2px var(--stats-accent),
    0 0 0 4px var(--stats-bg-base);
}

.option-pill input {
  position: absolute;
  inset: 0;
  opacity: 0;
  cursor: inherit;
}

.option-check {
  width: var(--stats-filter-check-size);
  height: var(--stats-filter-check-size);
  display: inline-grid;
  place-items: center;
  flex: 0 0 auto;
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  color: transparent;
  background: var(--stats-surface);
}

.option-pill.selected .option-check {
  border-color: var(--stats-accent);
  background: var(--stats-accent);
  color: var(--stats-on-accent);
}

.option-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.filter-empty {
  color: var(--stats-muted);
  font-size: var(--stats-font-md);
}
</style>
