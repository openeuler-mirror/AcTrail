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
  background: transparent;
}

.navigation-strip {
  min-width: 0;
  display: flex;
  gap: 6px;
  overflow-x: auto;
  scrollbar-width: none;
}

.navigation-strip::-webkit-scrollbar {
  display: none;
}

.navigation-control-primary {
  border-bottom: 1px solid var(--ui-border);
}

.navigation-control-primary .navigation-strip {
  gap: 20px;
  padding: 0 var(--ui-shell-gutter);
}

.navigation-control-secondary .navigation-strip {
  gap: 6px;
  padding: 8px var(--ui-shell-gutter);
}

.navigation-item {
  flex: 0 0 auto;
  height: 28px;
  padding: 0 10px;
  border: 1px solid transparent;
  border-radius: 999px;
  background: transparent;
  color: var(--ui-muted);
  cursor: pointer;
  font-size: var(--ui-font-sm);
  font-weight: var(--ui-weight-medium);
  transition:
    background var(--ui-duration-fast) var(--ui-ease-out),
    border-color var(--ui-duration-fast) var(--ui-ease-out),
    color var(--ui-duration-fast) var(--ui-ease-out);
}

.navigation-control-secondary .navigation-item {
  border-color: var(--ui-border);
}

.navigation-item:hover,
.navigation-control-secondary .navigation-item:hover {
  border-color: color-mix(in srgb, var(--ui-border-hover) 80%, transparent);
  background: color-mix(in srgb, var(--ui-text) 7%, transparent);
  color: var(--ui-text);
}

.navigation-control-secondary .navigation-item.active {
  border-color: color-mix(in srgb, var(--ui-text) 24%, transparent);
  background: color-mix(in srgb, var(--ui-text) 10%, transparent);
  color: var(--ui-text);
}

.navigation-control-primary .navigation-item {
  height: 40px;
  padding: 0;
  border: 0;
  border-bottom: 2px solid transparent;
  border-radius: 0;
  color: var(--ui-muted);
  font-weight: var(--ui-weight-semibold);
}

.navigation-control-primary .navigation-item:hover {
  border-bottom-color: color-mix(in srgb, var(--ui-text) 24%, transparent);
  color: var(--ui-text);
}

.navigation-control-primary .navigation-item.active {
  border-bottom-color: var(--ui-text);
  color: var(--ui-text);
}

.navigation-item:focus-visible,
.navigation-select:focus-visible {
  outline: 2px solid var(--ui-accent, var(--trace-interactive-text));
  outline-offset: var(--ui-space-xs, 4px);
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
    height: var(--ui-control-height-md, 38px);
    display: block;
    margin: 8px 12px;
    padding: 0 10px;
    border: 1px solid var(--ui-border, var(--ui-border));
    border-radius: var(--ui-radius-sm, 8px);
    background: var(--ui-surface, var(--ui-surface));
    color: var(--ui-text, var(--ui-text));
  }
}
</style>
