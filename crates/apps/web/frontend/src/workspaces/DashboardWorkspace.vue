<template>
  <main class="dashboard-workspace">
    <div class="dashboard-content">
      <section v-reveal class="dash-hero">
        <div class="dash-hero-main">
          <span class="dash-eyebrow">{{ t('dashboard.eyebrow') }}</span>
          <ul class="dash-status" :aria-label="t('dashboard.status.aria')">
            <li
              v-for="item in statusItems"
              :key="item.key"
              class="dash-status-item"
              :class="`is-${item.tone}`"
              :title="item.hint"
              :aria-label="item.aria"
            >
              <span class="dash-status-chip">
                <span class="dash-status-glyph" aria-hidden="true" />
                <span class="dash-status-state">{{ item.state }}</span>
              </span>
              <strong class="dash-status-name">{{ item.value }}</strong>
              <small class="dash-status-detail">{{ item.trailing }}</small>
            </li>
          </ul>
          <p class="dash-meta">{{ metaLine }}</p>
          <div class="dash-actions">
            <button class="dash-primary" type="button" @click="$emit('open-workspace', 'traces')">
              {{ t('dashboard.browseTraces') }}
            </button>
            <button class="dash-link" type="button" @click="$emit('open-workspace', 'stats')">
              {{ t('dashboard.inspectStats') }} →
            </button>
          </div>
        </div>

        <article class="dash-hero-card">
          <header>
            <span>{{ t('dashboard.requests') }}</span>
          </header>
          <strong><AnimatedNumber :value="requestCount" /></strong>
          <p>{{ activity ? t('dashboard.windowNote', { models: modelCount }) : t('dashboard.activityNotLoaded') }}</p>
          <div class="dash-legend">
            <span><i class="legend-completed" />{{ t('dashboard.completed') }} {{ requestCount }}</span>
            <span><i class="legend-alerts" />{{ t('dashboard.alerts') }} {{ alertCount }}</span>
          </div>
        </article>
      </section>

      <ActivityControls :activity="activityState" />
      <hr class="dash-rule" />

      <section class="dash-kpis">
        <article v-for="(kpi, index) in kpis" :key="kpi.key" v-reveal="index" class="dash-kpi">
          <span class="dash-kpi-rule" aria-hidden="true" />
          <span class="dash-kpi-label">{{ kpi.label }}</span>
          <strong><AnimatedNumber :value="kpi.value" :format="kpi.format" /></strong>
          <small>{{ kpi.note }}</small>
        </article>
      </section>

      <section v-reveal class="dash-section">
        <div class="dash-traffic-head">
          <div class="dash-traffic-copy">
            <span class="dash-eyebrow">{{ t('dashboard.trafficEyebrow') }}</span>
            <h2>{{ t('dashboard.trafficTitle') }}</h2>
            <p>{{ t('dashboard.trafficNote', { rollup: trafficBucketLabel }) }}</p>
          </div>
        </div>
        <div v-if="hasActivity" class="dash-charts">
          <article class="dash-chart-card">
            <header class="dash-chart-head">
              <span>{{ t('dashboard.trafficTasks') }}</span>
              <strong>{{ formatNumber(taskCurveTotal) }}</strong>
            </header>
            <PulseCurve
              :points="taskCurve"
              :labels="bucketLabels"
              color="var(--ui-chart-3)"
              :aria-label="t('dashboard.trafficTasksAria', { count: taskCurveTotal })"
            />
          </article>
          <article class="dash-chart-card">
            <header class="dash-chart-head">
              <span>{{ t('dashboard.trafficTokens') }}</span>
              <strong>{{ formatNumber(tokenCurveTotal) }}</strong>
            </header>
            <PulseCurve
              :points="tokenCurve"
              :labels="bucketLabels"
              color="var(--ui-chart-2)"
              :aria-label="t('dashboard.trafficTokensAria', { tokens: tokenCurveTotal })"
            />
          </article>
        </div>
        <EmptyState
          v-else
          compact
          :title="t(activity ? 'dashboard.noTraffic' : 'dashboard.activityNotLoaded')"
          :description="activity ? t('dashboard.noTrafficNote') : ''"
        />
      </section>

      <section class="dash-grid">
        <article v-reveal class="dash-panel">
          <header>
            <h3>{{ t('dashboard.models') }}</h3>
            <small>{{ t('dashboard.modelsNote') }}</small>
          </header>
          <ul v-if="models.length" class="dash-rows">
            <li v-for="model in models" :key="model.key">
              <span class="dash-row-name" :title="model.label">{{ model.label }}</span>
              <span class="dash-meter"><i :style="{ width: `${model.share}%` }" /></span>
              <span class="dash-row-value">{{ formatNumber(model.total) }}</span>
              <span class="dash-row-share">{{ model.share.toFixed(0) }}%</span>
            </li>
          </ul>
          <EmptyState v-else compact :title="t(activity ? 'dashboard.noModels' : 'dashboard.activityNotLoaded')" />
        </article>

        <article v-reveal="1" class="dash-panel">
          <header>
            <h3>{{ t('dashboard.alerts') }}</h3>
            <small>{{ t('dashboard.alertsNote') }}</small>
          </header>
          <ul v-if="alerts.length" class="dash-rows dash-rows-alerts">
            <li v-for="alert in alerts" :key="alert.alert_id">
              <span class="dash-severity" :class="`severity-${(alert.severity ?? '').toLowerCase()}`">
                {{ alert.severity }}
              </span>
              <span class="dash-row-name" :title="alert.title">{{ alert.title }}</span>
              <span class="dash-row-time">{{ formatClock(alert.created_at) }}</span>
            </li>
          </ul>
          <EmptyState v-else compact :title="t('dashboard.noAlerts')" />
        </article>
      </section>

      <div v-if="error" class="error-bar">{{ error }}</div>
    </div>
  </main>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import {
  listAlerts,
  readConsoleHealth,
  readDaemonStatus,
} from '../api';
import ActivityControls from './dashboard/ActivityControls.vue';
import { DashboardActivity } from './dashboard/activity';
import AnimatedNumber from '../components/AnimatedNumber.vue';
import EmptyState from '../components/EmptyState.vue';
import PulseCurve from '../components/PulseCurve.vue';
import { useLocale } from '../locale';

