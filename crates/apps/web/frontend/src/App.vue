<template>
  <div
    class="app-shell"
    :class="{
      'stats-theme stats-theme-granola stats-shell': activeWorkspace === WORKSPACE_IDS.stats,
    }"
  >
    <header class="topbar">
      <a
        class="brand"
        href="https://gitcode.com/openeuler/AcTrail"
        target="_blank"
        rel="noreferrer"
      >
        <span class="brand-mark">A</span>
        <div>
          <h1>AcTrail</h1>
          <p>{{ activeTitle }}</p>
        </div>
      </a>
      <div class="toolbar">
        <label class="search-box">
          <Search :size="18" aria-hidden="true" />
          <input v-model="query" type="search" placeholder="Filter" />
        </label>
        <button class="icon-button" type="button" title="Refresh" @click="refresh">
          <RefreshCw :size="18" aria-hidden="true" />
        </button>
      </div>
    </header>

    <GlobalTabs v-model="activeWorkspace" :tabs="workspaceTabs" />

    <div v-if="showLoading" class="load-progress" role="progressbar" aria-label="Loading data">
      <span class="load-progress-bar"></span>
    </div>

    <StatsWorkspace
      v-if="activeWorkspace === WORKSPACE_IDS.stats"
      :traces="traces"
      :query="query"
      @loading="setWorkspaceLoading"
      @open-trace="openTrace"
    />
    <TraceWorkspace
      v-else
      :traces="traces"
      :query="query"
      :refresh-nonce="refreshNonce"
      :pending-trace-selection="pendingTraceSelection"
      @active-title="setTraceTitle"
      @loading="setWorkspaceLoading"
    />

    <div v-if="error" class="error-bar">{{ error }}</div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { RefreshCw, Search } from '@lucide/vue';

import { clearServerCache, listTraces } from './api';
import GlobalTabs from './workspaces/GlobalTabs.vue';
import StatsWorkspace from './workspaces/StatsWorkspace.vue';
import TraceWorkspace from './workspaces/TraceWorkspace.vue';
import './workspaces/stats/theme.css';

const WORKSPACE_IDS = Object.freeze({
  stats: 'stats',
  traces: 'traces',
});

const workspaceTabs = Object.freeze([
  { id: WORKSPACE_IDS.stats, label: 'Stats' },
  { id: WORKSPACE_IDS.traces, label: 'Traces' },
]);

const activeWorkspace = ref(WORKSPACE_IDS.stats);
const traces = ref([]);
const query = ref('');
const error = ref('');
const refreshing = ref(false);
const workspaceLoading = ref(false);
const refreshNonce = ref(0);
const traceTitle = ref('No trace selected');
const pendingTraceSelection = ref(null);

const activeTitle = computed(() =>
  activeWorkspace.value === WORKSPACE_IDS.stats ? 'Stats' : traceTitle.value,
);
const showLoading = computed(() => refreshing.value || workspaceLoading.value);

onMounted(refresh);

async function refresh() {
  try {
    refreshing.value = true;
    error.value = '';
    await clearServerCache();
    const data = await listTraces();
    traces.value = data.traces ?? [];
    refreshNonce.value += 1;
  } catch (err) {
    error.value = String(err.message ?? err);
  } finally {
    refreshing.value = false;
  }
}

function setTraceTitle(title) {
  traceTitle.value = title || 'No trace selected';
}

function setWorkspaceLoading(value) {
  workspaceLoading.value = Boolean(value);
}

function openTrace(target) {
  pendingTraceSelection.value = {
    traceId: target.traceId,
    nonce: Date.now(),
  };
  activeWorkspace.value = WORKSPACE_IDS.traces;
}
</script>
