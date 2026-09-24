<template>
  <section class="multi-select-filter" :class="alignmentClass">
    <header>
      <div class="filter-title">
        <span>{{ title }}</span>
        <strong>{{ selectedSummary }}</strong>
      </div>
      <div v-if="showBulkActions" class="bulk-actions" :aria-label="t('stats.filters.bulkActions')">
        <button
          type="button"
          :disabled="disabled || !options.length"
          :title="t('stats.filters.selectAll')"
          :aria-label="t('stats.filters.selectAll')"
          @click="selectAll"
        >
          <CheckCheck :size="14" aria-hidden="true" />
        </button>
        <button
          type="button"
          :disabled="disabled || !options.length"
          :title="t('stats.filters.clearSelection')"
          :aria-label="t('stats.filters.clearSelection')"
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
    <div v-else class="filter-empty">{{ effectiveEmptyLabel }}</div>
  </section>
</template>

<script setup>
import { computed } from 'vue';
import { Check, CheckCheck, X } from '@lucide/vue';

import { useLocale } from '../../../../locale';

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
  defaultSelectedIds: {
    type: Array,
    default: () => [],
  },
  showBulkActions: {
    type: Boolean,
    default: false,
  },
  emptyLabel: {
    type: String,
    default: '',
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
const { t } = useLocale();
const optionIds = computed(() => new Set(props.options.map((option) => option.id)));
const hasExplicitSelection = computed(() =>
  Object.keys(props.modelValue ?? {}).some((id) => optionIds.value.has(id)),
);
const defaultSelectedSet = computed(
  () => new Set(props.defaultSelectedIds.filter((id) => optionIds.value.has(id))),
);
const selectedSet = computed(
  () =>
    new Set(
      props.options
        .filter((option) => isSelected(option.id))
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
const effectiveEmptyLabel = computed(() => props.emptyLabel || t('stats.filters.noOptions'));

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
      forceValue === null ? isSelected(option.id) : forceValue,
    ]),
  );
}

function isSelected(optionId) {
  if (hasExplicitSelection.value) {
    return Boolean(props.modelValue?.[optionId]);
  }
  return defaultSelectedSet.value.has(optionId);
}
</script>

<style scoped>
.multi-select-filter {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-sm);
}

header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-md);
}

.filter-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: var(--ui-space-sm);
}

.filter-title span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ui-text);
  font-size: var(--ui-font-ui);
  font-weight: var(--ui-weight-medium);
}

.filter-title strong {
  flex: 0 0 auto;
  padding: var(--ui-filter-count-padding);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-medium);
  font-variant-numeric: tabular-nums;
}

.bulk-actions {
  display: inline-flex;
  flex: 0 0 auto;
  gap: var(--ui-space-2xs);
  padding: var(--ui-space-2xs);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
}

.bulk-actions button {
  width: var(--ui-control-height-sm);
  height: calc(var(--ui-control-height-sm) - 4px);
  display: inline-grid;
  place-items: center;
  border: 0;
  border-radius: calc(var(--ui-radius-sm) - 2px);
  background: transparent;
  color: var(--ui-muted);
  cursor: pointer;
}

.bulk-actions button:hover:not(:disabled) {
  background: var(--ui-accent-muted);
  color: var(--ui-accent);
}

.bulk-actions button:disabled {
  cursor: not-allowed;
  opacity: 0.45;
}

.option-list {
  max-height: var(--ui-model-list-max-height);
  overflow: auto;
  display: flex;
  flex-wrap: wrap;
  gap: var(--ui-space-sm);
}

.align-start .option-list {
  justify-content: flex-start;
}

.align-end .option-list {
  justify-content: flex-end;
}

.align-stretch .option-list {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(var(--ui-filter-option-min-width), 1fr));
}

.option-pill {
  min-width: 0;
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: var(--ui-space-xs);
  padding: var(--ui-chip-padding);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  cursor: pointer;
  font-size: var(--ui-font-md);
}

.align-stretch .option-pill {
  width: 100%;
}

.option-pill.selected {
  border-color: var(--ui-accent-soft);
  background: var(--ui-accent-muted);
}

.option-pill:focus-within {
  box-shadow:
    0 0 0 2px var(--ui-accent),
    0 0 0 4px var(--ui-bg-base);
}

.option-pill input {
  position: absolute;
  inset: 0;
  opacity: 0;
  cursor: inherit;
}

.option-check {
  width: var(--ui-filter-check-size);
  height: var(--ui-filter-check-size);
  display: inline-grid;
  place-items: center;
  flex: 0 0 auto;
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  color: transparent;
  background: var(--ui-surface);
}

.option-pill.selected .option-check {
  border-color: var(--ui-accent);
  background: var(--ui-accent);
  color: var(--ui-on-accent);
}

.option-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.filter-empty {
  color: var(--ui-muted);
  font-size: var(--ui-font-md);
}
</style>
