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
          <p>{{ t('dashboard.windowNote', { days: rangeDays, models: modelCount }) }}</p>
          <div class="dash-legend">
            <span><i class="legend-completed" />{{ t('dashboard.completed') }} {{ requestCount }}</span>
            <span><i class="legend-alerts" />{{ t('dashboard.alerts') }} {{ alertCount }}</span>
          </div>
        </article>
      </section>

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
          <div class="dash-range" role="group" :aria-label="t('dashboard.trafficRangeAria')">
            <button
              v-for="range in TRAFFIC_RANGES"
              :key="range.id"
              type="button"
              :class="{ active: range.id === trafficRangeId }"
              :aria-pressed="range.id === trafficRangeId"
              @click="trafficRangeId = range.id"
            >
              {{ t(`dashboard.trafficBuckets.${range.id}`) }}
            </button>
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
          :title="t('dashboard.noTraffic')"
          :description="t('dashboard.noTrafficNote')"
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
          <EmptyState v-else compact :title="t('dashboard.noModels')" />
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
  listTraces,
  readConsoleHealth,
  readDaemonStatus,
  readLlmRequestsActivity,
} from '../api';
import AnimatedNumber from '../components/AnimatedNumber.vue';
import EmptyState from '../components/EmptyState.vue';
import PulseCurve from '../components/PulseCurve.vue';
import { useLocale } from '../locale';

const props = defineProps({
  refreshNonce: { type: Number, default: 0 },
});

const emit = defineEmits(['open-workspace', 'loading']);

const { t } = useLocale();
const RANGE_DAYS = 7;
const ALERT_LIMIT = 8;
/**
 * Live-traffic ranges pair a bucket width with how far back the section
 * looks, so each choice keeps a readable number of buckets on the axis.
 */
const TRAFFIC_RANGES = Object.freeze([
  { id: '10m', rollup: 'minute', bucketMs: 10 * 60_000, buckets: 18 },
  { id: '1h', rollup: 'hour', bucketMs: 60 * 60_000, buckets: 24 },
  { id: '1d', rollup: 'day', bucketMs: 24 * 60 * 60_000, buckets: 7 },
]);
const TRAFFIC_RANGE_STORAGE_KEY = 'actrail.dashboard.traffic-range';
/** Status probes are cheap local calls, so the indicators follow the services. */
const STATUS_POLL_MS = 10_000;

const traces = ref([]);
const alerts = ref([]);
const activity = ref(null);
const trafficActivity = ref(null);
const trafficRangeId = ref(readTrafficRangeId());
const trafficWindow = ref({ from: 0, to: 0 });
const daemon = ref(null);
const consoleResponding = ref(null);
const loading = ref(false);
const error = ref('');
let controller = null;
let trafficController = null;
let statusTimer = null;

const rangeDays = RANGE_DAYS;
const requestCount = computed(() => Number(activity.value?.summary?.completed_requests ?? 0));
const tokenTotal = computed(() => Number(activity.value?.summary?.total_tokens ?? 0));
const modelCount = computed(() => Number(activity.value?.summary?.model_count ?? 0));
const alertCount = computed(() => alerts.value.length);
const urgentAlerts = computed(() =>
  alerts.value.filter((alert) => ['critical', 'high'].includes(String(alert.severity ?? '').toLowerCase())),
);
const trafficRange = computed(
  () => TRAFFIC_RANGES.find((range) => range.id === trafficRangeId.value) ?? TRAFFIC_RANGES[0],
);
const trafficBucketLabel = computed(() => t(`dashboard.trafficBuckets.${trafficRange.value.id}`));

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
    days: RANGE_DAYS,
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
    note: t('dashboard.kpi.requestsNote', { models: modelCount.value }),
  },
  {
    key: 'tokens',
    label: t('dashboard.kpi.tokens'),
    value: tokenTotal.value,
    format: formatCompact,
    note: t('dashboard.kpi.tokensNote'),
  },
  {
    key: 'alerts',
    label: t('dashboard.kpi.alerts'),
    value: alertCount.value,
    format: null,
    note: t('dashboard.kpi.alertsNote', { urgent: urgentAlerts.value.length }),
  },
]);

