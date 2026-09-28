<template>
  <section class="agent-stats-workspace">
    <header class="agent-header">
      <div><span>{{ t('stats.agent.kicker') }}</span><h2>{{ t('stats.agent.title') }}</h2></div>
      <button type="button" :disabled="loading" @click="reload">{{ t('stats.agent.refresh') }}</button>
    </header>

    <div v-if="error" class="agent-error">{{ error }}</div>

    <section v-reveal class="agent-panel trace-panel">
      <header>
        <span class="panel-title">
          <h3>{{ t('stats.agent.trace.title') }}</h3>
          <Hint :text="t('stats.agent.time.standard')" :label="t('stats.agent.trace.title')" />
          <small>{{ t('stats.agent.trace.subtitle') }}</small>
        </span>
        <div class="timeline-mode" role="group" :aria-label="t('stats.agent.trace.modeAria')">
          <button
            v-for="option in timelineModes"
            :key="option.id"
            type="button"
            :class="{ active: timelineMode === option.id }"
            :aria-pressed="timelineMode === option.id"
            :title="t(`stats.agent.trace.${option.id}`)"
            @click="timelineMode = option.id"
          >
            {{ t(`stats.agent.trace.${option.id}`) }}
          </button>
          <button
            v-if="timelineMode === 'actual'"
            type="button"
            :class="{ active: foldIdle }"
            :aria-pressed="foldIdle"
            :title="t('stats.agent.trace.foldIdle')"
            @click="foldIdle = !foldIdle"
          >
            {{ t('stats.agent.trace.foldIdle') }}
          </button>
        </div>
      </header>
      <div class="trace-layout">
        <nav class="recent-traces" :aria-label="t('stats.agent.trace.recent')">
          <button
            v-for="trace in railTraces"
            :key="trace.id"
            type="button"
            :class="{ active: selectedTraceId === trace.id }"
            @click="selectTrace(trace.id)"
          ><strong>{{ trace.name }}</strong><small>#{{ trace.id }} · {{ trace.state }}</small></button>
          <button
            v-if="hiddenTraceCount"
            class="add-trace"
            type="button"
            :title="t('stats.agent.trace.add', { count: hiddenTraceCount })"
            :aria-label="t('stats.agent.trace.add', { count: hiddenTraceCount })"
            @click="addTrace"
          >+{{ hiddenTraceCount }}</button>
          <p v-if="!railTraces.length">{{ t('stats.agent.empty.traces') }}</p>
        </nav>
        <div class="trace-drilldown">
          <CompactTraceTimeline
            :segments="model.traceTimelineSegments"
            :mode="timelineMode"
            :fold-idle="foldIdle"
            :lane-labels="{
              model_side: t('stats.agent.time.model_side'),
              agent_side: t('stats.agent.time.agent_side'),
              unattributed: t('stats.agent.time.unattributed'),
            }"
            :aria-label="t('stats.agent.trace.timeline')"
            :empty-label="traceLoading ? t('stats.agent.loading') : t('stats.agent.empty.timeline')"
            :total-label="t('stats.agent.trace.events')"
          />
          <button v-if="selectedTraceId" class="open-trace" type="button" @click="openSelectedTrace">
            {{ t('stats.agent.trace.open') }}
          </button>
        </div>
        <div class="donut-card compact-donut">
          <DonutChart :series="traceTimeSeries" :format-value="formatDurationNanos" :center-label="t('stats.agent.time.total')" />
          <div class="legend"><span v-for="row in traceTimeSeries" :key="row.key"><i :style="{ background: row.color }" />{{ row.label }}</span></div>
        </div>
      </div>
    </section>

    <section v-reveal="1" class="agent-panel">
      <header>
        <span class="panel-title">
          <h3>{{ t('stats.agent.load.title') }}</h3>
          <Hint :text="t('stats.agent.load.subtitle')" :label="t('stats.agent.load.title')" />
        </span>
      </header>
      <div class="load-grid">
        <div class="chart-card tool-mix-card">
          <div class="chart-card-heading">
            <h4>{{ t('stats.agent.load.mix') }}</h4>
            <Hint :text="t('stats.agent.load.mixHint')" :label="t('stats.agent.load.mix')" />
          </div>
          <ToolMixPanel
            v-if="model.toolWorkloads.length"
            :tools="model.toolWorkloads"
            :aria-label="t('stats.agent.load.mix')"
            :total-label="t('stats.agent.load.totalCalls', { count: model.toolWorkloads[0].totalCalls })"
            :unavailable-hint="t('stats.agent.load.durationUnavailable')"
          />
          <p v-else class="chart-empty">{{ t('stats.agent.empty.tools') }}</p>
        </div>
      </div>
    </section>

    <section v-reveal="2" class="agent-panel">
      <header><h3>{{ t('stats.agent.features.title') }}</h3><small>{{ t('stats.agent.features.subtitle') }}</small></header>
      <div class="features-layout">
        <div class="metric-grid">
          <MetricCard :label="t('stats.agent.metrics.turns')" :value="formatDecimal(model.metrics.turns)" tone="input" />
          <MetricCard :label="t('stats.agent.metrics.prompt')" :value="formatDecimal(model.metrics.promptTokens)" tone="output" />
          <MetricCard :label="t('stats.agent.metrics.tools')" :value="formatDecimal(model.metrics.tools)" tone="reasoning" />
          <MetricCard :label="t('stats.agent.metrics.blocks')" :value="formatDecimal(model.metrics.blocks)" tone="cache-hit" />
          <MetricCard :label="t('stats.agent.metrics.reasoning')" :value="formatDecimal(model.metrics.reasoningTokens)" tone="cache-miss" />
          <MetricCard :label="t('stats.agent.metrics.ttft')" :value="formatLatencyUs(model.metrics.ttftUs)" tone="total" />
        </div>
        <div class="chart-card distribution-card">
          <h4>{{ t('stats.agent.features.inputDistribution') }}</h4>
          <InputTokenDistributionChart
            v-if="model.inputTokenSamples.length"
            :samples="model.inputTokenSamples"
            :aria-label="t('stats.agent.features.inputDistribution')"
            :x-label="t('stats.agent.features.inputXAxis')"
            :y-label="t('stats.agent.features.inputYAxis')"
          />
          <p v-else>{{ t('stats.agent.empty.distribution') }}</p>
          <small>{{ t('stats.agent.features.inputHint') }}</small>
        </div>
        <div class="donut-card token-card"><DonutChart :series="model.tokenSeries" :format-value="formatNumber" :center-label="t('stats.agent.features.tokens')" /><div class="legend"><span v-for="row in model.tokenSeries" :key="row.key"><i :style="{ background: row.color }" />{{ row.label }}</span></div></div>
      </div>
    </section>
  </section>
