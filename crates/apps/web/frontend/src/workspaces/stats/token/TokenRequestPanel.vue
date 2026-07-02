<template>
  <section class="token-request-panel">
    <header class="request-panel-header">
      <div>
        <span>Token Requests</span>
        <strong>{{ requests.length }} rows</strong>
      </div>
    </header>
    <TokenRequestTable
      :requests="requests"
      :selected-categories="selectedCategories"
      :loading="loading"
      @open-trace="$emit('open-trace', $event)"
    />
  </section>
</template>

<script setup>
import TokenRequestTable from './TokenRequestTable.vue';

defineProps({
  requests: {
    type: Array,
    required: true,
  },
  selectedCategories: {
    type: Array,
    required: true,
  },
  loading: {
    type: Boolean,
    default: false,
  },
});

defineEmits(['open-trace']);
</script>

<style scoped>
.token-request-panel {
  min-width: 0;
  min-height: 0;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-lg);
  background: var(--stats-surface);
  box-shadow:
    var(--stats-highlight),
    var(--stats-shadow);
  backdrop-filter: var(--stats-glass-filter);
  overflow: hidden;
}

.request-panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-lg) var(--stats-space-xl);
  border-bottom: 1px solid var(--stats-border);
  background: var(--stats-surface-bar);
}

.request-panel-header span {
  display: block;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}

.request-panel-header strong {
  display: block;
  margin-top: 3px;
  color: var(--stats-text);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-medium);
}
</style>
