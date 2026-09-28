<template>
  <div
    class="app-shell"
    :class="{ 'is-sidebar-collapsed': sidebarCollapsed, 'is-sidebar-animating': sidebarAnimating }"
    :style="{ '--ui-sidebar-frozen-width': sidebarFrozenWidth }"
  >
    <div class="top-gradient-blur" aria-hidden="true"></div>

    <aside ref="sidebarRef" class="app-sidebar" :aria-label="t('app.nav.aria')">
      <div class="sidebar-inner">
      <div class="sidebar-brand">
        <span class="brand-mark" aria-hidden="true">A</span>
        <span class="brand-copy">
          <strong>AcTrail</strong>
          <small>{{ t('app.brandTagline') }}</small>
        </span>
      </div>

      <nav class="sidebar-nav">
        <div v-for="group in navGroups" :key="group.id" class="sidebar-group">
          <span class="sidebar-group-label">{{ group.label }}</span>
          <Tooltip
            v-for="item in group.items"
            :key="item.id"
            :text="sidebarCollapsed ? item.label : ''"
            placement="bottom"
          >
            <button
              class="sidebar-nav-item"
              :class="{ active: activeWorkspace === item.id }"
              type="button"
              :aria-current="activeWorkspace === item.id ? 'page' : undefined"
              @click="activeWorkspace = item.id"
            >
              <span class="nav-icon" aria-hidden="true">
                <component :is="item.icon" :size="16" :stroke-width="1.9" />
              </span>
              <span class="nav-text">
                <span class="nav-label">{{ item.label }}</span>
              </span>
            </button>
          </Tooltip>
        </div>
      </nav>

      <div class="sidebar-footer">
        <a
          class="sidebar-repo-link"
          href="https://gitcode.com/openeuler/AcTrail"
          target="_blank"
          rel="noreferrer"
          :aria-label="t('app.nav.repository')"
          :title="t('app.nav.repository')"
        >
          <span class="nav-icon" aria-hidden="true"><Star :size="16" /></span>
          <span class="nav-text"><span class="nav-label">{{ t('app.nav.repository') }}</span></span>
        </a>
      </div>
      </div>
    </aside>

    <button
      class="sidebar-toggle"
      type="button"
      :title="sidebarCollapsed ? t('app.nav.expandSidebar') : t('app.nav.collapseSidebar')"
      :aria-label="sidebarCollapsed ? t('app.nav.expandSidebar') : t('app.nav.collapseSidebar')"
      :aria-expanded="!sidebarCollapsed"
      @click="toggleSidebar"
    >
      <ChevronLeft :size="14" aria-hidden="true" />
    </button>

    <div class="app-main">
      <a class="skip-link" href="#app-content" @click="focusContent">{{ t('app.nav.skipToContent') }}</a>
      <header class="app-header">
        <div class="floating-actions">
          <label class="search-box">
            <Search :size="16" aria-hidden="true" />
            <input v-model="query" type="search" :placeholder="t('app.controls.filter')" />
          </label>
          <span class="floating-divider" aria-hidden="true"></span>
          <ToolbarIconPicker v-model="selectedTheme" :label="t('app.controls.theme')" :options="themeOptions" />
          <ToolbarIconPicker v-model="selectedLanguage" :label="t('app.controls.language')" :options="languageOptions" />
          <button class="icon-button" type="button" :title="t('app.controls.refresh')" @click="refresh">
            <RefreshCw :size="16" aria-hidden="true" />
          </button>
        </div>
      </header>

      <div v-if="showLoading" class="load-progress" role="progressbar" :aria-label="t('app.controls.loadingData')">
        <span class="load-progress-bar"></span>
      </div>

      <div id="app-content" ref="contentRef" class="app-content" tabindex="-1">
        <DashboardWorkspace
          v-if="activeWorkspace === WORKSPACE_IDS.dashboard"
          :refresh-nonce="refreshNonce"
          @open-workspace="activeWorkspace = $event"
          @loading="setWorkspaceLoading"
        />
        <StatsWorkspace
          v-else-if="activeWorkspace === WORKSPACE_IDS.stats"
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
      </div>
    </div>

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
import {
  BarChart3,
  ChevronLeft,
  GitBranch,
  LayoutDashboard,
  Puzzle,
  RefreshCw,
  Search,
  SlidersHorizontal,
  Star,
} from '@lucide/vue';

