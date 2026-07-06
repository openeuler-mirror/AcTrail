<template>
  <section class="llm-requests-workspace">
    <LlmRequestsHeader
      :from-date="range.fromDate"
      :to-date="range.toDate"
      :query="searchQuery"
      :loading="loading"
      @update-range="setRange"
      @update-query="searchQuery = $event"
      @quick-range="setQuickRange"
      @refresh="reload"
      @export="exportRows"
    />

    <nav class="llm-tabs" :aria-label="t('stats.llm.tabs.aria')">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        type="button"
        :class="{ active: activeTab === tab.id }"
        @click="activeTab = tab.id"
      >
        {{ tab.label }}
      </button>
    </nav>

    <div v-if="error" class="error">{{ error }}</div>

    <OverviewPage
      v-if="activeTab === 'overview'"
      :activity="activity"
      :rows="rows"
      :row-total="rowTotal"
      :can-load-more="canLoadMore"
      :query="searchQuery"
      @load-more="loadMoreRows"
      @open-trace="$emit('open-trace', $event)"
    />
    <TrendsPage
      v-else-if="activeTab === 'trends'"
      :activity="activity"
      :rollup="activityRollup"
      @update-rollup="setActivityRollup"
    />
    <LatencyPage
      v-else-if="activeTab === 'latency'"
      :activity="activity"
    />
    <ExplorePage
      v-else-if="activeTab === 'explore' && parsedRange.ok"
      :from-ms="parsedRange.fromMs"
      :to-ms="parsedRange.toMs"
      :default-rollup="activity.range?.rollup"
    />
    <SettingsPage
      v-else-if="activeTab === 'settings'"
      :pricing-enabled="Boolean(activity.capabilities?.pricing)"
    />
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import { readLlmRequestRows, readLlmRequestsActivity, readLlmRequestsCsv } from '../../../api';
import { useLocale } from '../../../locale';
import ExplorePage from './ExplorePage.vue';
import LatencyPage from './LatencyPage.vue';
import LlmRequestsHeader from './LlmRequestsHeader.vue';
import OverviewPage from './OverviewPage.vue';
import SettingsPage from './SettingsPage.vue';
import TrendsPage from './TrendsPage.vue';
import { LLM_TABS, defaultRange, downloadText, quickRange, rangeToMillis } from './model';

const props = defineProps({
  query: {
    type: String,
    default: '',
  },
});

const emit = defineEmits(['loading', 'open-trace']);

const { t } = useLocale();
const tabs = computed(() =>
  LLM_TABS.map((tab) => ({
    ...tab,
    label: t(`stats.llm.tabs.${tab.id}`),
  })),
);
const activeTab = ref('overview');
const range = ref(defaultRange());
const activityRollup = ref('auto');
const searchQuery = ref(props.query);
const activity = ref(emptyActivity());
const rows = ref([]);
const rowTotal = ref(0);
const rowLimit = 50;
const error = ref('');
const loading = ref(false);
let controller = null;

const parsedRange = computed(() => rangeToMillis(range.value));
const canLoadMore = computed(() => rows.value.length < rowTotal.value);

watch(
  () => props.query,
  (value) => {
    searchQuery.value = value;
  },
);

watch(
  () => [range.value.fromDate, range.value.toDate],
  () => {
    reload();
  },
);

watch(
  loading,
  (value) => {
    emit('loading', value);
  },
  { immediate: true },
);

onMounted(() => {
  reload();
});

onBeforeUnmount(() => {
  controller?.abort();
  emit('loading', false);
});

function setRange(nextRange) {
  range.value = nextRange;
}

function setQuickRange(days) {
  range.value = quickRange(days);
}

async function reload() {
  const parsed = parsedRange.value;
  if (!parsed.ok) {
    error.value = parsed.error;
    activity.value = emptyActivity();
    rows.value = [];
    rowTotal.value = 0;
    return;
  }
  controller?.abort();
  controller = new AbortController();
  loading.value = true;
  error.value = '';
  try {
    const [activityResult, rowResult] = await Promise.all([
      readLlmRequestsActivity({
        fromMs: parsed.fromMs,
        toMs: parsed.toMs,
        rollup: activityRollup.value,
        signal: controller.signal,
      }),
      readLlmRequestRows({
        fromMs: parsed.fromMs,
        toMs: parsed.toMs,
        offset: 0,
        limit: rowLimit,
        signal: controller.signal,
      }),
    ]);
    activity.value = normalizeActivity(activityResult);
    rows.value = Array.isArray(rowResult.rows) ? rowResult.rows : [];
    rowTotal.value = Number(rowResult.page?.total ?? rows.value.length);
  } catch (err) {
    if (err?.name !== 'AbortError') {
      error.value = String(err.message ?? err);
      activity.value = emptyActivity();
      rows.value = [];
      rowTotal.value = 0;
    }
  } finally {
    loading.value = false;
  }
}

function setActivityRollup(value) {
  activityRollup.value = value || 'auto';
  reload();
}

