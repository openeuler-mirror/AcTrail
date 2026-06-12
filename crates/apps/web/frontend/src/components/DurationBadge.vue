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
  border: 1px solid #d8dfdf;
  border-radius: 5px;
  background: #edf2f1;
  color: var(--teal-deep);
  font-size: 10px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  line-height: 1;
  white-space: nowrap;
}

.duration-badge.is-live {
  border-color: #f3dbb0;
  background: #fdf4e3;
  color: #b06f04;
}
</style>