/**
 * Token totals per bucket, summed across model series. A 10-minute range asks
 * the backend for minute buckets and folds them, since that is its finest step.
 */
const tokenBucketTotals = computed(() => {
  const span = trafficRange.value.bucketMs;
  const totals = new Map();
  for (const row of trafficActivity.value?.trends?.models ?? []) {
    for (const point of row.points ?? []) {
      const start = Number(point.bucket_start_ms);
      if (!Number.isFinite(start)) {
        continue;
      }
      const bucket = Math.floor(start / span) * span;
      totals.set(bucket, (totals.get(bucket) ?? 0) + Number(point.value ?? 0));
    }
  }
  return totals;
});

/** Bucket starts across the whole window, empty buckets included. */
const bucketStarts = computed(() => {
  const span = trafficRange.value.bucketMs;
  const { from, to } = trafficWindow.value;
  if (!span || to <= from) {
    return [];
  }
  const starts = [];
  for (let start = Math.floor(from / span) * span; start <= to; start += span) {
    starts.push(start);
  }
  return starts;
});

const bucketLabels = computed(() =>
  bucketStarts.value.map((start) => formatBucketLabel(start, trafficRange.value.rollup)),
);

/** Task starts per bucket, so the task curve counts what the trace rail lists. */
const taskBucketTotals = computed(() => {
  const span = trafficRange.value.bucketMs;
  const { from, to } = trafficWindow.value;
  const counts = new Map();
  for (const trace of traces.value) {
    const startMs = traceStartMs(trace);
    if (startMs == null || startMs < from || startMs > to) {
      continue;
    }
    const bucket = Math.floor(startMs / span) * span;
    counts.set(bucket, (counts.get(bucket) ?? 0) + 1);
  }
  return counts;
});

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

watch(() => props.refreshNonce, load);
watch(loading, (value) => emit('loading', value));
watch(trafficRangeId, (value) => {
  window.localStorage.setItem(TRAFFIC_RANGE_STORAGE_KEY, value);
  loadTrafficActivity();
});

onMounted(() => {
  load();
  statusTimer = window.setInterval(refreshStatus, STATUS_POLL_MS);
});
onBeforeUnmount(() => {
  controller?.abort();
  controller = null;
  trafficController?.abort();
  trafficController = null;
  if (statusTimer !== null) {
    window.clearInterval(statusTimer);
    statusTimer = null;
  }
  emit('loading', false);
});

async function load() {
  controller?.abort();
  controller = new AbortController();
  loading.value = true;
  emit('loading', true);
  error.value = '';
  refreshStatus();
  try {
    const range = windowRange();
    const [traceData, alertData, activityData] = await Promise.all([
      listTraces(),
      listAlerts(ALERT_LIMIT),
      readLlmRequestsActivity({ ...range, rollup: 'hour', signal: controller.signal }),
    ]);
    traces.value = traceData.traces ?? [];
    alerts.value = alertData.alerts ?? [];
    activity.value = activityData;
    await loadTrafficActivity();
  } catch (err) {
    if (err?.name !== 'AbortError') {
      error.value = String(err.message ?? err);
    }
  } finally {
    loading.value = false;
    emit('loading', false);
  }
}

