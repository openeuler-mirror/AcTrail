<template>
  <span class="duration-badge" :class="{ 'is-live': live }" :title="title">
    <slot />
  </span>
</template>

<script setup>
defineProps({
  live: {
    type: Boolean,
    default: false,
  },
  title: {
    type: String,
    default: '',
  },
});
</script>

<style scoped>
.duration-badge {
  display: inline-flex;
  align-items: center;
  min-height: 18px;
  padding: 0 6px;
  border: 1px solid var(--trace-badge-duration-border);
  border-radius: 5px;
  background: var(--trace-badge-duration-bg);
  color: var(--trace-badge-duration-text);
  font-size: 10px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  line-height: 1;
  white-space: nowrap;
}

.duration-badge.is-live {
  border-color: var(--trace-badge-progress-border);
  background: var(--trace-badge-progress-bg);
  color: var(--trace-badge-progress-text);
}
</style>
