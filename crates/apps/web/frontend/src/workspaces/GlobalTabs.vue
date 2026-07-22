<template>
  <nav class="global-tab-strip" aria-label="Workspace views">
    <button
      v-for="tab in tabs"
      :key="tab.id"
      class="global-tab-button"
      :class="{ active: modelValue === tab.id }"
      type="button"
      :aria-current="modelValue === tab.id ? 'page' : undefined"
      @click="$emit('update:modelValue', tab.id)"
    >
      <component
        :is="tab.icon"
        v-if="tab.icon"
        class="global-tab-icon"
        :size="17"
        :stroke-width="2.2"
        aria-hidden="true"
      />
      <span>{{ tab.label }}</span>
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
  align-self: center;
  width: var(--workspace-tab-strip-width);
  max-width: calc(100% - var(--workspace-tab-strip-outer-gap) - var(--workspace-tab-strip-outer-gap));
  margin-top: var(--workspace-tab-strip-margin-top);
  display: flex;
  gap: var(--workspace-tab-gap);
  min-width: 0;
  overflow-x: auto;
  padding: var(--workspace-tab-padding);
  border: 1px solid var(--workspace-tab-strip-border, var(--border));
  border-radius: var(--workspace-tab-strip-radius);
  background: var(--workspace-tab-strip-bg, var(--surface));
  box-shadow: var(--workspace-tab-strip-shadow, none);
  scrollbar-width: none;
}

.global-tab-strip::-webkit-scrollbar {
  display: none;
}

.global-tab-button {
  flex: 0 0 auto;
  height: var(--workspace-tab-button-height);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--workspace-tab-button-gap, 0.45em);
  padding: 0 var(--workspace-tab-button-padding-x);
  border: 1px solid transparent;
  border-radius: var(--workspace-tab-radius);
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  font-weight: var(--workspace-tab-weight);
  letter-spacing: 0;
  transition:
    transform 0.14s ease,
    border-color 0.14s ease,
    background 0.14s ease,
    color 0.14s ease,
    box-shadow 0.14s ease;
}

.global-tab-icon {
  flex: 0 0 auto;
  color: var(--workspace-tab-icon-color, currentColor);
  transition:
    color 0.14s ease,
    transform 0.14s ease;
}

.global-tab-button:hover,
.global-tab-button.active {
  border-color: var(--workspace-tab-active-border);
  background: var(--workspace-tab-active-bg);
  color: var(--workspace-tab-active-color);
}

.global-tab-button:focus-visible {
  outline: 2px solid var(--workspace-tab-active-color);
  outline-offset: 2px;
}

.global-tab-button:hover {
  transform: translateY(-1px);
}

.global-tab-button.active {
  box-shadow: var(--workspace-tab-active-shadow, none);
}

.global-tab-button.active .global-tab-icon {
  color: var(--workspace-tab-active-icon-color, var(--workspace-tab-active-color));
  transform: scale(1.05);
}
</style>