const props = defineProps({
  traces: { type: Array, required: true },
  refreshNonce: { type: Number, default: 0 },
});

const emit = defineEmits(['open-workspace', 'loading']);

const { t } = useLocale();
const ALERT_LIMIT = 8;
/** Status probes are cheap local calls, so the indicators follow the services. */
const STATUS_POLL_MS = 10_000;

const traces = computed(() => props.traces);
const alerts = ref([]);
const activityState = new DashboardActivity();
const { data: activity, loading } = activityState;
const daemon = ref(null);
const consoleResponding = ref(null);
const error = ref('');
let metadataToken = null;
let statusTimer = null;

const requestCount = computed(() => activity.value ? Number(activity.value.summary?.completed_requests ?? 0) : '—');
const tokenTotal = computed(() => activity.value ? Number(activity.value.summary?.total_tokens ?? 0) : '—');
const modelCount = computed(() => Number(activity.value?.summary?.model_count ?? 0));
const alertCount = computed(() => alerts.value.length);
const urgentAlerts = computed(() =>
  alerts.value.filter((alert) => ['critical', 'high'].includes(String(alert.severity ?? '').toLowerCase())),
);
const trafficRollup = computed(() => activity.value?.range?.rollup ?? 'day');
const trafficBucketLabel = computed(() => t(`dashboard.trafficBuckets.${trafficRollup.value}`));

/** Unknown, stopped and running stay distinct: only the probe can prove the last one. */
const daemonState = computed(() => {
  const status = daemon.value;
  if (!status) {
    return 'unknown';
  }
  if (status.available) {
    return 'running';
  }
  return status.socket_path ? 'stopped' : 'unknown';
});

const consoleState = computed(() => {
  if (consoleResponding.value === null) {
    return 'unknown';
  }
  return consoleResponding.value ? 'running' : 'stopped';
});

/** Only the daemon itself can count what is running, and only while it answers. */
const agentCount = computed(() => {
  const status = daemon.value;
  if (!status?.available || status.active_traces == null) {
    return null;
  }
  return Number(status.active_traces);
});

