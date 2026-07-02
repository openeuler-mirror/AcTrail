<template>
  <main class="stats-workspace">
    <aside class="stats-rail" aria-label="Stats views">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="stats-rail-button"
        :class="{ active: activeTab === tab.id }"
        type="button"
        @click="activeTab = tab.id"
      >
        {{ tab.label }}
      </button>
    </aside>
    <section class="stats-content">
      <TokenStatTab
        v-if="activeTab === STATS_TAB_IDS.token"
        :traces="traces"
        :query="query"
        @loading="$emit('loading', $event)"
        @open-trace="$emit('open-trace', $event)"
      />
    </section>
  </main>
</template>

<script setup>
import { ref } from 'vue';

import TokenStatTab from './stats/TokenStatTab.vue';

const STATS_TAB_IDS = Object.freeze({
  token: 'token',
});

const tabs = Object.freeze([
  { id: STATS_TAB_IDS.token, label: 'Token' },
]);

defineProps({
  traces: {
    type: Array,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
});

defineEmits(['loading', 'open-trace']);

const activeTab = ref(STATS_TAB_IDS.token);
</script>

<style scoped>
.stats-workspace {
  min-width: 0;
  min-height: 0;
  height: calc(100vh - var(--topbar-height) - var(--global-tabs-height));
  overflow: hidden;
  display: grid;
  grid-template-columns: var(--stats-sidebar-width) minmax(0, 1fr);
  background: var(--stats-bg-gradient), var(--stats-bg-base);
}

.stats-rail {
  min-width: 0;
  padding: var(--stats-space-2xl) var(--stats-space-lg);
  border-right: 1px solid var(--stats-border);
  background: var(--stats-surface-bar);
  backdrop-filter: var(--stats-glass-filter);
}

.stats-rail-button {
  width: 100%;
  height: var(--stats-control-height-lg);
  padding: 0 var(--stats-space-lg);
  border: 1px solid transparent;
  border-radius: var(--stats-radius-md);
  background: transparent;
  color: var(--stats-muted);
  cursor: pointer;
  font-size: var(--stats-font-ui);
  font-weight: var(--stats-weight-medium);
  text-align: left;
}

.stats-rail-button:hover,
.stats-rail-button.active {
  border-color: var(--stats-accent-soft);
  background: var(--stats-accent-muted);
  color: var(--stats-text);
}

.stats-content {
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

@media (max-width: 760px) {
  .stats-workspace {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr);
  }

  .stats-rail {
    display: flex;
    gap: var(--stats-space-xs);
    overflow-x: auto;
    padding: var(--stats-space-md) var(--stats-space-lg);
    border-right: 0;
    border-bottom: 1px solid var(--stats-border);
  }

  .stats-rail-button {
    width: auto;
    flex: 0 0 auto;
  }
}
</style>