/** The live-traffic window is far shorter than the KPI window, so it queries itself. */
async function loadTrafficActivity() {
  trafficController?.abort();
  trafficController = new AbortController();
  const { bucketMs, buckets, rollup } = trafficRange.value;
  const to = Date.now();
  const from = to - buckets * bucketMs;
  trafficWindow.value = { from, to };
  try {
    trafficActivity.value = await readLlmRequestsActivity({
      fromMs: from,
      toMs: to,
      rollup,
      signal: trafficController.signal,
    });
  } catch (err) {
    if (err?.name !== 'AbortError') {
      trafficActivity.value = null;
    }
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

function windowRange() {
  const to = Date.now();
  return { fromMs: to - RANGE_DAYS * 86_400_000, toMs: to };
}

/** The chosen bucket width survives a workspace switch, like the shell prefs. */
function readTrafficRangeId() {
  const stored = window.localStorage.getItem(TRAFFIC_RANGE_STORAGE_KEY);
  return TRAFFIC_RANGES.some((range) => range.id === stored) ? stored : TRAFFIC_RANGES[0].id;
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

<style scoped>
.dashboard-workspace {
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: auto;
  scrollbar-gutter: stable;
  background: var(--ui-bg-base);
}

.dashboard-content {
  width: min(100%, var(--ui-shell-max-width));
  margin: 0 auto;
  padding: var(--ui-viewport-padding);
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-2xl);
}

.dash-hero {
  display: grid;
  grid-template-columns: minmax(0, 1.15fr) minmax(300px, 0.85fr);
  align-items: center;
  gap: var(--ui-space-3xl);
}

.dash-eyebrow {
  color: var(--ui-muted);
  font-size: 11px;
  font-weight: var(--ui-weight-semibold);
  letter-spacing: 0.1em;
  text-transform: uppercase;
}

.dash-status {
  --dash-status-glyph: 8px;

  display: grid;
  gap: var(--ui-space-md);
  margin: var(--ui-space-lg) 0 var(--ui-space-md);
  padding: 0;
  list-style: none;
}

.dash-status-item {
  --dash-status-color: var(--ui-muted);

  display: flex;
  align-items: center;
  gap: var(--ui-space-sm);
  min-width: 0;
}

.dash-status-item.is-running {
  --dash-status-color: var(--ui-viz-success);
}

.dash-status-item.is-stopped {
  --dash-status-color: var(--ui-viz-failure);
}

/* The state reads as a tinted pill around the lamp: same hue as the lamp, a
   shade off so the state word stays legible next to it. */
.dash-status-chip {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 24px;
  padding: 0 10px 0 8px;
  border: 1px solid color-mix(in srgb, var(--dash-status-color) 34%, transparent);
  border-radius: var(--ui-radius-pill);
  background: color-mix(in srgb, var(--dash-status-color) 14%, transparent);
  color: var(--dash-status-color);
  font-size: 11px;
  font-weight: var(--ui-weight-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
}

.dash-status-glyph {
  flex: 0 0 auto;
  width: var(--dash-status-glyph);
  height: var(--dash-status-glyph);
  border-radius: 50%;
  background: var(--dash-status-color);
}

.dash-status-item.is-running .dash-status-glyph {
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--dash-status-color) 24%, transparent);
  animation: dash-status-pulse 2s ease-out infinite;
}

.dash-status-item.is-idle .dash-status-glyph {
  background: color-mix(in srgb, var(--ui-muted) 55%, transparent);
}

.dash-status-item.is-unknown .dash-status-glyph {
  background: transparent;
  border: 2px dashed color-mix(in srgb, var(--ui-muted) 70%, transparent);
}

.dash-status-name {
  min-width: 0;
  font-family: var(--ui-value-font);
  font-size: clamp(15px, 1.4vw, 18px);
  font-weight: var(--ui-weight-medium);
  letter-spacing: 0.04em;
  line-height: 1.4;
  text-transform: uppercase;
}

.dash-status-item.is-unknown .dash-status-name {
  color: var(--ui-muted);
}

/* The process name rides on the same line: stacked, a short token under a
   sentence reads as a 4x width jump. */
.dash-status-detail {
  min-width: 0;
  overflow: hidden;
  color: var(--ui-muted);
  font-size: var(--ui-font-md);
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@keyframes dash-status-pulse {
  0% {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--ui-viz-success) 30%, transparent);
  }

  70%,
  100% {
    box-shadow: 0 0 0 6px transparent;
  }
}

@media (prefers-reduced-motion: reduce) {
  .dash-status-item.is-running .dash-status-glyph {
    animation: none;
  }
}

.dash-meta {
  margin: 0 0 var(--ui-space-lg);
  color: var(--ui-muted);
  font-size: var(--ui-font-ui);
}

.dash-actions {
  display: flex;
  align-items: center;
  gap: var(--ui-space-lg);
}

