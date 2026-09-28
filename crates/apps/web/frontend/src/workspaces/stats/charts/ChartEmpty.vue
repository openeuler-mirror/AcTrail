<template>
  <div class="chart-empty">{{ message }}</div>
</template>

<script setup>
defineProps({
  message: {
    type: String,
    default: 'No chart data',
  },
});
</script>

<style scoped>
.chart-empty {
  min-height: var(--ui-empty-min-height);
  display: grid;
  place-items: center;
  color: var(--ui-muted);
  font-family: var(--ui-heading-font);
  font-size: var(--ui-font-display-sm);
  font-weight: var(--ui-weight-regular);
}
</style>
