<template>
  <section class="alerts-workspace">
    <header class="alerts-header">
      <div class="alerts-heading">
        <span>{{ t('alerts.kicker') }}</span>
        <h2>{{ t('alerts.title') }}</h2>
        <p>{{ t('alerts.subtitle') }}</p>
      </div>
      <div class="alerts-header-actions">
        <div class="alerts-total">
          <span>{{ t('alerts.total') }}</span>
          <strong>{{ alerts.length }}</strong>
        </div>
        <button class="alerts-refresh" type="button" :disabled="requestInFlight" @click="loadAlerts()">
          <RefreshCw :size="16" aria-hidden="true" />
          <span>{{ t('alerts.refresh') }}</span>
        </button>
        <details class="alerts-refresh-settings">
          <summary :aria-label="t('alerts.refreshSettings')" :title="t('alerts.refreshSettings')">
            <Settings2 :size="16" aria-hidden="true" />
          </summary>
          <label class="alerts-poll-interval">
            <span>{{ t('alerts.autoRefresh') }}</span>
            <span class="alerts-poll-input">
              <input
                v-model="pollIntervalInput"
                type="number"
                min="1"
                step="1"
                :aria-label="t('alerts.pollInterval')"
                @input="applyPollInterval"
                @blur="restorePollIntervalInput"
              />
              <small>{{ t('alerts.seconds') }}</small>
            </span>
          </label>
        </details>
      </div>
    </header>

    <div v-if="error" class="alerts-error">{{ error }}</div>

    <div class="alerts-workbench">
      <aside class="alerts-list" :aria-label="t('alerts.listAria')">
        <div class="alerts-list-heading">
          <span>{{ t('alerts.latest') }}</span>
          <div class="alerts-list-tools">
            <select v-model="selectedSeverity" :aria-label="t('alerts.severityFilter')">
              <option value="all">{{ t('alerts.allSeverities') }}</option>
              <option v-for="severity in severityOptions" :key="severity" :value="severity">
                {{ severity }}
              </option>
            </select>
            <strong>{{ filteredAlerts.length }}</strong>
          </div>
        </div>
        <TransitionGroup name="alert-list" tag="div" class="alerts-list-items">
          <button
            v-for="alert in filteredAlerts"
            :key="alert.alert_id"
            class="alert-row"
            :class="{ active: selectedAlertId === alert.alert_id }"
            type="button"
            @click="selectedAlertId = alert.alert_id"
          >
            <span class="alert-row-heading">
              <strong>{{ alert.title }}</strong>
              <span class="severity" :class="`severity-${alert.severity}`">{{ alert.severity }}</span>
            </span>
            <span class="alert-row-kind">{{ alert.kind }}</span>
            <span class="alert-row-footer">
              <span>trace-{{ alert.trace_id }}</span>
              <time>{{ formatTime(alert.created_at) }}</time>
            </span>
          </button>
        </TransitionGroup>
        <p v-if="!filteredAlerts.length && !loading" class="alerts-empty">{{ emptyMessage }}</p>
      </aside>

      <main class="alert-detail">
        <div v-if="loading" class="alerts-empty">{{ t('alerts.loading') }}</div>
        <template v-else-if="selectedAlert">
          <header class="alert-detail-header">
            <div>
              <span class="severity" :class="`severity-${selectedAlert.severity}`">
                {{ selectedAlert.severity }}
              </span>
              <h2>{{ selectedAlert.title }}</h2>
              <p>{{ selectedAlert.kind }} · {{ selectedAlert.producer_plugin_id }}</p>
            </div>
            <button class="open-trace" type="button" @click="openTrace(selectedAlert.trace_id)">
              {{ t('alerts.openTrace') }}
            </button>
          </header>

          <dl class="alert-fields">
            <div><dt>{{ t('alerts.alertId') }}</dt><dd>{{ selectedAlert.alert_id }}</dd></div>
            <div><dt>{{ t('alerts.traceId') }}</dt><dd>trace-{{ selectedAlert.trace_id }}</dd></div>
            <div><dt>{{ t('alerts.createdAt') }}</dt><dd>{{ formatTime(selectedAlert.created_at) }}</dd></div>
            <div><dt>{{ t('alerts.definition') }}</dt><dd>{{ selectedAlert.definition_key }}</dd></div>
          </dl>

          <section v-if="residualFiles.length" class="alert-payload-panel residual-files">
            <h3>{{ t('alerts.residualFiles') }}</h3>
            <ul>
              <li v-for="path in residualFiles" :key="path"><code>{{ path }}</code></li>
            </ul>
          </section>
          <section v-else class="alert-payload-panel generic-payload">
            <h3>{{ t('alerts.payload') }}</h3>
            <JsonTree :value="selectedAlert.payload" />
          </section>
        </template>
        <p v-else class="alerts-empty">{{ t('alerts.select') }}</p>
      </main>
    </div>
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { RefreshCw, Settings2 } from '@lucide/vue';

import { listAlerts, readAlert } from '../api';
import JsonTree from '../components/JsonTree.vue';
import { useLocale } from '../locale';

const props = defineProps({
  query: {
    type: String,
    default: '',
  },
  refreshNonce: {
    type: Number,
    default: 0,
  },
  activationNonce: {
    type: Number,
    default: 0,
  },
  notifiedAlertId: {
    type: Number,
    default: 0,
  },
});

