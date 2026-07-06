<template>
  <section class="single-select-control">
    <header>
      <span>{{ title }}</span>
      <strong v-if="showActive && activeText">{{ activeText }}</strong>
    </header>

    <div class="single-select-options" role="radiogroup" :aria-label="title">
      <button
        v-for="option in options"
        :key="option.id"
        type="button"
        role="radio"
        :aria-checked="isSelected(option.id)"
        :class="{ selected: isSelected(option.id) }"
        :disabled="disabled"
        @click="$emit('update:modelValue', option.id)"
      >
        {{ option.label }}
      </button>
    </div>
  </section>
</template>

<script setup>
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
    type: [String, Number],
    required: true,
  },
  showActive: {
    type: Boolean,
    default: false,
  },
  activeText: {
    type: String,
    default: '',
  },
  disabled: {
    type: Boolean,
    default: false,
  },
});

defineEmits(['update:modelValue']);

function isSelected(optionId) {
  return String(props.modelValue) === String(optionId);
}
</script>

<style scoped>
.single-select-control {
  min-width: min(100%, 220px);
  display: grid;
  gap: var(--stats-space-sm);
}

header {
  min-height: var(--stats-control-height-sm);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-sm);
}

header span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--stats-text);
  font-size: var(--stats-font-ui);
  font-weight: var(--stats-weight-medium);
}

header strong {
  flex: 0 0 auto;
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 3px 8px;
  border: 1px solid var(--stats-accent);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-accent);
  color: var(--stats-on-accent);
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
  box-shadow: var(--stats-highlight);
}

.single-select-options {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  gap: var(--stats-space-xs);
  padding: var(--stats-space-2xs);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface);
}

button {
  min-width: 0;
  height: var(--stats-control-height-sm);
  padding: 0 var(--stats-segment-padding-x);
  border: 0;
  border-radius: calc(var(--stats-radius-sm) - 2px);
  background: transparent;
  color: var(--stats-muted);
  cursor: pointer;
  font: inherit;
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
}

button:hover:not(:disabled) {
  background: var(--stats-accent-faint);
  color: var(--stats-accent);
}

button.selected {
  background: var(--stats-accent-muted);
  color: var(--stats-accent);
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
</style>