const statusItems = computed(() => {
  const row = (key, service, process, state) => ({
    key,
    value: t(`dashboard.status.${service}`),
    state: t(`dashboard.status.${state}`),
    trailing: t(`dashboard.status.${process}`),
    tone: state,
    aria: t('dashboard.status.rowAria', {
      service: t(`dashboard.status.${service}`),
      process: t(`dashboard.status.${process}`),
      state: t(`dashboard.status.${state}`),
    }),
  });
  const items = [
    row('daemon', 'daemon', 'daemonProcess', daemonState.value),
    row('console', 'console', 'consoleProcess', consoleState.value),
  ];
  // Nothing tracks agents while the daemon is away, so the row only exists when
  // it answers; its count then owns the value slot.
  if (daemonState.value === 'running') {
    const count = agentCount.value;
    const agentState = count == null ? 'unknown' : (count > 0 ? 'running' : 'idle');
    items.push({
      key: 'agents',
      value: t('dashboard.status.agents'),
      state: t(`dashboard.status.${agentState}`),
      trailing: count == null ? t('dashboard.status.unknownValue') : String(count),
      tone: agentState,
      aria: t('dashboard.status.rowAria', {
        service: t('dashboard.status.agents'),
        process: String(count ?? t('dashboard.status.unknownValue')),
        state: t(`dashboard.status.${agentState}`),
      }),
      hint: count == null ? undefined : t('dashboard.status.agentsHint'),
    });
  }
  return items;
});

const metaLine = computed(() =>
  t('dashboard.meta', {
    traces: traces.value.length,
    alerts: alertCount.value,
  }),
);

const kpis = computed(() => [
  {
    key: 'traces',
    label: t('dashboard.kpi.traces'),
    value: traces.value.length,
    format: null,
    note: t('dashboard.kpi.tracesNote'),
  },
  {
    key: 'requests',
    label: t('dashboard.kpi.requests'),
    value: requestCount.value,
    format: null,
    note: activity.value ? t('dashboard.kpi.requestsNote', { models: modelCount.value }) : t('dashboard.activityNotLoaded'),
  },
  {
    key: 'tokens',
    label: t('dashboard.kpi.tokens'),
    value: tokenTotal.value,
    format: formatCompact,
    note: t(activity.value ? 'dashboard.kpi.tokensNote' : 'dashboard.activityNotLoaded'),
  },
  {
    key: 'alerts',
    label: t('dashboard.kpi.alerts'),
    value: alertCount.value,
    format: null,
    note: t('dashboard.kpi.alertsNote', { urgent: urgentAlerts.value.length }),
  },
]);

const tokenBucketTotals = computed(() => {
  const totals = new Map();
  for (const row of activity.value?.trends?.models ?? []) {
    for (const point of row.points ?? []) {
      const start = Number(point.bucket_start_ms);
      if (Number.isFinite(start)) totals.set(start, (totals.get(start) ?? 0) + Number(point.value ?? 0));
    }
  }
  return totals;
});
const taskBucketTotals = computed(() => {
  const range = activity.value?.range;
  const counts = new Map();
  if (!range) return counts;
  for (const trace of traces.value) {
    const start = traceStartMs(trace);
    if (start == null || start < range.from_ms || start >= range.to_ms) continue;
    const bucket = bucketStart(start);
    counts.set(bucket, (counts.get(bucket) ?? 0) + 1);
  }
  return counts;
});
const bucketStarts = computed(() => [...new Set([
  ...tokenBucketTotals.value.keys(), ...taskBucketTotals.value.keys(),
])].sort((a, b) => a - b));
const bucketLabels = computed(() => bucketStarts.value.map(start => formatBucketLabel(start, trafficRollup.value)));

function bucketStart(milliseconds) {
  const date = new Date(milliseconds);
  if (trafficRollup.value === 'month') return Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), 1);
  if (trafficRollup.value === 'week') {
    const days = Math.floor(milliseconds / 86_400_000);
    return Math.max(0, days - (days + 3) % 7) * 86_400_000;
  }
  const span = { minute: 60_000, hour: 3_600_000, day: 86_400_000 }[trafficRollup.value];
  return Math.floor(milliseconds / span) * span;
}

