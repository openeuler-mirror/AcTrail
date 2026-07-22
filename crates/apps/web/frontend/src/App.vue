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
      :refresh-nonce="refreshNonce"
      :notified-alert-id="lastNotifiedAlertId"
      :alert-baseline-established="alertBaselineEstablished"
      :pending-selection="pendingStatsSelection"
      @alerts-notified="lastNotifiedAlertId = $event"
      @alert-baseline-established="alertBaselineEstablished = true"
      @alert-notification="showAlertNotification"
      @selection-consumed="pendingStatsSelection = null"
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
      @selection-consumed="pendingTraceSelection = null"
    />

    <section class="notification-stack" aria-label="Notifications">
      <article
        v-for="notification in notifications"
        :key="notification.id"
        class="app-notification"
        role="status"
        aria-live="polite"
      >
        <span class="app-notification-indicator" aria-hidden="true"></span>
        <div class="app-notification-copy">
          <strong>{{ notification.title }}</strong>
          <span>{{ notification.message }}</span>
        </div>
        <button
          v-if="notification.actionLabel"
          class="app-notification-action"
          type="button"
          @click="runNotificationAction(notification)"
        >
          {{ notification.actionLabel }}
        </button>
        <button
          class="app-notification-dismiss"
          type="button"
          :aria-label="t('alerts.dismissToast')"
          @click="dismissNotification(notification.id)"
        >
          ×
        </button>
      </article>
    </section>

    <div v-if="error" class="error-bar">{{ error }}</div>
  </div>
</template>

<script setup>
import { computed, markRaw, onBeforeUnmount, onMounted, ref, watch } from 'vue';
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
const STATS_TAB_IDS = Object.freeze({ alerts: 'alerts' });
const TRACE_TAB_IDS = Object.freeze({ alerts: 'alerts' });
const NOTIFICATION_DURATION_STORAGE_KEY = 'actrail.notifications.duration-ms';
const DEFAULT_NOTIFICATION_DURATION_MS = 8000;

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
const pendingStatsSelection = ref(null);
const brandRepoRevealed = ref(false);
const lastNotifiedAlertId = ref(0);
const alertBaselineEstablished = ref(false);
const notifications = ref([]);
const { t } = provideLocale(selectedLanguage);
const notificationTimers = new Map();
const notificationDurationMs = readNotificationDurationMs();
let nextNotificationId = 1;

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

onBeforeUnmount(() => {
  for (const timer of notificationTimers.values()) {
    window.clearTimeout(timer);
  }
  notificationTimers.clear();
});

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
    tabId: target.tabId ?? TRACE_TAB_IDS.alerts,
    nonce: Date.now(),
  };
  activeWorkspace.value = WORKSPACE_IDS.traces;
}

function showAlertNotification({ latestAlertId, newCount }) {
  const existing = notifications.value.find((notification) => notification.action === 'open-alerts');
  const cumulativeCount = (existing?.count ?? 0) + newCount;
  if (existing) dismissNotification(existing.id);
  const notification = {
    id: nextNotificationId,
    title: t('alerts.notificationTitle'),
    message: t('alerts.newAlertsToast', { count: cumulativeCount }),
    count: cumulativeCount,
    actionLabel: t('alerts.viewAlerts'),
    action: 'open-alerts',
  };
  nextNotificationId += 1;
  notifications.value = [...notifications.value, notification];
  notificationTimers.set(notification.id, window.setTimeout(
    () => dismissNotification(notification.id),
    notificationDurationMs,
  ));
  lastNotifiedAlertId.value = Math.max(lastNotifiedAlertId.value, latestAlertId);
}

function runNotificationAction(notification) {
  if (notification.action === 'open-alerts') {
    pendingStatsSelection.value = {
      tabId: STATS_TAB_IDS.alerts,
      nonce: Date.now(),
    };
    activeWorkspace.value = WORKSPACE_IDS.stats;
  }
  dismissNotification(notification.id);
}

function dismissNotification(notificationId) {
  const timer = notificationTimers.get(notificationId);
  if (timer !== undefined) {
    window.clearTimeout(timer);
    notificationTimers.delete(notificationId);
  }
  notifications.value = notifications.value.filter((notification) => notification.id !== notificationId);
}

function readNotificationDurationMs() {
  const stored = Number(window.localStorage.getItem(NOTIFICATION_DURATION_STORAGE_KEY));
  return Number.isFinite(stored) && stored > 0 ? stored : DEFAULT_NOTIFICATION_DURATION_MS;
}

function revealBrandRepoLink() {
  brandRepoRevealed.value = true;
}
</script>

<style scoped>
.notification-stack {
  position: fixed;
  top: calc(var(--topbar-height) + var(--global-tabs-height) + var(--stats-space-lg));
  right: var(--stats-space-xl);
  z-index: 80;
  width: min(26rem, calc(100vw - 2 * var(--stats-space-xl)));
  display: grid;
  gap: var(--stats-space-md);
  pointer-events: none;
}

.app-notification {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto auto;
  align-items: center;
  gap: var(--stats-space-md);
  padding: var(--stats-space-lg);
  border: 1px solid var(--stats-accent-soft);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface-strong);
  box-shadow: var(--stats-shadow);
  color: var(--stats-text);
  pointer-events: auto;
}

.app-notification-indicator {
  width: var(--stats-space-sm);
  height: var(--stats-space-sm);
  border-radius: 50%;
  background: var(--stats-danger);
  box-shadow: 0 0 0 var(--stats-space-xs) color-mix(in srgb, var(--stats-danger) 18%, transparent);
}

.app-notification-copy {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-2xs);
}

.app-notification-copy strong {
  font-size: var(--stats-font-ui);
  font-weight: var(--stats-weight-medium);
}

.app-notification-copy span {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.app-notification-action,
.app-notification-dismiss {
  border: 0;
  background: transparent;
  color: var(--stats-accent);
  cursor: pointer;
  font: inherit;
  font-weight: var(--stats-weight-medium);
}

.app-notification-dismiss {
  color: var(--stats-muted);
  font-size: var(--stats-font-lg);
}

.app-notification-action:focus-visible,
.app-notification-dismiss:focus-visible {
  outline: 2px solid var(--stats-accent);
  outline-offset: var(--stats-space-xs);
}

@media (max-width: 47.5rem) {
  .notification-stack {
    right: var(--stats-space-lg);
    width: calc(100vw - 2 * var(--stats-space-lg));
  }

  .app-notification {
    grid-template-columns: auto minmax(0, 1fr) auto;
  }

  .app-notification-action {
    grid-column: 2 / -1;
    justify-self: start;
  }
}
</style>
