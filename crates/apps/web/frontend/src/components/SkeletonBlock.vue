<template>
  <span
    class="skeleton"
    :class="{ 'skeleton-static': !animated }"
    :style="{ width, height, borderRadius: radius }"
    aria-hidden="true"
  />
</template>

<script setup>
defineProps({
  width: { type: String, default: '100%' },
  height: { type: String, default: '12px' },
  radius: { type: String, default: 'var(--ui-radius-sm)' },
  animated: { type: Boolean, default: true },
});
</script>

<style scoped>
.skeleton {
  display: block;
  background: linear-gradient(
    90deg,
    var(--ui-surface-soft) 0%,
    color-mix(in srgb, var(--ui-surface-soft) 55%, var(--ui-surface-strong)) 50%,
    var(--ui-surface-soft) 100%
  );
  background-size: 200% 100%;
  animation: skeleton-sweep 1.5s linear infinite;
}

.skeleton-static {
  animation: none;
  background: var(--ui-surface-soft);
}

@keyframes skeleton-sweep {
  from {
    background-position: 200% 0;
  }

  to {
    background-position: -200% 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .skeleton {
    animation: none;
    background: var(--ui-surface-soft);
  }
}
</style>
