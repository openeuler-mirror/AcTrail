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
  gap: var(--ui-space-sm);
}

header {
  min-height: var(--ui-control-height-sm);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-sm);
}

header span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ui-text);
  font-size: var(--ui-font-ui);
  font-weight: var(--ui-weight-medium);
}

header strong {
  flex: 0 0 auto;
  max-width: 160px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 3px 8px;
  border: 1px solid var(--ui-accent);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-accent);
  color: var(--ui-on-accent);
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-medium);
  box-shadow: var(--ui-highlight);
}

.single-select-options {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  gap: var(--ui-space-xs);
  padding: var(--ui-space-2xs);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface);
}

button {
  min-width: 0;
  height: var(--ui-control-height-sm);
  padding: 0 var(--ui-segment-padding-x);
  border: 0;
  border-radius: calc(var(--ui-radius-sm) - 2px);
  background: transparent;
  color: var(--ui-muted);
  cursor: pointer;
  font: inherit;
  font-size: var(--ui-font-sm);
  font-weight: var(--ui-weight-medium);
}

button:hover:not(:disabled) {
  background: var(--ui-accent-faint);
  color: var(--ui-accent);
}

button.selected {
  background: var(--ui-accent-muted);
  color: var(--ui-accent);
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
</style>
