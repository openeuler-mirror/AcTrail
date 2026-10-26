<template>
  <div class="chart-mode-control" :aria-label="label">
    <button
      v-for="mode in modes"
      :key="mode.id"
      type="button"
      :class="{ active: modelValue === mode.id }"
      @click="$emit('update:modelValue', mode.id)"
    >
      {{ mode.label }}
    </button>
  </div>
</template>

<script setup>
defineProps({
  modes: {
    type: Array,
    required: true,
  },
  modelValue: {
    type: String,
    required: true,
  },
  label: {
    type: String,
    default: 'Chart mode',
  },
});

defineEmits(['update:modelValue']);
</script>

<style scoped>
.chart-mode-control {
  display: inline-flex;
  min-width: 0;
  flex-wrap: wrap;
  gap: var(--ui-space-2xs);
  padding: var(--ui-space-2xs);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface);
  backdrop-filter: var(--ui-control-filter);
}

.chart-mode-control button {
  height: var(--ui-control-height-sm);
  padding: 0 var(--ui-segment-padding-x);
  border: 0;
  border-radius: var(--ui-radius-sm);
  background: transparent;
  color: var(--ui-muted);
  cursor: pointer;
  font-size: var(--ui-font-sm);
  font-weight: var(--ui-weight-medium);
}

.chart-mode-control button:hover,
.chart-mode-control button.active {
  background: var(--ui-accent);
  color: var(--ui-on-accent);
}
</style>
