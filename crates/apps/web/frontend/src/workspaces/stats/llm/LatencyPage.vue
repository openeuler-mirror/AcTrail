<template>
  <section class="latency-page">
    <div class="latency-intro">
      <h3>{{ t('stats.llm.latency.title') }}</h3>
      <p>{{ t('stats.llm.latency.subtitle') }}</p>
    </div>

    <div class="latency-controls">
      <SingleSelectControl
        v-model="dimension"
        :title="t('stats.llm.latency.dimension')"
        :options="dimensionOptions"
      />
    </div>

    <div class="latency-grid">
      <SampleDistributionPanel
        :title="t('stats.llm.latency.ttftDistribution')"
        :subtitle="dimensionSubtitle"
        :series="distributionSeries('ttft')"
      />
      <SampleDistributionPanel
        :title="t('stats.llm.latency.tpotDistribution')"
        :subtitle="dimensionSubtitle"
        :series="distributionSeries('tpot')"
      />
    </div>

    <div class="latency-grid">
      <ChartPanel
        :title="t('stats.llm.latency.avgTtftTrend')"
        :subtitle="trendSubtitle"
        :series="trendSeries('ttft_avg')"
        mode="line"
        :format-value="formatLatencyUs"
        :empty-label="t('stats.llm.latency.empty')"
      />
      <ChartPanel
        :title="t('stats.llm.latency.avgTpotTrend')"
        :subtitle="trendSubtitle"
        :series="trendSeries('tpot_avg')"
        mode="line"
        :format-value="formatLatencyUs"
        :empty-label="t('stats.llm.latency.empty')"
      />
    </div>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue';

import { useLocale } from '../../../locale';
import ChartPanel from './ChartPanel.vue';
import SampleDistributionPanel from './SampleDistributionPanel.vue';
import SingleSelectControl from './SingleSelectControl.vue';
import { formatLatencyUs } from './model';

const props = defineProps({
  activity: {
    type: Object,
    required: true,
  },
});

const { t } = useLocale();
const dimension = ref('models');
const dimensionOptions = computed(() => [
  { id: 'models', label: t('stats.llm.trends.models') },
  { id: 'endpoints', label: t('stats.llm.trends.endpoints') },
  { id: 'apps', label: t('stats.llm.trends.apps') },
]);
const latency = computed(() => props.activity.latency ?? emptyLatency());
const activeGroups = computed(() => normalizeGroups(latency.value.grouped?.[dimension.value]));
const activeTrendGroups = computed(() => normalizeTrendGroups(latency.value.trends?.[dimension.value]));
const activeRollup = computed(() => latency.value.trends?.rollup ?? props.activity.range?.rollup ?? 'auto');
const activeRollupLabel = computed(() =>
  activeRollup.value === 'auto' ? t('stats.llm.common.auto') : t(`stats.llm.rollups.${activeRollup.value}`),
);
const dimensionSubtitle = computed(() =>
  t('stats.llm.latency.groupedBy', { dimension: optionLabel(dimensionOptions.value, dimension.value) }),
);
const trendSubtitle = computed(() =>
  t('stats.llm.latency.avgTrendSubtitle', {
    dimension: optionLabel(dimensionOptions.value, dimension.value),
    rollup: activeRollupLabel.value,
  }),
);

function distributionSeries(metric) {
  return activeGroups.value.map((group) => ({
    key: group.key,
    label: group.label,
    distribution: group[metric] ?? emptyDistribution(),
  }));
}

function trendSeries(metric) {
  return activeTrendGroups.value.map((group) => {
    const points = normalizePoints(group[metric]);
    return {
      key: group.key,
      label: group.label,
      total: averagePointValue(points),
      points,
    };
  });
}

function normalizeGroups(groups) {
  return (Array.isArray(groups) ? groups : []).map((group, index) => ({
    key: String(group?.key ?? index),
    label: String(group?.label ?? group?.key ?? index),
    ttft: normalizeDistribution(group?.ttft),
    tpot: normalizeDistribution(group?.tpot),
  }));
}

function normalizeTrendGroups(groups) {
  return (Array.isArray(groups) ? groups : []).map((group, index) => ({
    key: String(group?.key ?? index),
    label: String(group?.label ?? group?.key ?? index),
    ttft_avg: normalizePoints(group?.ttft_avg),
    tpot_avg: normalizePoints(group?.tpot_avg),
  }));
}

function normalizeDistribution(value) {
  return {
    sample_count: Number(value?.sample_count ?? 0),
    missing_count: Number(value?.missing_count ?? 0),
    min_us: value?.min_us ?? null,
    max_us: value?.max_us ?? null,
    mean_us: value?.mean_us ?? null,
    p50_us: value?.p50_us ?? null,
    p90_us: value?.p90_us ?? null,
    p95_us: value?.p95_us ?? null,
    p99_us: value?.p99_us ?? null,
    samples_us: Array.isArray(value?.samples_us) ? value.samples_us : [],
  };
}

function normalizePoints(points) {
  return (Array.isArray(points) ? points : []).map((point, index) => ({
    bucket_key: String(point?.bucket_key ?? index),
    bucket_label: String(point?.bucket_label ?? point?.bucket_key ?? index),
    bucket_start_ms: Number(point?.bucket_start_ms ?? 0),
    value: Number(point?.value ?? 0),
  }));
}

function averagePointValue(points) {
  if (!points.length) {
    return 0;
  }
  return points.reduce((sum, point) => sum + Number(point.value ?? 0), 0) / points.length;
}

function optionLabel(options, id) {
  return options.find((option) => option.id === id)?.label ?? id;
}

function emptyLatency() {
  return {
    grouped: { models: [], endpoints: [], apps: [] },
    trends: { rollup: 'auto', models: [], endpoints: [], apps: [] },
  };
}

function emptyDistribution() {
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
.latency-page {
  min-width: 0;
  display: grid;
  gap: var(--stats-section-gap);
}

.latency-intro {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-2xs);
}

.latency-intro h3 {
  margin: 0;
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-medium);
  line-height: var(--stats-line-height-tight);
}

.latency-intro p {
  margin: 0;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.latency-controls {
  display: flex;
  align-items: flex-end;
  flex-wrap: wrap;
  gap: var(--stats-space-md);
}

.latency-controls :deep(.single-select-control) {
  width: fit-content;
  min-width: 0;
  max-width: 100%;
}

.latency-controls :deep(.single-select-options) {
  width: fit-content;
  max-width: 100%;
}

.latency-grid {
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--stats-space-lg);
}

@media (max-width: 940px) {
  .latency-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
