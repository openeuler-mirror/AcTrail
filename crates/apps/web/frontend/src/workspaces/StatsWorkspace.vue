<template>
  <main class="stats-workspace">
    <aside class="stats-rail" :aria-label="t('stats.rail.aria')">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="stats-rail-button"
        :class="{ active: activeTab === tab.id }"
        type="button"
        @click="selectTab(tab.id)"
      >
        {{ tab.label }}
      </button>
    </aside>
    <section class="stats-content">
      <LlmRequestsWorkspace
        v-if="activeTab === STATS_TAB_IDS.llmRequests"
        :query="query"
        @loading="$emit('loading', $event)"
        @open-trace="$emit('open-trace', $event)"
      />
      <AlertsWorkspace
        v-else-if="activeTab === STATS_TAB_IDS.alerts"
        :query="query"
        :refresh-nonce="refreshNonce"
        :activation-nonce="alertActivationNonce"
        :notified-alert-id="notifiedAlertId"
        @alerts-loaded="handleAlertsLoaded"
        @loading="$emit('loading', $event)"
        @open-trace="$emit('open-trace', $event)"
      />
    </section>
  </main>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

import { useLocale } from '../locale';
import AlertsWorkspace from './AlertsWorkspace.vue';
import LlmRequestsWorkspace from './stats/llm/LlmRequestsWorkspace.vue';

const STATS_TAB_IDS = Object.freeze({
  llmRequests: 'llm_requests',
  alerts: 'alerts',
});

const { t } = useLocale();
const tabs = computed(() => [
  { id: STATS_TAB_IDS.llmRequests, label: t('stats.rail.llmRequests') },
  { id: STATS_TAB_IDS.alerts, label: t('stats.rail.alerts') },
]);

const props = defineProps({
  traces: {
    type: Array,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
  refreshNonce: {
    type: Number,
    default: 0,
  },
  notifiedAlertId: {
    type: Number,
    default: 0,
  },
  alertBaselineEstablished: {
    type: Boolean,
    default: false,
  },
  pendingSelection: {
    type: Object,
    default: null,
  },
});

const emit = defineEmits([
  'alert-baseline-established',
  'alert-notification',
  'alerts-notified',
  'loading',
  'open-trace',
  'selection-consumed',
]);

const activeTab = ref(STATS_TAB_IDS.llmRequests);
const alertActivationNonce = ref(0);

watch(
  () => props.pendingSelection,
  (target) => {
    if (target?.tabId === STATS_TAB_IDS.alerts) {
      selectTab(STATS_TAB_IDS.alerts);
      emit('selection-consumed');
    }
  },
  { immediate: true },
);

function selectTab(tabId) {
  activeTab.value = tabId;
  if (tabId === STATS_TAB_IDS.alerts) {
    alertActivationNonce.value += 1;
  }
}

function handleAlertsLoaded({ latestAlertId, newCount }) {
  if (!props.alertBaselineEstablished) {
    emit('alerts-notified', Math.max(props.notifiedAlertId, latestAlertId));
    emit('alert-baseline-established');
    return;
  }
  if (newCount > 0) {
    emit('alert-notification', {
      latestAlertId,
      newCount,
    });
  }
}
</script>

<style scoped>
.stats-workspace {
  position: relative;
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

.stats-rail-button:focus-visible {
  outline: 2px solid var(--stats-accent);
  outline-offset: var(--stats-space-xs);
}

.stats-content {
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  display: flex;
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