.dash-primary {
  height: 40px;
  padding: 0 18px;
  border: 0;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-text);
  color: var(--ui-bg-base);
  cursor: pointer;
  font-size: var(--ui-font-md);
  font-weight: var(--ui-weight-semibold);
  transition: box-shadow var(--ui-duration-fast) var(--ui-ease-out);
}

.dash-primary:hover {
  box-shadow: 0 12px 26px color-mix(in srgb, var(--ui-text) 22%, transparent);
}

.dash-link {
  border: 0;
  background: transparent;
  color: var(--ui-text);
  cursor: pointer;
  font-size: var(--ui-font-md);
  font-weight: var(--ui-weight-medium);
}

.dash-link:hover {
  color: var(--ui-accent-strong);
}

.dash-hero-card {
  display: grid;
  gap: var(--ui-space-xs);
  padding: var(--ui-space-2xl);
  border: 1px solid color-mix(in srgb, var(--ui-border) 82%, transparent);
  border-radius: var(--ui-radius-xl);
  background: color-mix(in srgb, var(--ui-bg-shell) 82%, transparent);
  box-shadow: var(--ui-shadow-lg);
}

.dash-hero-card header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  color: var(--ui-muted);
  font-size: 11px;
  font-weight: var(--ui-weight-semibold);
  letter-spacing: 0.09em;
  text-transform: uppercase;
}

.dash-hero-card strong {
  font-family: var(--ui-value-font);
  font-size: clamp(34px, 3.4vw, 48px);
  font-weight: var(--ui-weight-medium);
  font-variant-numeric: tabular-nums;
  line-height: 1;
}

.dash-hero-card p {
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.dash-legend {
  display: flex;
  gap: var(--ui-space-lg);
  margin-top: var(--ui-space-xs);
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.dash-legend span {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.dash-legend i {
  width: 8px;
  height: 8px;
  border-radius: 2px;
}

.legend-completed {
  background: var(--ui-viz-success);
}

.legend-alerts {
  background: var(--ui-viz-failure);
}

.dash-rule {
  height: 1px;
  margin: 0;
  border: 0;
  background: linear-gradient(
    90deg,
    color-mix(in srgb, var(--ui-border) 90%, transparent) 0 46%,
    transparent 46% 54%,
    color-mix(in srgb, var(--ui-border) 90%, transparent) 54% 100%
  );
}

.dash-kpis {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: var(--ui-space-lg);
}

.dash-kpi {
  min-width: 0;
  display: grid;
  gap: 6px;
  padding: var(--ui-space-xl) var(--ui-space-2xl);
  border: 1px solid color-mix(in srgb, var(--ui-border) 82%, transparent);
  border-radius: var(--ui-radius-xl);
  background: color-mix(in srgb, var(--ui-bg-shell) 82%, transparent);
}

.dash-kpi-rule {
  width: 26px;
  height: 2px;
  border-radius: 2px;
  background: color-mix(in srgb, var(--ui-text) 24%, transparent);
}

.dash-kpi-label {
  color: var(--ui-muted);
  font-size: var(--ui-font-md);
  font-weight: var(--ui-weight-medium);
}

.dash-kpi strong {
  font-family: var(--ui-value-font);
  font-size: 30px;
  font-weight: var(--ui-weight-medium);
  font-variant-numeric: tabular-nums;
  line-height: 1.1;
}

.dash-kpi small {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.dash-section {
  display: grid;
  gap: 4px;
}

.dash-section h2 {
  margin: 4px 0 0;
  font-size: var(--ui-font-display-md);
  font-weight: var(--ui-weight-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
}

.dash-section p {
  margin: 0 0 var(--ui-space-md);
  color: var(--ui-muted);
  font-size: var(--ui-font-md);
  line-height: 1.5;
}

.dash-charts {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--ui-space-lg);
  margin-top: var(--ui-space-lg);
}

.dash-traffic-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--ui-space-lg);
}

.dash-traffic-copy {
  min-width: 0;
  display: grid;
  gap: 4px;
}

.dash-range {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  border: 1px solid color-mix(in srgb, var(--ui-border) 82%, transparent);
  border-radius: var(--ui-radius-pill);
  background: color-mix(in srgb, var(--ui-bg-shell) 82%, transparent);
}

.dash-range button {
  height: 24px;
  padding: 0 10px;
  border: 0;
  border-radius: var(--ui-radius-pill);
  background: transparent;
  color: var(--ui-muted);
  font-size: 11px;
  font-weight: var(--ui-weight-semibold);
  cursor: pointer;
  transition:
    background-color var(--ui-duration-fast) var(--ui-ease-out),
    color var(--ui-duration-fast) var(--ui-ease-out);
}

.dash-range button:hover {
  color: var(--ui-text);
}

.dash-range button.active {
  background: color-mix(in srgb, var(--ui-text) 10%, transparent);
  color: var(--ui-text);
}

.dash-chart-card {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-md);
  padding: var(--ui-space-2xl);
  border: 1px solid color-mix(in srgb, var(--ui-border) 82%, transparent);
  border-radius: var(--ui-radius-xl);
  background: color-mix(in srgb, var(--ui-bg-shell) 82%, transparent);
}

.dash-chart-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--ui-space-sm);
}