import { clearServerCache, listTraces } from './api';
import ToolbarIconPicker from './components/ToolbarIconPicker.vue';
import Tooltip from './components/Tooltip.vue';
import DashboardWorkspace from './workspaces/DashboardWorkspace.vue';
import ConfigWorkspace from './workspaces/ConfigWorkspace.vue';
import PluginsWorkspace from './workspaces/PluginsWorkspace.vue';
import StatsWorkspace from './workspaces/StatsWorkspace.vue';
import TraceWorkspace from './workspaces/TraceWorkspace.vue';
import { DEFAULT_LANGUAGE_ID, LANGUAGES, provideLocale } from './locale';
import { DEFAULT_THEME_MODE, THEME_MODES, applyThemeMode } from './theme';
import './workspaces/runtime.css';

const WORKSPACE_IDS = Object.freeze({
  dashboard: 'dashboard',
  stats: 'stats',
  config: 'config',
  plugins: 'plugins',
  traces: 'traces',
});
const NAV_GROUP_IDS = Object.freeze({ observe: 'observe', control: 'control' });
const WORKSPACE_ICONS = Object.freeze({
  [WORKSPACE_IDS.dashboard]: markRaw(LayoutDashboard),
  [WORKSPACE_IDS.stats]: markRaw(BarChart3),
  [WORKSPACE_IDS.traces]: markRaw(GitBranch),
  [WORKSPACE_IDS.config]: markRaw(SlidersHorizontal),
  [WORKSPACE_IDS.plugins]: markRaw(Puzzle),
});
const NAV_GROUP_MEMBERS = Object.freeze({
  [NAV_GROUP_IDS.observe]: [WORKSPACE_IDS.dashboard, WORKSPACE_IDS.stats, WORKSPACE_IDS.traces],
  [NAV_GROUP_IDS.control]: [WORKSPACE_IDS.config, WORKSPACE_IDS.plugins],
});
const NAV_GROUP_IDS_ORDER = Object.freeze([NAV_GROUP_IDS.observe, NAV_GROUP_IDS.control]);
const STATS_TAB_IDS = Object.freeze({ alerts: 'alerts' });
const TRACE_TAB_IDS = Object.freeze({ alerts: 'alerts' });
const NOTIFICATION_DURATION_STORAGE_KEY = 'actrail.notifications.duration-ms';
const DEFAULT_NOTIFICATION_DURATION_MS = 8000;
const SIDEBAR_COLLAPSED_STORAGE_KEY = 'actrail.sidebar.collapsed';
const THEME_SWATCHES = Object.freeze({
  system: ['#ffffff', '#1d1b18', '#8b8680'],
  light: ['#f0eee8', '#faf9f5', '#8b8680'],
  white: ['#ffffff', '#f6f6f6', '#2d2a26'],
  dark: ['#1d1b18', '#2a2723', '#f6f4f1'],
});

const languageOptions = LANGUAGES;

const activeWorkspace = ref(WORKSPACE_IDS.dashboard);
const selectedTheme = ref(DEFAULT_THEME_MODE);
const selectedLanguage = ref(DEFAULT_LANGUAGE_ID);
const sidebarCollapsed = ref(readSidebarCollapsed());
const sidebarAnimating = ref(false);
const sidebarFrozenWidth = ref('auto');
const sidebarRef = ref(null);
const contentRef = ref(null);
let sidebarAnimationTimer = null;
const traces = ref([]);
const query = ref('');
const error = ref('');
const refreshing = ref(false);
const workspaceLoading = ref(false);
const refreshNonce = ref(0);
const traceTitle = ref('');
const pendingTraceSelection = ref(null);
const pendingStatsSelection = ref(null);
const lastNotifiedAlertId = ref(0);
const alertBaselineEstablished = ref(false);
const notifications = ref([]);
const { t } = provideLocale(selectedLanguage);
const notificationTimers = new Map();
const notificationDurationMs = readNotificationDurationMs();
let nextNotificationId = 1;

const navGroups = computed(() =>
  NAV_GROUP_IDS_ORDER.map((groupId) => ({
    id: groupId,
    label: t(`app.navGroups.${groupId}`),
    items: NAV_GROUP_MEMBERS[groupId].map((workspaceId) => ({
      id: workspaceId,
      label: t(`app.workspaces.${workspaceId}`),
      icon: WORKSPACE_ICONS[workspaceId],
    })),
  })),
);

