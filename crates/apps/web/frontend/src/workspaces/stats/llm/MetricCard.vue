<template>
  <article class="metric-card">
    <span>{{ label }}</span>
    <strong>{{ value }}</strong>
    <small v-if="detail">{{ detail }}</small>
  </article>
</template>

<script setup>
defineProps({
  label: {
    type: String,
    required: true,
  },
  value: {
    type: String,
    required: true,
  },
  detail: {
    type: String,
    default: '',
  },
});
</script>

<style scoped>
.metric-card {
  min-width: 0;
  min-height: 92px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: var(--stats-space-sm);
  padding: var(--stats-panel-padding);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  box-shadow: var(--stats-shadow);
}

.metric-card span {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.metric-card strong {
  color: var(--stats-text);
  font-size: var(--stats-font-display-md);
  font-weight: var(--stats-weight-medium);
  line-height: var(--stats-line-height-tight);
}

.metric-card small {
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}
</style>