.dash-chart-head > span {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-semibold);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.dash-chart-head strong {
  font-family: var(--ui-value-font);
  font-size: 20px;
  font-weight: var(--ui-weight-medium);
  font-variant-numeric: tabular-nums;
}

@media (max-width: 900px) {
  .dash-charts {
    grid-template-columns: minmax(0, 1fr);
  }
}

.dash-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: var(--ui-space-lg);
}

.dash-panel {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-md);
  padding: var(--ui-space-xl) var(--ui-space-2xl);
  border: 1px solid color-mix(in srgb, var(--ui-border) 82%, transparent);
  border-radius: var(--ui-radius-xl);
  background: color-mix(in srgb, var(--ui-bg-shell) 82%, transparent);
}

.dash-panel header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--ui-space-md);
}

.dash-panel h3 {
  margin: 0;
  font-size: var(--ui-font-title);
  font-weight: var(--ui-weight-semibold);
}

.dash-panel small {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
  text-align: right;
}

.dash-rows {
  display: grid;
  gap: 2px;
  margin: 0;
  padding: 0;
  list-style: none;
}

.dash-rows li {
  display: grid;
  grid-template-columns: minmax(0, 1.4fr) minmax(60px, 0.9fr) 68px 42px;
  align-items: center;
  gap: var(--ui-space-sm);
  padding: 7px 2px;
  border-bottom: 1px solid color-mix(in srgb, var(--ui-border) 70%, transparent);
}

.dash-rows li:last-child {
  border-bottom: 0;
}

.dash-rows-alerts li {
  grid-template-columns: 78px minmax(0, 1fr) 56px;
}

.dash-row-name {
  min-width: 0;
  overflow: hidden;
  font-size: var(--ui-font-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dash-meter {
  height: 6px;
  overflow: hidden;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-surface-soft);
}

.dash-meter i {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: var(--ui-chart-3);
}

.dash-row-value,
.dash-row-share,
.dash-row-time {
  color: var(--ui-muted);
  font-family: var(--ui-value-font);
  font-size: var(--ui-font-xs);
  font-variant-numeric: tabular-nums;
  text-align: right;
}

.dash-severity {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 18px;
  padding: 0 8px;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-surface-soft);
  color: var(--ui-muted);
  font-size: 10px;
  font-weight: var(--ui-weight-semibold);
  letter-spacing: 0.03em;
  text-transform: uppercase;
}

.severity-critical,
.severity-high {
  background: var(--ui-danger-bg);
  color: var(--ui-danger);
}

.severity-medium,
.severity-warning {
  background: var(--ui-warning-bg);
  color: var(--ui-warning);
}

@media (max-width: 1100px) {
  .dash-hero,
  .dash-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .dash-kpis {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 760px) {
  .dashboard-content {
    padding: var(--ui-viewport-padding-mobile);
  }

  .dash-kpis {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
