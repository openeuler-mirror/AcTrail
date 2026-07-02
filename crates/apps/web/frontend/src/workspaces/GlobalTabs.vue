<template>
  <nav class="global-tab-strip" aria-label="Workspace views">
    <button
      v-for="tab in tabs"
      :key="tab.id"
      class="global-tab-button"
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
.global-tab-strip {
  display: flex;
  gap: var(--workspace-tab-gap);
  min-width: 0;
  overflow-x: auto;
  padding: var(--workspace-tab-padding);
  border-bottom: 1px solid var(--border);
  background: var(--surface);
}

.global-tab-button {
  flex: 0 0 auto;
  height: var(--workspace-tab-button-height);
  padding: 0 var(--workspace-tab-button-padding-x);
  border: 1px solid transparent;
  border-radius: var(--workspace-tab-radius);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  font-weight: var(--workspace-tab-weight);
}

.global-tab-button:hover,
.global-tab-button.active {
  border-color: var(--workspace-tab-active-border);
  background: var(--workspace-tab-active-bg);
  color: var(--workspace-tab-active-color);
}
</style>
