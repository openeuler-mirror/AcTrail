<template>
  <div class="navigation-control" :class="`navigation-control-${variant}`">
    <nav
      class="navigation-strip"
      role="tablist"
      :aria-label="ariaLabel"
    >
      <button
        v-for="item in items"
        :id="tabId(item.id)"
        :key="item.id"
        class="navigation-item"
        :class="{ active: modelValue === item.id }"
        type="button"
        role="tab"
        :aria-controls="controlsId"
        :aria-selected="modelValue === item.id"
        :disabled="item.disabled"
        :tabindex="modelValue === item.id ? 0 : -1"
        @click="selectItem(item.id)"
        @keydown="moveFocus($event)"
      >
        <slot name="item" :item="item" :active="modelValue === item.id">
          {{ item.label }}
        </slot>
      </button>
    </nav>

    <select
      class="navigation-select"
      :aria-label="ariaLabel"
      :value="modelValue"
      @change="selectItem($event.target.value)"
    >
      <option
        v-for="item in items"
        :key="item.id"
        :value="item.id"
        :disabled="item.disabled"
      >
        {{ item.label }}
      </option>
    </select>
  </div>
</template>

<script setup>
const props = defineProps({
  items: {
    type: Array,
    required: true,
  },
  modelValue: {
    type: String,
    required: true,
  },
  ariaLabel: {
    type: String,
    required: true,
  },
  controlsId: {
    type: String,
    required: true,
  },
  idPrefix: {
    type: String,
    required: true,
  },
  variant: {
    type: String,
    default: 'secondary',
    validator: (value) => ['primary', 'secondary'].includes(value),
  },
});

const emit = defineEmits(['update:modelValue', 'select']);

function tabId(itemId) {
  return `${props.idPrefix}-${itemId}-tab`;
}

function selectItem(itemId) {
  if (itemId === props.modelValue) {
    return;
  }
  emit('update:modelValue', itemId);
  emit('select', itemId);
}

function moveFocus(event) {
  const keys = ['ArrowLeft', 'ArrowRight', 'Home', 'End'];
  if (!keys.includes(event.key)) {
    return;
  }
  const tabs = Array.from(
    event.currentTarget.closest('[role="tablist"]')?.querySelectorAll('[role="tab"]:not(:disabled)')
      ?? [],
  );
  if (!tabs.length) {
    return;
  }
  const index = tabs.indexOf(event.currentTarget);
  if (index < 0) {
    return;
  }
  event.preventDefault();
  if (event.key === 'Home') {
    tabs[0].focus();
    return;
  }
  if (event.key === 'End') {
    tabs[tabs.length - 1].focus();
    return;
  }
  const offset = event.key === 'ArrowRight' ? 1 : -1;
  tabs[(index + offset + tabs.length) % tabs.length].focus();
}
</script>

<style scoped>
.navigation-control {
  min-width: 0;
  border-bottom: 1px solid var(--stats-border, var(--border));
  background: var(--stats-surface-bar, var(--surface));
  backdrop-filter: var(--stats-glass-filter, none);
}

.navigation-strip {
  min-width: 0;
  display: flex;
  gap: var(--stats-space-xs, 4px);
  overflow-x: auto;
  padding: var(--stats-space-sm, 10px) var(--stats-space-lg, 12px);
}

.navigation-control-secondary .navigation-strip {
  padding-top: var(--stats-space-xs, 6px);
  padding-bottom: var(--stats-space-xs, 6px);
  background: var(--surface-muted);
}

.navigation-item {
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

.navigation-control-primary .navigation-item {
  font-weight: var(--stats-weight-semibold, 700);
}

.navigation-item:hover,
.navigation-item.active {
  border-color: var(--trace-interactive-border);
  background: var(--trace-interactive-bg);
  color: var(--trace-interactive-text);
}

.navigation-item:focus-visible,
.navigation-select:focus-visible {
  outline: 2px solid var(--stats-accent, var(--trace-interactive-text));
  outline-offset: var(--stats-space-xs, 4px);
}

.navigation-select {
  display: none;
}

@media (max-width: 760px) {
  .navigation-strip {
    display: none;
  }

  .navigation-select {
    width: calc(100% - 24px);
    height: var(--stats-control-height-md, 38px);
    display: block;
    margin: 8px 12px;
    padding: 0 10px;
    border: 1px solid var(--stats-border, var(--border));
    border-radius: var(--stats-radius-sm, 8px);
    background: var(--stats-surface, var(--surface));
    color: var(--stats-text, var(--text));
  }
}
</style>
