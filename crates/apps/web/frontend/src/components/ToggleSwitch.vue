<template>
  <label class="toggle" :class="{ 'is-disabled': disabled }">
    <input
      type="checkbox"
      class="toggle-input"
      :checked="checked"
      :disabled="disabled"
      :aria-label="ariaLabel"
      @change="$emit('change', $event.target.checked)"
    />
    <span class="toggle-track" aria-hidden="true"><span class="toggle-thumb" /></span>
    <span v-if="label" class="toggle-label">{{ label }}</span>
  </label>
</template>

<script setup>
defineProps({
  checked: { type: Boolean, default: false },
  disabled: { type: Boolean, default: false },
  label: { type: String, default: '' },
  ariaLabel: { type: String, default: '' },
});

defineEmits(['change']);
</script>

<style scoped>
.toggle {
  display: inline-flex;
  align-items: center;
  gap: var(--ui-space-sm);
  cursor: pointer;
}

.toggle.is-disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.toggle-input {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
}

.toggle-track {
  position: relative;
  width: 44px;
  height: 24px;
  flex: 0 0 auto;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-surface-soft);
  box-shadow: inset 0 0 0 1px var(--ui-border);
  transition:
    background var(--ui-duration-fast) var(--ui-ease-out),
    box-shadow var(--ui-duration-fast) var(--ui-ease-out);
}

.toggle-thumb {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--ui-surface-raised);
  box-shadow: var(--ui-shadow);
  transition: transform var(--ui-duration-fast) var(--ui-ease-out);
}

.toggle-input:checked + .toggle-track {
  background: var(--ui-accent);
  box-shadow: inset 0 0 0 1px var(--ui-accent);
}

.toggle-input:checked + .toggle-track .toggle-thumb {
  transform: translateX(20px);
}

.toggle-input:focus-visible + .toggle-track {
  outline: var(--ui-focus-ring-width) solid var(--ui-focus-ring-color);
  outline-offset: var(--ui-focus-ring-offset);
}

.toggle-label {
  color: var(--ui-text);
  font-size: var(--ui-font-sm);
}
</style>
