<template>
  <section class="trends-page">
    <div class="trend-controls">
      <SingleSelectControl
        :model-value="selectedRollup"
        :title="t('stats.llm.trends.rollup')"
        :options="rollupOptions"
        @update:model-value="$emit('update-rollup', $event)"
      />
      <MultiSelectFilter
        :title="t('stats.llm.trends.chart')"
        :options="chartModes"
        :model-value="chartModeSelection"
        :default-selected-ids="defaultChartModeIds"
        @update:model-value="chartModeSelection = $event"
      />
    </div>

    <div class="charts">
      <ChartPanel
        :title="t('stats.llm.trends.models')"
        :series="trends.models"
        :modes="activeChartModes"
        :format-value="formatNumber"
        :series-limit-title="t('stats.llm.trends.displayedSeries')"
        :series-limit-options="seriesLimitOptions"
      />
      <ChartPanel
        :title="t('stats.llm.trends.endpoints')"
        :series="trends.endpoints"
        :modes="activeChartModes"
        :format-value="formatNumber"
        :empty-label="t('stats.llm.trends.endpointEmpty')"
        :series-limit-title="t('stats.llm.trends.displayedSeries')"
        :series-limit-options="seriesLimitOptions"
      />
      <ChartPanel
        :title="t('stats.llm.trends.apps')"
        :series="trends.apps"
        :modes="activeChartModes"
        :format-value="formatNumber"
        :empty-label="t('stats.llm.trends.appEmpty')"
        :series-limit-title="t('stats.llm.trends.displayedSeries')"
        :series-limit-options="seriesLimitOptions"
      />
      <ChartPanel
        :title="t('stats.llm.trends.tokenCategories')"
        :series="tokenCategorySeries"
        :modes="activeChartModes"
        :format-value="formatNumber"
        :resolve-hidden-keys="resolveBoundHiddenKeys"
        :resolve-visible-series="resolvePartitionedVisibleSeries"
      />
      <ChartPanel
        :title="t('stats.llm.trends.missingUsage')"
        :series="missingSeries"
        :modes="activeChartModes"
        :format-value="formatNumber"
        :empty-label="t('stats.llm.trends.missingUsageEmpty')"
      />
    </div>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue';

import MultiSelectFilter from '../token/filters/MultiSelectFilter.vue';
import { useLocale } from '../../../locale';
import ChartPanel from './ChartPanel.vue';
import SingleSelectControl from './SingleSelectControl.vue';
import {
  ROLLUPS,
  TOP_N_OPTIONS,
  formatNumber,
  resolveBoundHiddenKeys,
  resolvePartitionedVisibleSeries,
  tokenCategorySeries as buildTokenCategorySeries,
} from './model';

const props = defineProps({
  activity: {
    type: Object,
    required: true,
  },
  rollup: {
    type: String,
    default: 'auto',
  },
});

defineEmits(['update-rollup']);

const { t } = useLocale();
const seriesLimitOptions = computed(() => [
  { id: 'all', label: t('stats.llm.common.allSeries') },
  ...TOP_N_OPTIONS.map((option) => ({ id: String(option), label: t('stats.llm.common.topSeries', { count: option }) })),
]);
const rollupOptions = computed(() => ROLLUPS.map(localizedRollup));
const chartModes = computed(() => [
  { id: 'line', label: t('stats.llm.common.line') },
  { id: 'histogram', label: t('stats.llm.common.histogram') },
]);
const defaultChartModeIds = Object.freeze(['line', 'histogram']);
const chartModeSelection = ref({});
const trends = computed(() => props.activity.trends ?? {});
const activeRollup = computed(() => trends.value.rollup ?? props.activity.range?.rollup ?? ROLLUPS[0].id);
const selectedRollup = computed(() => {
  const active = String(activeRollup.value);
  return rollupOptions.value.some((option) => option.id === active) ? active : ROLLUPS[0].id;
});
const activeChartModeSelection = computed(() => {
  const explicit = chartModes.value.some((mode) =>
    Object.prototype.hasOwnProperty.call(chartModeSelection.value, mode.id),
  );
  if (explicit) {
    return chartModeSelection.value;
  }
  return Object.fromEntries(chartModes.value.map((mode) => [mode.id, defaultChartModeIds.includes(mode.id)]));
});
const activeChartModes = computed(() =>
  chartModes.value.filter((mode) => Boolean(activeChartModeSelection.value[mode.id])).map((mode) => mode.id),
);
const missingSeries = computed(() => [
  {
    key: 'missing_usage',
    label: t('stats.llm.metrics.missingUsage'),
    total: (trends.value.missing_usage ?? []).reduce((sum, point) => sum + Number(point.value ?? 0), 0),
    points: trends.value.missing_usage ?? [],
  },
]);
const tokenCategorySeries = computed(() => buildTokenCategorySeries(trends.value.token_categories ?? [], t));

function localizedRollup(option) {
  return {
    id: option.id,
    label: t(`stats.llm.rollups.${option.id}`),
  };
}

</script>

<style scoped>
.trends-page {
  min-width: 0;
  display: grid;
  gap: var(--stats-section-gap);
}

.trend-controls {
  display: flex;
  align-items: flex-end;
  flex-wrap: wrap;
  gap: var(--stats-space-md);
}

.charts {
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--stats-space-lg);
}

@media (max-width: 940px) {
  .charts {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
