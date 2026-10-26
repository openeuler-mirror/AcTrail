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
  gap: var(--ui-space-sm);
  padding: var(--ui-panel-padding);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
  box-shadow: var(--ui-shadow);
}

.metric-card span {
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.metric-card strong {
  color: var(--ui-text);
  font-size: var(--ui-font-display-md);
  font-weight: var(--ui-weight-medium);
  line-height: var(--ui-line-height-tight);
}

.metric-card small {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}
</style>