async function loadMoreRows() {
  const parsed = parsedRange.value;
  if (!parsed.ok || loading.value) {
    return;
  }
  controller?.abort();
  controller = new AbortController();
  loading.value = true;
  error.value = '';
  try {
    const rowResult = await readLlmRequestRows({
      fromMs: parsed.fromMs,
      toMs: parsed.toMs,
      offset: rows.value.length,
      limit: rowLimit,
      signal: controller.signal,
    });
    rows.value = rows.value.concat(Array.isArray(rowResult.rows) ? rowResult.rows : []);
    rowTotal.value = Number(rowResult.page?.total ?? rowTotal.value);
  } catch (err) {
    if (err?.name !== 'AbortError') {
      error.value = String(err.message ?? err);
    }
  } finally {
    loading.value = false;
  }
}

async function exportRows() {
  const parsed = parsedRange.value;
  if (!parsed.ok) {
    error.value = parsed.error;
    return;
  }
  loading.value = true;
  error.value = '';
  try {
    const csv = await readLlmRequestsCsv({
      fromMs: parsed.fromMs,
      toMs: parsed.toMs,
      view: activeTab.value === 'overview' ? 'overview' : 'rows',
    });
    downloadText(`actrail-llm-requests-${activeTab.value}.csv`, 'text/csv;charset=utf-8', csv);
  } catch (err) {
    error.value = String(err.message ?? err);
  } finally {
    loading.value = false;
  }
}

function normalizeActivity(value) {
  return {
    range: value?.range ?? null,
    capabilities: value?.capabilities ?? { pricing: false, failed_requests: false, guardrails: false },
    summary: value?.summary ?? emptyActivity().summary,
    coverage: value?.coverage ?? {},
    overview: value?.overview ?? {},
    trends: value?.trends ?? {},
    latency: value?.latency ?? emptyActivity().latency,
  };
}

function emptyActivity() {
  return {
    range: null,
    capabilities: { pricing: false, failed_requests: false, guardrails: false },
    summary: {
      completed_requests: 0,
      failed_requests: null,
      missing_usage_count: 0,
      trace_count: 0,
      model_count: 0,
      endpoint_count: 0,
      app_count: 0,
      total_tokens: 0,
      input_tokens: 0,
      output_tokens: 0,
      reasoning_tokens: 0,
      cache_hit_tokens: 0,
      cache_miss_tokens: 0,
      estimated_spend_cny: null,
    },
    coverage: {},
    overview: { top_models: [], top_endpoints: [], top_apps: [], token_categories: [] },
    trends: { models: [], endpoints: [], apps: [], token_categories: [], missing_usage: [] },
    latency: {
      ttft: emptyLatencyDistribution(),
      tpot: emptyLatencyDistribution(),
      grouped: {
        models: [],
        endpoints: [],
        apps: [],
      },
      trends: {
        rollup: 'auto',
        models: [],
        endpoints: [],
        apps: [],
      },
    },
  };
}

function emptyLatencyDistribution() {
  return {
    sample_count: 0,
    missing_count: 0,
    min_us: null,
    max_us: null,
    mean_us: null,
    p50_us: null,
    p90_us: null,
    p95_us: null,
    p99_us: null,
    samples_us: [],
  };
}
</script>

<style scoped>
.llm-requests-workspace {
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: var(--stats-section-gap);
  width: 100%;
  max-width: none;
  margin: 0;
  padding: var(--stats-viewport-padding);
  font-family: var(--stats-body-font);
}

.llm-requests-workspace :deep(h2),
.llm-requests-workspace :deep(h3) {
  font-family: var(--stats-heading-font);
}

.llm-requests-workspace :deep(.metric-card strong),
.llm-requests-workspace :deep(.donut-total) {
  font-family: var(--stats-value-font);
}

.llm-tabs {
  display: inline-flex;
  width: fit-content;
  max-width: 100%;
  gap: var(--stats-space-2xs);
  padding: var(--stats-space-2xs);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
}

.llm-tabs button {
  min-height: var(--stats-control-height-md);
  padding: 0 var(--stats-segment-padding-x);
  border: 0;
  border-radius: var(--stats-radius-sm);
  background: transparent;
  color: var(--stats-muted);
  cursor: pointer;
  font-size: var(--stats-font-sm);
}

.llm-tabs button:hover,
.llm-tabs button.active {
  background: var(--stats-accent);
  color: var(--stats-on-accent);
}

.error {
  padding: var(--stats-space-sm) var(--stats-space-md);
  border: 1px solid rgba(190, 18, 60, 0.22);
  border-radius: var(--stats-radius-sm);
  background: rgba(190, 18, 60, 0.1);
  color: var(--stats-danger);
  font-size: var(--stats-font-sm);
}

@media (max-width: 760px) {
  .llm-requests-workspace {
    gap: var(--stats-section-gap-mobile);
    padding: var(--stats-viewport-padding-mobile);
  }
}
</style>
