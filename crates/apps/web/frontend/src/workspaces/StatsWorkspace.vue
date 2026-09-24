<template>
  <main class="stats-workspace">
    <nav class="stats-tabs" :aria-label="t('stats.rail.aria')">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="stats-tab"
        :class="{ active: activeTab === tab.id }"
        type="button"
        :aria-current="activeTab === tab.id ? 'page' : undefined"
        @click="selectTab(tab.id)"
      >
        {{ tab.label }}
      </button>
    </nav>
    <section class="stats-content">
      <AgentStatsWorkspace
        v-if="activeTab === STATS_TAB_IDS.agentOverview"
        :traces="traces"
        :refresh-nonce="refreshNonce"
        @loading="$emit('loading', $event)"
        @open-trace="$emit('open-trace', $event)"
      />
      <LlmRequestsWorkspace
        v-else-if="activeTab === STATS_TAB_IDS.llmRequests"
        :query="query"
        @loading="$emit('loading', $event)"
        @open-trace="$emit('open-trace', $event)"
      />
      <TimeAttributionWorkspace
        v-else-if="activeTab === STATS_TAB_IDS.timeAttribution"
        :query="query"
        :refresh-nonce="refreshNonce"
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
import AgentStatsWorkspace from './stats/agent/AgentStatsWorkspace.vue';
import LlmRequestsWorkspace from './stats/llm/LlmRequestsWorkspace.vue';
import TimeAttributionWorkspace from './stats/time-attribution/TimeAttributionWorkspace.vue';

const STATS_TAB_IDS = Object.freeze({
  agentOverview: 'agent_overview',
  llmRequests: 'llm_requests',
  timeAttribution: 'time_attribution',
  alerts: 'alerts',
});

const { t } = useLocale();
const tabs = computed(() => [
  { id: STATS_TAB_IDS.agentOverview, label: t('stats.rail.agentOverview') },
  { id: STATS_TAB_IDS.llmRequests, label: t('stats.rail.llmRequests') },
  { id: STATS_TAB_IDS.timeAttribution, label: t('stats.rail.timeAttribution') },
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

const activeTab = ref(STATS_TAB_IDS.agentOverview);
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
  height: 100%;
  padding-top: var(--ui-header-offset);
  overflow: hidden;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  background: var(--ui-bg-base);
}

.stats-tabs {
  min-width: 0;
  display: flex;
  gap: 20px;
  padding: 0 var(--ui-shell-gutter);
  border-bottom: 1px solid var(--ui-border);
  overflow-x: auto;
  scrollbar-width: none;
}

.stats-tabs::-webkit-scrollbar {
  display: none;
}

.stats-tab {
  flex: 0 0 auto;
  height: 40px;
  padding: 0;
  border: 0;
  border-bottom: 2px solid transparent;
  border-radius: 0;
  background: transparent;
  color: var(--ui-muted);
  cursor: pointer;
  font-size: var(--ui-font-md);
  font-weight: var(--ui-weight-semibold);
  transition:
    border-color var(--ui-duration-fast) var(--ui-ease-out),
    color var(--ui-duration-fast) var(--ui-ease-out);
}

.stats-tab:hover {
  border-bottom-color: color-mix(in srgb, var(--ui-text) 24%, transparent);
  color: var(--ui-text);
}

.stats-tab.active {
  border-bottom-color: var(--ui-text);
  color: var(--ui-text);
}

.stats-tab:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--ui-accent) 40%, transparent);
  outline-offset: 2px;
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

  .stats-tabs {
    gap: 16px;
    padding: 0 16px;
  }
}
</style>