</template>

<script setup>
import { computed, defineAsyncComponent, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { readLlmRequestsActivity, readTimeAttributionActivity, readTraceTimeAttribution } from '../../../api';
import Hint from '../../../components/Hint.vue';
import { useLocale } from '../../../locale';
import { defaultRange, formatLatencyUs, formatNumber, rangeToMillis } from '../llm/model';
import DonutChart from '../llm/DonutChart.vue';
import MetricCard from '../llm/MetricCard.vue';
import CompactTraceTimeline from './CompactTraceTimeline.vue';
import { AgentStatsModel, formatDurationNanos } from './model';

const InputTokenDistributionChart = defineAsyncComponent(() => import('./InputTokenDistributionChart.vue'));
const ToolMixPanel = defineAsyncComponent(() => import('./ToolMixPanel.vue'));

const props = defineProps({ traces: { type: Array, required: true }, refreshNonce: { type: Number, default: 0 } });
const emit = defineEmits(['loading', 'open-trace']);
const { t } = useLocale();
const range = ref(defaultRange());
const DEFAULT_VISIBLE_TRACES = 3;
const TIMELINE_MODE_STORAGE_KEY = 'actrail.stats.timeline-mode';
const timelineModes = Object.freeze([{ id: 'compact' }, { id: 'actual' }]);
const timelineMode = ref(readTimelineMode());
const foldIdle = ref(true);
const llm = ref(null);
const attribution = ref(null);
const traceAttribution = ref(null);
const selectedTraceId = ref(null);
const visibleTraceCount = ref(DEFAULT_VISIBLE_TRACES);
const loading = ref(false);
const traceLoading = ref(false);
const error = ref('');
let controller = null;
let traceController = null;
// `/api/traces` answers oldest-first, so the rail sorts by creation time and
// keeps the remaining traces behind the add entry.
const orderedTraces = computed(() => [...props.traces].sort(compareTraceRecency));
const railTraces = computed(() => orderedTraces.value.slice(0, visibleTraceCount.value));
const hiddenTraceCount = computed(() => Math.max(orderedTraces.value.length - railTraces.value.length, 0));
const model = computed(() => new AgentStatsModel({ llm: llm.value, attribution: attribution.value, trace: traceAttribution.value }));
const traceTimeSeries = computed(() => localizedTimeSeries(
  traceAttribution.value ? model.value.traceTimeSeries : model.value.timeSeries,
));

watch(() => props.refreshNonce, reload);
watch(timelineMode, (mode) => {
  window.localStorage.setItem(TIMELINE_MODE_STORAGE_KEY, mode);
});
watch(railTraces, (rows) => {
  if (!rows.some((row) => row.id === selectedTraceId.value)) selectTrace(rows[0]?.id ?? null);
}, { immediate: true });
watch(loading, (value) => emit('loading', value));
onMounted(reload);
onBeforeUnmount(() => {
  controller?.abort();
  traceController?.abort();
  controller = null;
  traceController = null;
  loading.value = false;
  traceLoading.value = false;
  emit('loading', false);
});

async function reload() {
  const parsed = rangeToMillis(range.value);
  if (!parsed.ok) { error.value = parsed.error; return; }
  if (selectedTraceId.value != null) void selectTrace(selectedTraceId.value);
  controller?.abort();
  const activeController = new AbortController();
  controller = activeController;
  loading.value = true;
  error.value = '';
  try {
    const result = await Promise.all([
      readLlmRequestsActivity({ ...parsed, signal: activeController.signal }),
      readTimeAttributionActivity({ ...parsed, signal: activeController.signal }),
    ]);
    if (controller === activeController) [llm.value, attribution.value] = result;
  } catch (cause) {
    if (controller === activeController && cause.name !== 'AbortError') error.value = cause.message;
  } finally {
    if (controller === activeController) {
      controller = null;
      loading.value = false;
    }
  }
}

async function selectTrace(traceId) {
  traceController?.abort();
  selectedTraceId.value = traceId;
  traceAttribution.value = null;
  if (traceId == null) {
    traceController = null;
    traceLoading.value = false;
    return;
  }
  const activeController = new AbortController();
  traceController = activeController;
  traceLoading.value = true;
  try {
    const result = await readTraceTimeAttribution(traceId, { signal: activeController.signal });
    if (traceController === activeController) traceAttribution.value = result;
  } catch (cause) {
    if (traceController === activeController && cause.name !== 'AbortError') error.value = cause.message;
  } finally {
    if (traceController === activeController) {
      traceController = null;
      traceLoading.value = false;
    }
  }
}

function formatDecimal(value) { return value == null ? '—' : Number(value).toLocaleString(undefined, { maximumFractionDigits: 1 }); }
function addTrace() { visibleTraceCount.value += 1; }
function compareTraceRecency(left, right) {
  const delta = traceCreatedNanos(right) - traceCreatedNanos(left);
  return delta === 0n ? 0 : delta > 0n ? 1 : -1;
}
function traceCreatedNanos(trace) {
  try {
    return BigInt(trace?.created_at_unix_nanos ?? 0);
  } catch {
    return 0n;
  }
}
function readTimelineMode() {
  const stored = window.localStorage.getItem(TIMELINE_MODE_STORAGE_KEY);
  return stored === 'actual' ? 'actual' : 'compact';
}
function localizedTimeSeries(rows) {
  return rows.map((row) => ({ ...row, label: t(`stats.agent.time.${row.key}`) }));
}
function openSelectedTrace() { emit('open-trace', { traceId: selectedTraceId.value, tabId: 'waterfall' }); }
</script>

<style scoped src="./agent-stats.css"></style>