const taskCurve = computed(() =>
  bucketStarts.value.map((start) => taskBucketTotals.value.get(start) ?? 0),
);
const tokenCurve = computed(() =>
  bucketStarts.value.map((start) => tokenBucketTotals.value.get(start) ?? 0),
);
const taskCurveTotal = computed(() => taskCurve.value.reduce((sum, value) => sum + value, 0));
const tokenCurveTotal = computed(() => tokenCurve.value.reduce((sum, value) => sum + value, 0));
const hasActivity = computed(() => taskCurveTotal.value > 0 || tokenCurveTotal.value > 0);

/** Task start as epoch millis; a trace still running falls back to creation. */
function traceStartMs(trace) {
  const started = toNanos(trace?.started_at_unix_nanos);
  const created = toNanos(trace?.created_at_unix_nanos);
  const nanos = started != null && started > 0n ? started : created;
  return nanos == null ? null : Number(nanos / 1_000_000n);
}

function toNanos(value) {
  if (value == null) {
    return null;
  }
  try {
    return BigInt(value);
  } catch {
    return null;
  }
}

/** Mirrors the backend bucket labels, which are rendered in UTC. */
function formatBucketLabel(startMs, rollup) {
  const date = new Date(startMs);
  if (Number.isNaN(date.getTime())) {
    return String(startMs);
  }
  const day = `${date.getUTCFullYear()}-${pad2(date.getUTCMonth() + 1)}-${pad2(date.getUTCDate())}`;
  if (rollup === 'minute') {
    return `${day} ${pad2(date.getUTCHours())}:${pad2(date.getUTCMinutes())}`;
  }
  if (rollup === 'hour') {
    return `${day} ${pad2(date.getUTCHours())}:00`;
  }
  if (rollup === 'month') {
    return day.slice(0, 7);
  }
  return day;
}

function pad2(value) {
  return String(value).padStart(2, '0');
}

const models = computed(() => {
  const rows = activity.value?.overview?.top_models ?? [];
  const total = rows.reduce((sum, row) => sum + Number(row.total ?? 0), 0) || 1;
  return rows.slice(0, 8).map((row) => ({
    key: row.key,
    label: row.label ?? row.key,
    total: Number(row.total ?? 0),
    share: (Number(row.total ?? 0) / total) * 100,
  }));
});

watch(() => props.refreshNonce, () => { loadMetadata(); activityState.refresh(); });
watch(loading, value => emit('loading', value));
onMounted(() => {
  loadMetadata();
  statusTimer = window.setInterval(refreshStatus, STATUS_POLL_MS);
});
onBeforeUnmount(() => {
  metadataToken = null;
  activityState.cancel();
  if (statusTimer !== null) window.clearInterval(statusTimer);
  emit('loading', false);
});

async function loadMetadata() {
  const token = Symbol();
  metadataToken = token;
  error.value = '';
  refreshStatus();
  try {
    const result = await listAlerts(ALERT_LIMIT);
    if (metadataToken === token) alerts.value = result.alerts ?? [];
  } catch (failure) {
    if (metadataToken === token) error.value = String(failure.message ?? failure);
  }
}

/**
 * Probe the two services behind the indicators. A failing probe is the answer
 * itself, so the dashboard data above stays untouched.
 */
async function refreshStatus() {
  const [daemonProbe, consoleProbe] = await Promise.allSettled([
    readDaemonStatus(),
    readConsoleHealth(),
  ]);
  daemon.value = daemonProbe.status === 'fulfilled' ? daemonProbe.value : null;
  consoleResponding.value = consoleProbe.status === 'fulfilled' ? consoleProbe.value : false;
}

function formatNumber(value) {
  return Number(value ?? 0).toLocaleString();
}

function formatCompact(value) {
  const number = Number(value ?? 0);
  if (number >= 1_000_000) {
    return `${(number / 1_000_000).toFixed(number >= 10_000_000 ? 0 : 1)}M`;
  }
  if (number >= 1_000) {
    return `${(number / 1_000).toFixed(number >= 10_000 ? 0 : 1)}k`;
  }
  return String(Math.round(number));
}

function formatClock(value) {
  const millis = Number(value ?? 0);
  if (!Number.isFinite(millis) || millis <= 0) {
    return '—';
  }
  return new Date(millis).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
}
</script>

<style scoped src="./dashboard/dashboard.css"></style>
