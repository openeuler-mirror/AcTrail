<template>
  <span
    class="config-hint"
    @mouseenter="hovered = true"
    @mouseleave="hovered = false"
  >
    <button
      ref="trigger"
      type="button"
      :aria-label="`More information about ${label}`"
      :aria-describedby="visible ? hintId : undefined"
      :aria-expanded="visible"
      @click="togglePinned"
      @focus="focused = true"
      @blur="close"
      @keydown.esc.prevent="dismiss"
    >
      <CircleHelp :size="14" aria-hidden="true" />
    </button>
    <Teleport to=".app-shell">
      <span
        v-if="visible"
        :id="hintId"
        ref="popover"
        class="config-hint-popover"
        role="tooltip"
        :style="popoverStyle"
      >
        {{ text }}
      </span>
    </Teleport>
  </span>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useId, watch } from 'vue';
import { CircleHelp } from '@lucide/vue';

defineProps({
  label: { type: String, required: true },
  text: { type: String, required: true },
});

const hintId = `plugin-config-hint-${useId()}`;
const hovered = ref(false);
const focused = ref(false);
const pinned = ref(false);
const trigger = ref(null);
const popover = ref(null);
const popoverStyle = ref({});
const visible = computed(() => hovered.value || focused.value || pinned.value);

watch(visible, async (open) => {
  if (!open) {
    popoverStyle.value = {};
    return;
  }
  popoverStyle.value = { visibility: 'hidden' };
  await nextTick();
  updatePosition();
});

onMounted(() => {
  window.addEventListener('resize', updatePosition);
  window.addEventListener('scroll', updatePosition, true);
});

onBeforeUnmount(() => {
  window.removeEventListener('resize', updatePosition);
  window.removeEventListener('scroll', updatePosition, true);
});

function close() {
  focused.value = false;
  pinned.value = false;
}

function togglePinned(event) {
  if (pinned.value) {
    pinned.value = false;
    focused.value = false;
    hovered.value = false;
    event.currentTarget.blur();
    return;
  }
  pinned.value = true;
}

function dismiss(event) {
  pinned.value = false;
  hovered.value = false;
  event.currentTarget.blur();
}

function updatePosition() {
  if (!visible.value || !trigger.value || !popover.value) return;
  const triggerRect = trigger.value.getBoundingClientRect();
  const popoverRect = popover.value.getBoundingClientRect();
  const gap = cssPixelValue('--stats-space-sm');
  const viewportInset = cssPixelValue('--stats-space-lg');
  const left = Math.min(
    Math.max(viewportInset, triggerRect.left),
    window.innerWidth - popoverRect.width - viewportInset,
  );
  const below = triggerRect.bottom + gap;
  const above = triggerRect.top - popoverRect.height - gap;
  const top = below + popoverRect.height <= window.innerHeight - viewportInset
    ? below
    : Math.max(viewportInset, above);
  popoverStyle.value = {
    left: `${Math.max(viewportInset, left)}px`,
    top: `${top}px`,
    visibility: 'visible',
  };
}

function cssPixelValue(name) {
  const value = Number.parseFloat(getComputedStyle(trigger.value).getPropertyValue(name));
  if (!Number.isFinite(value)) throw new Error(`missing numeric CSS property ${name}`);
  return value;
}
</script>

<style scoped>
.config-hint {
  position: relative;
  display: inline-flex;
  align-items: center;
}

.config-hint > button {
  width: 1.4rem;
  height: 1.4rem;
  display: grid;
  place-items: center;
  padding: 0;
  border: 1px solid var(--stats-border-strong);
  border-radius: 50%;
  background: var(--stats-surface-strong);
  color: var(--stats-muted);
  cursor: help;
}

.config-hint > button:hover,
.config-hint > button:focus-visible,
.config-hint > button[aria-expanded="true"] {
  border-color: var(--stats-accent-soft);
  background: var(--stats-accent-muted);
  color: var(--stats-accent);
  outline: none;
}

.config-hint-popover {
  position: fixed;
  z-index: 100;
  width: max-content;
  max-width: min(22rem, calc(100vw - 2 * var(--stats-space-xl)));
  padding: var(--stats-space-md) var(--stats-space-lg);
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  box-shadow: var(--stats-shadow);
  color: var(--stats-text);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-regular);
  line-height: 1.45;
  text-transform: none;
  white-space: normal;
}

:global(.stats-theme-arc-glass) .config-hint-popover {
  border-color: rgb(15 15 20 / 14%);
  background: rgb(255 255 255 / 94%);
  box-shadow:
    0 0.65rem 1.8rem rgb(15 15 20 / 14%),
    inset 0 1px 0 rgb(255 255 255 / 80%);
}
</style>
