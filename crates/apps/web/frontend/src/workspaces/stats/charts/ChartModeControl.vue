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
  gap: var(--stats-space-2xs);
  padding: var(--stats-space-2xs);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface);
  backdrop-filter: var(--stats-control-filter);
}

.chart-mode-control button {
  height: var(--stats-control-height-sm);
  padding: 0 var(--stats-segment-padding-x);
  border: 0;
  border-radius: var(--stats-radius-sm);
  background: transparent;
  color: var(--stats-muted);
  cursor: pointer;
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
}

.chart-mode-control button:hover,
.chart-mode-control button.active {
  background: var(--stats-accent);
  color: var(--stats-on-accent);
}
</style>
