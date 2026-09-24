<template>
  <span
    ref="anchor"
    class="tooltip-anchor"
    @pointerenter="onPointerEnter"
    @pointerleave="onPointerLeave"
    @focusin="show"
    @focusout="hide"
  >
    <slot />
    <Teleport to="body">
      <span
        v-if="visible && text"
        ref="bubble"
        class="tooltip-bubble"
        :class="`tooltip-${placement}`"
        :style="bubbleStyle"
        role="tooltip"
      >
        {{ text }}
      </span>
    </Teleport>
  </span>
</template>

<script setup>
import { computed, onBeforeUnmount, ref, nextTick } from 'vue';

const props = defineProps({
  text: { type: String, default: '' },
  /** Delay before a pointer hover opens the bubble; focus opens immediately. */
  delay: { type: Number, default: 500 },
  placement: { type: String, default: 'top' },
});

const EDGE_MARGIN = 12;
const anchor = ref(null);
const bubble = ref(null);
const visible = ref(false);
const offset = ref({ top: 0, left: 0, flipped: false });
let timer = null;

const bubbleStyle = computed(() => ({
  top: `${offset.value.top}px`,
  left: `${offset.value.left}px`,
}));

onBeforeUnmount(clearTimer);

function onPointerEnter() {
  clearTimer();
  timer = window.setTimeout(show, props.delay);
}

function onPointerLeave() {
  clearTimer();
  hide();
}

function clearTimer() {
  if (timer !== null) {
    window.clearTimeout(timer);
    timer = null;
  }
}

async function show() {
  if (!props.text) {
    return;
  }
  visible.value = true;
  await nextTick();
  place();
}

function hide() {
  visible.value = false;
}

function place() {
  const element = bubble.value;
  const target = anchor.value;
  if (!element || !target) {
    return;
  }
  const rect = target.getBoundingClientRect();
  const size = element.getBoundingClientRect();
  const above = rect.top - size.height - 8 >= EDGE_MARGIN;
  const top = above ? rect.top - size.height - 8 : rect.bottom + 8;
  const left = Math.min(
    Math.max(rect.left + rect.width / 2 - size.width / 2, EDGE_MARGIN),
    window.innerWidth - size.width - EDGE_MARGIN,
  );
  offset.value = { top, left, flipped: !above };
}
</script>

<style scoped>
.tooltip-anchor {
  display: inline-flex;
  min-width: 0;
}

.tooltip-bubble {
  position: fixed;
  z-index: 3000;
  max-width: min(24rem, calc(100vw - 2 * 12px));
  padding: 3px 7px;
  border: 1px solid color-mix(in srgb, var(--ui-border) 80%, transparent);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-floating-surface);
  box-shadow: var(--ui-floating-shadow);
  color: var(--ui-text);
  font-size: var(--ui-font-sm);
  line-height: 1.4;
  text-align: left;
  white-space: normal;
  pointer-events: none;
}
</style>