const activeTitle = computed(() => {
  if (activeWorkspace.value === WORKSPACE_IDS.dashboard) {
    return t('app.titles.dashboard');
  }
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
const themeOptions = computed(() =>
  THEME_MODES.map((mode) => ({
    id: mode.id,
    label: t(`app.theme.${mode.id}`),
    swatch: THEME_SWATCHES[mode.id],
  })),
);

watch(
  selectedTheme,
  (mode) => {
    void applyTheme(mode);
  },
  { immediate: true },
);

watch(sidebarCollapsed, (collapsed) => {
  window.localStorage.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, collapsed ? '1' : '0');
});

onMounted(refresh);

onBeforeUnmount(() => {
  window.clearTimeout(sidebarAnimationTimer);
  for (const timer of notificationTimers.values()) {
    window.clearTimeout(timer);
  }
  notificationTimers.clear();
});

async function applyTheme(mode) {
  try {
    await applyThemeMode(mode);
  } catch (err) {
    selectedTheme.value = DEFAULT_THEME_MODE;
    error.value = String(err.message ?? err);
  }
}

function toggleSidebar() {
  const element = sidebarRef.value;
  if (element) {
    sidebarFrozenWidth.value = `${Math.round(element.getBoundingClientRect().width)}px`;
  }
  sidebarAnimating.value = true;
  sidebarCollapsed.value = !sidebarCollapsed.value;
  window.clearTimeout(sidebarAnimationTimer);
  sidebarAnimationTimer = window.setTimeout(() => {
    sidebarAnimating.value = false;
    sidebarFrozenWidth.value = 'auto';
  }, 340);
}

function focusContent() {
  contentRef.value?.focus?.();
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
    focus: target.focus ?? null,
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

function readSidebarCollapsed() {
  return window.localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY) === '1';
}

</script>

<style scoped>
.skip-link {
  position: absolute;
  top: var(--ui-space-sm);
  left: var(--ui-shell-gutter);
  z-index: 60;
  padding: var(--ui-space-xs) var(--ui-space-md);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-raised);
  box-shadow: var(--ui-shadow);
  color: var(--ui-text);
  font-size: var(--ui-font-sm);
  text-decoration: none;
  transform: translateY(-200%);
  transition: transform var(--ui-duration-fast) var(--ui-ease-out);
}

.skip-link:focus-visible {
  transform: translateY(0);
}

.app-content:focus {
  outline: none;
}

.notification-stack {
  position: fixed;
  top: calc(var(--ui-header-offset) + var(--ui-space-sm));
  right: var(--ui-space-xl);
  z-index: 80;
  width: min(26rem, calc(100vw - 2 * var(--ui-space-xl)));
  display: grid;
  gap: var(--ui-space-md);
  pointer-events: none;
}

.app-notification {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto auto;
  align-items: center;
  gap: var(--ui-space-md);
  padding: var(--ui-space-lg);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-lg);
  background: var(--ui-floating-surface);
  box-shadow:
    var(--ui-shadow-lg),
    var(--ui-shadow);
  color: var(--ui-text);
  pointer-events: auto;
}

@starting-style {
  .app-notification {
    opacity: 0;
    transform: translateY(-8px) scale(0.98);
  }
}

.app-notification {
  transition:
    opacity var(--ui-dur-hover) var(--ui-ease-out-strong),
    transform var(--ui-dur-hover) var(--ui-ease-out-strong);
}

.app-notification-indicator {
  width: var(--ui-space-sm);
  height: var(--ui-space-sm);
  border-radius: 50%;
  background: var(--ui-danger);
  box-shadow: 0 0 0 var(--ui-space-xs) color-mix(in srgb, var(--ui-danger) 18%, transparent);
}

.app-notification-copy {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-2xs);
}

.app-notification-copy strong {
  font-size: var(--ui-font-ui);
  font-weight: var(--ui-weight-medium);
}

.app-notification-copy span {
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.app-notification-action,
.app-notification-dismiss {
  border: 0;
  background: transparent;
  color: var(--ui-accent);
  cursor: pointer;
  font: inherit;
  font-weight: var(--ui-weight-medium);
}

.app-notification-dismiss {
  color: var(--ui-muted);
  font-size: var(--ui-font-lg);
}

.app-notification-action:focus-visible,
.app-notification-dismiss:focus-visible {
  outline: 2px solid var(--ui-accent);
  outline-offset: var(--ui-space-xs);
}

@media (max-width: 47.5rem) {
  .notification-stack {
    right: var(--ui-space-lg);
    width: calc(100vw - 2 * var(--ui-space-lg));
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
