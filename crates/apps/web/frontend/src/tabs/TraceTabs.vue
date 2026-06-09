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
  gap: 4px;
  min-width: 0;
  overflow-x: auto;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border);
  background: var(--surface);
}

.tab-button {
  flex: 0 0 auto;
  height: 34px;
  padding: 0 12px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
}

.tab-button:hover,
.tab-button.active {
  border-color: #bdd7d2;
  background: #eef7f5;
  color: var(--teal-deep);
}
</style>
