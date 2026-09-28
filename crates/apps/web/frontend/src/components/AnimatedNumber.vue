<template>
  <span class="animated-number">{{ display }}</span>
</template>

<script setup>
import { computed, onBeforeUnmount, ref, watch } from 'vue';

const props = defineProps({
  value: { type: [Number, String], default: null },
  format: { type: Function, default: null },
  duration: { type: Number, default: 900 },
});

const REVEAL_MS = 900;
const prefersReducedMotion = typeof window !== 'undefined'
  && typeof window.matchMedia === 'function'
  && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

const current = ref(0);
let frame = null;
let startedAt = 0;

const numericTarget = computed(() => {
  const parsed = Number(props.value);
  return Number.isFinite(parsed) ? parsed : null;
});

const display = computed(() => (numericTarget.value === null ? '—' : formatValue(current.value)));

watch(numericTarget, (target) => {
  if (target === null) {
    current.value = 0;
    return;
  }
  if (prefersReducedMotion) {
    current.value = target;
    return;
  }
  cancel();
  startedAt = performance.now();
  frame = requestAnimationFrame(step);
}, { immediate: true });

onBeforeUnmount(cancel);

function step(now) {
  const target = numericTarget.value ?? 0;
  const progress = Math.min((now - startedAt) / Math.max(props.duration, REVEAL_MS * 0.2), 1);
  current.value = target * easeOut(progress);
  if (progress < 1) {
    frame = requestAnimationFrame(step);
  } else {
    frame = null;
    current.value = target;
  }
}

function easeOut(progress) {
  return 1 - (1 - progress) ** 3;
}

function formatValue(value) {
  if (props.format) {
    return props.format(value);
  }
  return Math.round(value).toLocaleString();
}

function cancel() {
  if (frame !== null) {
    cancelAnimationFrame(frame);
    frame = null;
  }
}
</script>

<style scoped>
.animated-number {
  font-variant-numeric: tabular-nums;
}
</style>