const emit = defineEmits(['alerts-loaded', 'loading', 'open-trace']);
const { t } = useLocale();
const POLL_INTERVAL_STORAGE_KEY = 'actrail.alerts.poll-interval-seconds';
const DEFAULT_POLL_INTERVAL_SECONDS = 1;
const initialPollIntervalSeconds = readPollIntervalSeconds();
const alerts = ref([]);
const selectedAlertId = ref(null);
const selectedAlert = ref(null);
const loading = ref(false);
const requestInFlight = ref(false);
const error = ref('');
const pollIntervalSeconds = ref(initialPollIntervalSeconds);
const pollIntervalInput = ref(String(initialPollIntervalSeconds));
const selectedSeverity = ref('all');
let loadToken = null;
let detailToken = null;
let pollTimer = null;

const filteredAlerts = computed(() => {
  const needle = props.query.trim().toLowerCase();
  return alerts.value.filter((alert) => {
    if (selectedSeverity.value !== 'all' && alert.severity !== selectedSeverity.value) return false;
    if (!needle) return true;
    return [alert.title, alert.kind, alert.severity, alert.trace_id, JSON.stringify(alert.payload)]
      .join(' ')
      .toLowerCase()
      .includes(needle);
  });
});
const severityOptions = computed(() => [...new Set(alerts.value.map((alert) => alert.severity))].sort());
const emptyMessage = computed(() => (
  props.query.trim() || selectedSeverity.value !== 'all'
    ? t('alerts.noResults')
    : t('alerts.empty')
));
const residualFiles = computed(() =>
  Array.isArray(selectedAlert.value?.payload?.residual_files)
    ? selectedAlert.value.payload.residual_files
    : [],
);

watch(
  () => [props.refreshNonce, props.activationNonce],
  () => loadAlerts(),
  { immediate: true },
);

watch(selectedAlertId, (alertId) => loadDetail(alertId));

onMounted(startPolling);

onBeforeUnmount(() => {
  stopPolling();
  emit('loading', false);
});

async function loadAlerts(background = false) {
  if (requestInFlight.value) {
    return;
  }
  const token = Symbol();
  loadToken = token;
  requestInFlight.value = true;
  if (!background) {
    loading.value = true;
    emit('loading', true);
  }
  error.value = '';
  try {
    const data = await listAlerts();
    if (loadToken !== token) {
      return;
    }
    alerts.value = data.alerts ?? [];
    const latestAlertId = alerts.value.reduce(
      (latest, alert) => Math.max(latest, Number(alert.alert_id) || 0),
      0,
    );
    const newCount = alerts.value.filter(
      (alert) => (Number(alert.alert_id) || 0) > props.notifiedAlertId,
    ).length;
    emit('alerts-loaded', { latestAlertId, newCount });
    if (!alerts.value.some((alert) => alert.alert_id === selectedAlertId.value)) {
      selectedAlertId.value = alerts.value[0]?.alert_id ?? null;
    }
  } catch (err) {
    if (loadToken === token) {
      error.value = String(err.message ?? err);
    }
  } finally {
    if (loadToken === token) {
      requestInFlight.value = false;
      if (!background) {
        loading.value = false;
        emit('loading', false);
      }
    }
  }
}

function startPolling() {
  stopPolling();
  pollTimer = window.setInterval(() => loadAlerts(true), pollIntervalSeconds.value * 1000);
}

function stopPolling() {
  if (pollTimer !== null) {
    window.clearInterval(pollTimer);
    pollTimer = null;
  }
}

function applyPollInterval(event) {
  const value = Number(event.currentTarget.value);
  if (!Number.isInteger(value) || value < 1) {
    return;
  }
  pollIntervalSeconds.value = value;
  window.localStorage.setItem(POLL_INTERVAL_STORAGE_KEY, String(value));
  startPolling();
}

function restorePollIntervalInput() {
  const value = Number(pollIntervalInput.value);
  if (!Number.isInteger(value) || value < 1) {
    pollIntervalInput.value = String(pollIntervalSeconds.value);
  }
}

function readPollIntervalSeconds() {
  const storedValue = window.localStorage.getItem(POLL_INTERVAL_STORAGE_KEY);
  if (storedValue === null) {
    return DEFAULT_POLL_INTERVAL_SECONDS;
  }
  const value = Number(storedValue);
  if (!Number.isInteger(value) || value < 1) {
    throw new Error(`${POLL_INTERVAL_STORAGE_KEY} must be a positive integer`);
  }
  return value;
}

async function loadDetail(alertId) {
  if (alertId == null) {
    selectedAlert.value = null;
    return;
  }
  const token = Symbol();
  detailToken = token;
  try {
    const data = await readAlert(alertId);
    if (detailToken === token && selectedAlertId.value === alertId) {
      selectedAlert.value = data.alert ?? null;
    }
  } catch (err) {
    if (detailToken === token) {
      error.value = String(err.message ?? err);
    }
  }
}

function openTrace(traceId) {
  emit('open-trace', { traceId });
}

function formatTime(timestamp) {
  const value = Number(timestamp);
  return Number.isFinite(value) ? new Date(value).toLocaleString() : String(timestamp ?? '');
}
</script>

<style scoped src="./stats/alerts.css"></style>
<style scoped src="./stats/alert-live.css"></style>
