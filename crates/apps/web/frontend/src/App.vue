<template>
  <div
    class="app-shell"
    :class="themeClasses"
  >
    <header class="topbar">
      <div class="brand">
        <span class="brand-mark">A</span>
        <div class="brand-copy">
          <span
            class="brand-title-row"
            :class="{ 'is-revealed': brandRepoRevealed }"
            @pointerenter="revealBrandRepoLink"
            @focusin="revealBrandRepoLink"
          >
            <h1>AcTrail</h1>
          </span>
          <p>{{ activeTitle }}</p>
        </div>
        <a
          class="brand-repo-link"
          :class="{ 'is-revealed': brandRepoRevealed }"
          href="https://gitcode.com/openeuler/AcTrail"
          target="_blank"
          rel="noreferrer"
          aria-label="Open AcTrail repository"
          title="Open AcTrail repository"
          :tabindex="brandRepoRevealed ? 0 : -1"
        >
          <Star class="brand-repo-star" :size="28" aria-hidden="true" />
        </a>
      </div>
      <div class="toolbar">
        <ToolbarIconPicker v-model="selectedTheme" :label="t('app.controls.theme')" :options="themeOptions" />
        <ToolbarIconPicker v-model="selectedLanguage" :label="t('app.controls.language')" :options="languageOptions" />
        <label class="search-box">
          <Search :size="18" aria-hidden="true" />
          <input v-model="query" type="search" :placeholder="t('app.controls.filter')" />
        </label>
        <button class="icon-button" type="button" :title="t('app.controls.refresh')" @click="refresh">
          <RefreshCw :size="18" aria-hidden="true" />
        </button>
      </div>
    </header>

    <GlobalTabs v-model="activeWorkspace" :tabs="workspaceTabs" />

    <div v-if="showLoading" class="load-progress" role="progressbar" :aria-label="t('app.controls.loadingData')">
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
import { computed, markRaw, onMounted, ref, watch } from 'vue';
import { BarChart3, GitBranch, Puzzle, RefreshCw, Search, SlidersHorizontal, Star } from '@lucide/vue';

import { clearServerCache, listTraces } from './api';
import ToolbarIconPicker from './components/ToolbarIconPicker.vue';
import ConfigWorkspace from './workspaces/ConfigWorkspace.vue';
import GlobalTabs from './workspaces/GlobalTabs.vue';
import PluginsWorkspace from './workspaces/PluginsWorkspace.vue';
import StatsWorkspace from './workspaces/StatsWorkspace.vue';
import TraceWorkspace from './workspaces/TraceWorkspace.vue';
import { DEFAULT_LANGUAGE_ID, LANGUAGES, provideLocale } from './locale';
import { DEFAULT_THEME_ID, THEMES, loadTheme } from './theme';
import './workspaces/runtime.css';

const WORKSPACE_IDS = Object.freeze({
  stats: 'stats',
  config: 'config',
  plugins: 'plugins',
  traces: 'traces',
});
const WORKSPACE_ICONS = Object.freeze({
  [WORKSPACE_IDS.stats]: markRaw(BarChart3),
  [WORKSPACE_IDS.config]: markRaw(SlidersHorizontal),
  [WORKSPACE_IDS.plugins]: markRaw(Puzzle),
  [WORKSPACE_IDS.traces]: markRaw(GitBranch),
});

const themeOptions = THEMES;
const languageOptions = LANGUAGES;

const activeWorkspace = ref(WORKSPACE_IDS.stats);
const selectedTheme = ref(DEFAULT_THEME_ID);
const activeTheme = ref(DEFAULT_THEME_ID);
const selectedLanguage = ref(DEFAULT_LANGUAGE_ID);
const traces = ref([]);
const query = ref('');
const error = ref('');
const refreshing = ref(false);
const workspaceLoading = ref(false);
const refreshNonce = ref(0);
const traceTitle = ref('');
const pendingTraceSelection = ref(null);
const brandRepoRevealed = ref(false);
const { t } = provideLocale(selectedLanguage);

const workspaceTabs = computed(() => [
  { id: WORKSPACE_IDS.stats, label: t('app.workspaces.stats'), icon: WORKSPACE_ICONS[WORKSPACE_IDS.stats] },
  { id: WORKSPACE_IDS.config, label: t('app.workspaces.config'), icon: WORKSPACE_ICONS[WORKSPACE_IDS.config] },
  { id: WORKSPACE_IDS.plugins, label: t('app.workspaces.plugins'), icon: WORKSPACE_ICONS[WORKSPACE_IDS.plugins] },
  { id: WORKSPACE_IDS.traces, label: t('app.workspaces.traces'), icon: WORKSPACE_ICONS[WORKSPACE_IDS.traces] },
]);

const activeTitle = computed(() => {
  if (activeWorkspace.value === WORKSPACE_IDS.stats) {
    return t('app.titles.stats');
  }
  if (activeWorkspace.value === WORKSPACE_IDS.config) {
    return t('app.titles.config');
  }
  if (activeWorkspace.value === WORKSPACE_IDS.plugins) {
    return t('app.titles.plugins');
  }
  return traceTitle.value || t('app.titles.noTraceSelected');
});
const showLoading = computed(() => refreshing.value || workspaceLoading.value);
const themeClasses = computed(() => ({
  'stats-theme': true,
  'stats-shell': true,
  [`theme-${activeTheme.value}`]: true,
  [`stats-theme-${activeTheme.value}`]: true,
  [`app-theme-${activeTheme.value}`]: true,
}));

let themeLoadToken = 0;

watch(
  selectedTheme,
  (themeId) => {
    void applyTheme(themeId);
  },
  { immediate: true },
);

onMounted(refresh);

async function applyTheme(themeId) {
  const token = ++themeLoadToken;
  try {
    await loadTheme(themeId);
    if (token === themeLoadToken) {
      activeTheme.value = themeId;
    }
  } catch (err) {
    if (token === themeLoadToken) {
      selectedTheme.value = activeTheme.value;
      error.value = String(err.message ?? err);
    }
  }
}

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
  traceTitle.value = title || '';
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

function revealBrandRepoLink() {
  brandRepoRevealed.value = true;
}
</script>
