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
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-lg);
  background: var(--ui-surface);
  box-shadow:
    var(--ui-highlight),
    var(--ui-shadow);
  backdrop-filter: var(--ui-glass-filter);
  overflow: hidden;
}

.request-panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-lg);
  padding: var(--ui-space-lg) var(--ui-space-xl);
  border-bottom: 1px solid var(--ui-border);
  background: var(--ui-surface-bar);
}

.request-panel-header span {
  display: block;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  font-weight: var(--ui-weight-medium);
  text-transform: uppercase;
}

.request-panel-header strong {
  display: block;
  margin-top: 3px;
  color: var(--ui-text);
  font-family: var(--ui-heading-font);
  font-size: var(--ui-font-display-sm);
  font-weight: var(--ui-weight-medium);
}
</style>
