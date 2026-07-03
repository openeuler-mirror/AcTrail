<template>
  <section class="distribution-panel" :class="{ expanded }">
    <header class="distribution-head">
      <div>
        <h3>{{ title }}</h3>
        <p class="distribution-subtitle" :class="{ empty: !subtitle }">{{ subtitle || '\u00a0' }}</p>
      </div>
      <button
        class="icon-button"
        type="button"
        :title="expanded ? t('stats.llm.chartPanel.collapse') : t('stats.llm.chartPanel.expand')"
        @click="expanded = !expanded"
      >
        <Minimize2 v-if="expanded" :size="16" />
        <Maximize2 v-else :size="16" />
      </button>
    </header>

    <div v-if="normalizedSeries.length" class="distribution-body">
      <div class="distribution-bars" role="list" :aria-label="t('stats.llm.distribution.valuesAria', { title })">
        <button
          v-for="row in barRows"
          :key="row.key"
          type="button"
          class="distribution-row"
          :class="{ muted: isSeriesMuted(row), child: row.parentKey }"
          :disabled="Boolean(row.parentKey && hiddenKeys.has(row.parentKey))"
          :title="`${row.label}: ${formatValue(row.total)} (${formatShare(row.share)})`"
          @click="toggleSeries(row.key)"
        >
          <div class="row-main">
            <span class="row-name">
              <span class="swatch" :style="{ background: row.color }" />
              <span>{{ row.label }}</span>
            </span>
            <span class="row-percent">{{ formatShare(row.share) }}</span>
          </div>
          <div class="row-track" aria-hidden="true">
            <div class="row-fill" :style="{ width: row.width, background: row.color }" />
          </div>
          <div class="row-value">{{ formatValue(row.total) }}</div>
        </button>
      </div>

      <DonutChart
        class="distribution-donut"
        :aria-label="title"
        :series="visibleSeries"
        :total-value="donutTotal"
        :format-value="formatValue"
      />
    </div>

    <div v-else class="distribution-empty">{{ effectiveEmptyLabel }}</div>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue';
import { Maximize2, Minimize2 } from '@lucide/vue';

import { useLocale } from '../../../locale';
import DonutChart from './DonutChart.vue';

const props = defineProps({
  title: {
    type: String,
    required: true,
  },
  subtitle: {
    type: String,
    default: '',
  },
  series: {
    type: Array,
    default: () => [],
  },
  shareDenominator: {
    type: Number,
    default: 0,
  },
  formatValue: {
    type: Function,
    required: true,
  },
  resolveHiddenKeys: {
    type: Function,
    default: null,
  },
  resolveVisibleSeries: {
    type: Function,
    default: null,
  },
  emptyLabel: {
    type: String,
    default: '',
  },
});

const palette = Object.freeze([
  'var(--stats-chart-1)',
  'var(--stats-chart-2)',
  'var(--stats-chart-3)',
  'var(--stats-chart-4)',
  'var(--stats-chart-5)',
  'var(--stats-chart-6)',
  'var(--stats-chart-7)',
  'var(--stats-chart-8)',
]);
const hiddenKeys = ref(new Set());
const expanded = ref(false);
const { t } = useLocale();

const normalizedSeries = computed(() =>
  (props.series ?? [])
    .filter((series) => Number(series.total ?? series.value ?? 0) > 0)
    .map((series, index) => ({
      key: String(series.key ?? index),
      label: String(series.label ?? series.key ?? t('stats.llm.chartPanel.seriesFallback', { index: index + 1 })),
      total: Number(series.total ?? series.value ?? 0),
      parentKey: series.parentKey ? String(series.parentKey) : null,
      color: series.color || palette[index % palette.length],
    })),
);
const denominator = computed(() => {
  const configured = Number(props.shareDenominator ?? 0);
  if (configured > 0) {
    return configured;
  }
  return normalizedSeries.value.reduce((sum, series) => sum + Math.max(0, series.total), 0);
});
const barRows = computed(() =>
  normalizedSeries.value.map((series) => {
    const share = denominator.value > 0 ? Math.max(0, series.total) / denominator.value : 0;
    return {
      ...series,
      share,
      width: `${Math.min(100, Math.max(1.5, share * 100))}%`,
    };
  }),
);
const visibleSeries = computed(() => {
  const defaultSeries = normalizedSeries.value.filter((series) => !isSeriesMuted(series));
  if (!props.resolveVisibleSeries) {
    return defaultSeries;
  }
  const resolved = props.resolveVisibleSeries({
    series: normalizedSeries.value,
    hiddenKeys: new Set(hiddenKeys.value),
  });
  return Array.isArray(resolved) ? resolved : defaultSeries;
});
const donutTotal = computed(() => visibleSeries.value.reduce((sum, series) => sum + Math.max(0, series.total), 0));
const effectiveEmptyLabel = computed(() => props.emptyLabel || t('stats.llm.distribution.empty'));

function toggleSeries(key) {
  if (props.resolveHiddenKeys) {
    const resolved = props.resolveHiddenKeys({
      key,
      hiddenKeys: new Set(hiddenKeys.value),
      series: normalizedSeries.value,
    });
    if (Array.isArray(resolved) || resolved instanceof Set) {
      hiddenKeys.value = new Set(resolved);
      return;
    }
  }
  const next = new Set(hiddenKeys.value);
  if (next.has(key)) {
    next.delete(key);
  } else {
    next.add(key);
  }
  for (const descendant of descendantsOf(key)) {
    if (next.has(key)) {
      next.add(descendant.key);
    } else {
      next.delete(descendant.key);
    }
  }
  hiddenKeys.value = next;
}

function isSeriesMuted(series) {
  return hiddenKeys.value.has(series.key) || Boolean(series.parentKey && hiddenKeys.value.has(series.parentKey));
}

function descendantsOf(parentKey) {
  const children = normalizedSeries.value.filter((series) => series.parentKey === parentKey);
  return children.flatMap((child) => [child, ...descendantsOf(child.key)]);
}

function formatShare(value) {
  const share = Number(value ?? 0);
  if (!Number.isFinite(share) || share <= 0) {
    return '0.0%';
  }
  return `${(share * 100).toFixed(share < 0.001 ? 2 : 1)}%`;
}
</script>

<style scoped src="./DistributionPanel.css"></style>
