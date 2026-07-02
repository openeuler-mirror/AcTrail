<template>
  <div
    class="app-shell"
    :class="themeClasses"
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
        <label class="theme-picker">
          <span>Theme</span>
          <select v-model="selectedTheme">
            <option v-for="theme in themeOptions" :key="theme.id" :value="theme.id">
              {{ theme.label }}
            </option>
          </select>
        </label>
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
    <ConfigWorkspace
      v-else-if="activeWorkspace === WORKSPACE_IDS.config"
      :query="query"
      :refresh-nonce="refreshNonce"
      @loading="setWorkspaceLoading"
    />
    <PluginsWorkspace
      v-else-if="activeWorkspace === WORKSPACE_IDS.plugins"
      :query="query"
      :refresh-nonce="refreshNonce"
      @loading="setWorkspaceLoading"
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
import ConfigWorkspace from './workspaces/ConfigWorkspace.vue';
import GlobalTabs from './workspaces/GlobalTabs.vue';
import PluginsWorkspace from './workspaces/PluginsWorkspace.vue';
import StatsWorkspace from './workspaces/StatsWorkspace.vue';
import TraceWorkspace from './workspaces/TraceWorkspace.vue';
import './workspaces/runtime.css';
import './workspaces/stats/theme.css';
import './workspaces/theme-switcher.css';

const WORKSPACE_IDS = Object.freeze({
  stats: 'stats',
  config: 'config',
  plugins: 'plugins',
  traces: 'traces',
});

const workspaceTabs = Object.freeze([
  { id: WORKSPACE_IDS.stats, label: 'Stats' },
  { id: WORKSPACE_IDS.config, label: 'Config' },
  { id: WORKSPACE_IDS.plugins, label: 'Plugins' },
  { id: WORKSPACE_IDS.traces, label: 'Traces' },
]);

const THEME_IDS = Object.freeze({
  granola: 'granola',
  neutral: 'neutral',
  dark: 'dark',
});

const themeOptions = Object.freeze([
  { id: THEME_IDS.granola, label: 'Granola' },
  { id: THEME_IDS.neutral, label: 'Neutral' },
  { id: THEME_IDS.dark, label: 'Dark' },
]);

const activeWorkspace = ref(WORKSPACE_IDS.stats);
const selectedTheme = ref(THEME_IDS.granola);
const traces = ref([]);
const query = ref('');
const error = ref('');
const refreshing = ref(false);
const workspaceLoading = ref(false);
const refreshNonce = ref(0);
const traceTitle = ref('No trace selected');
const pendingTraceSelection = ref(null);

const activeTitle = computed(() => {
  if (activeWorkspace.value === WORKSPACE_IDS.stats) {
    return 'Stats';
  }
  if (activeWorkspace.value === WORKSPACE_IDS.config) {
    return 'Current configuration';
  }
  if (activeWorkspace.value === WORKSPACE_IDS.plugins) {
    return 'Plugin enablement';
  }
  return traceTitle.value;
});
const showLoading = computed(() => refreshing.value || workspaceLoading.value);
const themeClasses = computed(() => ({
  'stats-theme': true,
  'stats-shell': true,
  'stats-theme-granola': selectedTheme.value === THEME_IDS.granola,
  'stats-theme-neutral': selectedTheme.value === THEME_IDS.neutral,
  'stats-theme-dark': selectedTheme.value === THEME_IDS.dark,
  'app-theme-granola': selectedTheme.value === THEME_IDS.granola,
  'app-theme-neutral': selectedTheme.value === THEME_IDS.neutral,
  'app-theme-dark': selectedTheme.value === THEME_IDS.dark,
}));

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
