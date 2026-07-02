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
  min-height: var(--stats-empty-min-height);
  display: grid;
  place-items: center;
  color: var(--stats-muted);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-regular);
}
</style>
