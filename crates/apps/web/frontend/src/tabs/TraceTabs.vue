<template>
  <nav class="tab-strip" aria-label="Trace views">
    <button
      v-for="tab in tabs"
      :key="tab.id"
      class="tab-button"
      :class="{ active: modelValue === tab.id }"
      type="button"
      @click="$emit('update:modelValue', tab.id)"
    >
      {{ tab.label }}
    </button>
  </nav>
</template>

<script setup>
defineProps({
  tabs: {
    type: Array,
    required: true,
  },
  modelValue: {
    type: String,
    required: true,
  },
});

defineEmits(['update:modelValue']);
</script>

<style scoped>
.tab-strip {
  display: flex;
  gap: var(--stats-space-xs, 4px);
  min-width: 0;
  overflow-x: auto;
  padding: var(--stats-space-sm, 10px) var(--stats-space-lg, 12px);
  border-bottom: 1px solid var(--stats-border, var(--border));
  background: var(--stats-surface-bar, var(--surface));
  backdrop-filter: var(--stats-glass-filter, none);
}

.tab-button {
  flex: 0 0 auto;
  height: var(--stats-control-height-md, 34px);
  padding: 0 var(--stats-segment-padding-x, 12px);
  border: 1px solid transparent;
  border-radius: var(--stats-radius-sm, 8px);
  background: transparent;
  color: var(--stats-muted, var(--muted));
  cursor: pointer;
  font-size: var(--stats-font-sm, inherit);
  font-weight: var(--stats-weight-medium, inherit);
}

.tab-button:hover,
.tab-button.active {
  border-color: var(--trace-interactive-border);
  background: var(--trace-interactive-bg);
  color: var(--trace-interactive-text);
}
</style>
