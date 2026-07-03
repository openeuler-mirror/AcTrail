<template>
  <section class="sample-distribution-panel">
    <header>
      <div>
        <h3>{{ title }}</h3>
        <p>{{ subtitle }}</p>
      </div>
      <div class="panel-controls">
        <SingleSelectControl
          v-model="binCount"
          :title="t('stats.llm.latency.bins')"
          :options="binOptions"
        />
        <div class="layer-toggle-group" :aria-label="t('stats.llm.latency.chartLayers')">
          <button
            v-for="layer in layerOptions"
            :key="layer.id"
            type="button"
            class="layer-toggle"
            :class="{ selected: isLayerVisible(layer.id) }"
            :aria-pressed="isLayerVisible(layer.id)"
            :disabled="isLastVisibleLayer(layer.id)"
            @click="toggleLayer(layer.id)"
          >
            <span class="layer-glyph" :class="`layer-glyph-${layer.id}`" aria-hidden="true"></span>
            <span>{{ layer.label }}</span>
          </button>
        </div>
      </div>
    </header>

    <div class="latency-summary">
      <span>{{ t('stats.llm.latency.samples') }} <strong>{{ formatNumber(summary.sample_count) }}</strong></span>
      <span>{{ t('stats.llm.latency.avg') }} <strong>{{ formatLatency(summary.mean_us) }}</strong></span>
      <span>{{ t('stats.llm.latency.p50') }} <strong>{{ formatLatency(summary.p50_us) }}</strong></span>
      <span>{{ t('stats.llm.latency.p95') }} <strong>{{ formatLatency(summary.p95_us) }}</strong></span>
    </div>

    <div v-if="hasSamples" class="distribution-body">
      <svg class="distribution-svg" viewBox="0 0 820 320" role="img" :aria-label="title">
        <line class="axis" :x1="chart.left" :x2="chart.right" :y1="chart.bottom" :y2="chart.bottom" />
        <line class="axis" :x1="chart.left" :x2="chart.left" :y1="chart.top" :y2="chart.bottom" />
        <text
          v-if="showHistogram"
          class="axis-label"
          :x="chart.left - 8"
          :y="chart.top + 4"
          text-anchor="end"
        >
          {{ formatDistributionPercent(maxBinRatio) }}
        </text>
        <text
          v-if="showHistogram"
          class="axis-label"
          :x="chart.left - 8"
          :y="chart.bottom + 4"
          text-anchor="end"
        >
          0%
        </text>
        <g v-if="showHistogram" class="histogram-bars">
          <rect
            v-for="bar in histogramBars"
            :key="bar.key"
            class="histogram-bar"
            :x="bar.x"
            :y="bar.y"
            :width="bar.width"
            :height="bar.height"
            rx="3"
            :style="{ fill: bar.color }"
          >
            <title>
              {{ bar.label }} / {{ formatLatency(bar.start) }} - {{ formatLatency(bar.end) }}:
              {{ formatNumber(bar.count) }} ({{ formatDistributionPercent(bar.ratio) }})
            </title>
          </rect>
        </g>
        <template v-if="showKde">
          <path
            v-for="line in kdePaths"
            :key="line.key"
            class="kde-line"
            :d="line.path"
            :style="{ stroke: line.color }"
          />
        </template>
        <g class="x-axis">
          <text
            v-for="tick in xTicks"
            :key="tick.key"
            class="axis-label"
            :x="tick.x"
            :y="chart.bottom + 27"
            text-anchor="middle"
          >
            {{ tick.label }}
          </text>
        </g>
      </svg>
      <div class="legend">
        <button
          v-for="series in normalizedSeries"
          :key="series.key"
          type="button"
          :title="series.key"
          :class="{ muted: !isSeriesVisible(series.key) }"
          :aria-pressed="isSeriesVisible(series.key)"
          :disabled="isLastVisibleSeries(series.key)"
          @click="toggleSeries(series.key)"
        >
          <i class="series-swatch" :style="{ background: series.color }"></i>
          <span class="series-label">{{ series.label }}</span>
        </button>
      </div>
    </div>

    <div v-else class="latency-empty">{{ t('stats.llm.latency.empty') }}</div>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

import { useLocale } from '../../../locale';
import SingleSelectControl from './SingleSelectControl.vue';
import {
  DEFAULT_LATENCY_BIN_COUNT,
  DEFAULT_LATENCY_KDE_POINTS,
  LATENCY_BIN_OPTIONS,
  formatLatencyUs,
  formatNumber,
} from './model';

const props = defineProps({
  title: {
    type: String,
    required: true,
  },
  subtitle: {
    type: String,
    default: '',
  },
  distribution: {
    type: Object,
    default: null,
  },
  series: {
    type: Array,
    default: () => [],
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
const { t } = useLocale();
const chart = Object.freeze({ left: 62, right: 790, top: 28, bottom: 252 });
const binCount = ref(String(DEFAULT_LATENCY_BIN_COUNT));
const visibleLayerIds = ref(['histogram', 'kde']);
const hiddenSeriesKeys = ref([]);
const binOptions = LATENCY_BIN_OPTIONS.map((count) => ({ id: String(count), label: String(count) }));
const layerOptions = computed(() => [
  { id: 'histogram', label: t('stats.llm.latency.histogram') },
  { id: 'kde', label: t('stats.llm.latency.kde') },
]);
const visibleLayerSet = computed(() => new Set(visibleLayerIds.value));
const showHistogram = computed(() => visibleLayerSet.value.has('histogram'));
const showKde = computed(() => visibleLayerSet.value.has('kde'));

const sourceSeries = computed(() => {
  if (Array.isArray(props.series) && props.series.length) {
    return props.series;
  }
  return [
    {
      key: 'total',
      label: props.title,
      distribution: props.distribution,
    },
  ];
});

const normalizedSeries = computed(() =>
  sourceSeries.value
    .map((series, index) => {
      const distribution = normalizeDistribution(series.distribution);
      return {
        key: String(series.key ?? index),
        label: String(series.label ?? series.key ?? t('stats.llm.chartPanel.seriesFallback', { index: index + 1 })),
        color: series.color || palette[index % palette.length],
        distribution,
        samples: distribution.samples_us,
      };
    })
    .filter((series) => series.samples.length > 0),
);
const hiddenSeriesSet = computed(() => new Set(hiddenSeriesKeys.value));
const visibleSeries = computed(() =>
  normalizedSeries.value.filter((series) => !hiddenSeriesSet.value.has(series.key)),
);

const combinedSamples = computed(() =>
  visibleSeries.value.flatMap((series) => series.samples).sort((left, right) => left - right),
);
const hasSamples = computed(() => combinedSamples.value.length > 0);
const summary = computed(() => distributionFromSamples(combinedSamples.value));
const domain = computed(() => {
  if (!combinedSamples.value.length) {
    return { min: 0, max: 1 };
  }
  const min = combinedSamples.value[0];
  const max = combinedSamples.value[combinedSamples.value.length - 1];
  return min === max ? { min: Math.max(0, min - 1), max: max + 1 } : { min, max };
});
const histogramRows = computed(() => buildHistograms(visibleSeries.value, Number(binCount.value), domain.value));
const maxBinRatio = computed(() =>
  Math.max(0.01, ...histogramRows.value.flatMap((series) => series.bins.map((bin) => bin.ratio))),
);
const histogramBars = computed(() => {
  const seriesCount = Math.max(1, histogramRows.value.length);
  return histogramRows.value.flatMap((series, seriesIndex) =>
    series.bins.map((bin) => {
      const startX = scale(bin.start, domain.value.min, domain.value.max, chart.left, chart.right);
      const endX = scale(bin.end, domain.value.min, domain.value.max, chart.left, chart.right);
      const laneWidth = Math.max(1, (endX - startX) / seriesCount);
      const gap = Math.min(3, laneWidth * 0.28);
      const height = scale(bin.ratio, 0, maxBinRatio.value, 0, chart.bottom - chart.top);
      return {
        ...bin,
        key: `${series.key}:${bin.key}`,
        label: series.label,
        color: series.color,
        x: startX + seriesIndex * laneWidth + gap / 2,
        y: chart.bottom - height,
        width: Math.max(1, laneWidth - gap),
        height,
      };
    }),
  );
});
const kdeRows = computed(() =>
  visibleSeries.value.map((series) => ({
    key: series.key,
    color: series.color,
    points: buildKde(series.samples, DEFAULT_LATENCY_KDE_POINTS, domain.value),
  })),
);
const kdePaths = computed(() => {
  return kdeRows.value
    .filter((series) => series.points.length)
    .map((series) => {
      const maxDensity = Math.max(1e-12, ...series.points.map((point) => point.density));
      return {
        key: series.key,
        color: series.color,
        path: series.points
          .map((point, index) => {
            const x = scale(point.value, domain.value.min, domain.value.max, chart.left, chart.right);
            const y = scale(point.density, 0, maxDensity, chart.bottom, chart.top);
            return `${index === 0 ? 'M' : 'L'} ${x} ${y}`;
          })
          .join(' '),
      };
    });
});
const xTicks = computed(() => {
  const count = 4;
  return Array.from({ length: count }, (_, index) => {
    const value = domain.value.min + ((domain.value.max - domain.value.min) * index) / (count - 1);
    return {
      key: index,
      x: scale(value, domain.value.min, domain.value.max, chart.left, chart.right),
      label: formatLatency(value),
    };
  });
});

watch(
  normalizedSeries,
  (series) => {
    const currentKeys = new Set(series.map((item) => item.key));
    const nextHiddenKeys = hiddenSeriesKeys.value.filter((key) => currentKeys.has(key));
    if (nextHiddenKeys.length >= series.length) {
      nextHiddenKeys.pop();
    }
    hiddenSeriesKeys.value = nextHiddenKeys;
  },
  { immediate: true },
);

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
    samples_us: (Array.isArray(value?.samples_us) ? value.samples_us : [])
      .map((sample) => Number(sample))
      .filter((sample) => Number.isFinite(sample) && sample >= 0)
      .sort((left, right) => left - right),
  };
}

function distributionFromSamples(samples) {
  return {
    sample_count: samples.length,
    mean_us: mean(samples),
    p50_us: percentile(samples, 0.5),
    p95_us: percentile(samples, 0.95),
  };
}

function buildHistograms(seriesRows, count, valueDomain) {
  if (!seriesRows.length) {
    return [];
  }
  const binTotal = Math.max(1, Math.trunc(count || DEFAULT_LATENCY_BIN_COUNT));
  const span = Math.max(1, valueDomain.max - valueDomain.min);
  const width = span / binTotal;
  return seriesRows.map((series) => {
    const bins = Array.from({ length: binTotal }, (_, index) => {
      const start = valueDomain.min + index * width;
      const end = index + 1 === binTotal ? valueDomain.max : start + width;
      return { key: index, start, end, count: 0 };
    });
    for (const value of series.samples) {
      const index = Math.min(binTotal - 1, Math.max(0, Math.floor((value - valueDomain.min) / width)));
      bins[index].count += 1;
    }
    const sampleCount = Math.max(1, series.samples.length);
    return {
      key: series.key,
      label: series.label,
      color: series.color,
      bins: bins.map((bin) => ({
        ...bin,
        ratio: bin.count / sampleCount,
      })),
    };
  });
}

function buildKde(source, pointCount, valueDomain) {
  if (!source.length) {
    return [];
  }
  if (source.length === 1) {
    return [{ value: source[0], density: 1 }];
  }
  const bandwidth = kdeBandwidth(source, valueDomain);
  return Array.from({ length: pointCount }, (_, index) => {
    const value = valueDomain.min + ((valueDomain.max - valueDomain.min) * index) / Math.max(1, pointCount - 1);
    const density =
      source.reduce((sum, sample) => {
        const z = (value - sample) / bandwidth;
        return sum + Math.exp(-0.5 * z * z);
      }, 0) /
      (source.length * bandwidth * Math.sqrt(2 * Math.PI));
    return { value, density };
  });
}

function kdeBandwidth(source, valueDomain) {
  const average = source.reduce((sum, value) => sum + value, 0) / source.length;
  const variance = source.reduce((sum, value) => sum + (value - average) ** 2, 0) / source.length;
  const silverman = 1.06 * Math.sqrt(variance) * source.length ** -0.2;
  return Math.max(silverman, (valueDomain.max - valueDomain.min) / 100, 1);
}

function percentile(samples, value) {
  if (!samples.length) {
    return null;
  }
  const index = Math.round((samples.length - 1) * value);
  return samples[index] ?? null;
}

function mean(samples) {
  if (!samples.length) {
    return null;
  }
  return samples.reduce((sum, value) => sum + value, 0) / samples.length;
}

function scale(value, min, max, outMin, outMax) {
  if (max <= min) {
    return (outMin + outMax) / 2;
  }
  return outMin + ((value - min) / (max - min)) * (outMax - outMin);
}

function formatLatency(value) {
  return formatLatencyUs(value);
}

function formatDistributionPercent(value) {
  const ratio = Number(value ?? 0);
  if (!Number.isFinite(ratio)) {
    return '0%';
  }
  return `${(ratio * 100).toFixed(ratio >= 0.1 ? 0 : 1)}%`;
}

function isLayerVisible(layerId) {
  return visibleLayerSet.value.has(layerId);
}

function isLastVisibleLayer(layerId) {
  return isLayerVisible(layerId) && visibleLayerIds.value.length <= 1;
}

function toggleLayer(layerId) {
  if (isLastVisibleLayer(layerId)) {
    return;
  }
  const next = new Set(visibleLayerIds.value);
  if (next.has(layerId)) {
    next.delete(layerId);
  } else {
    next.add(layerId);
  }
  visibleLayerIds.value = Array.from(next);
}

function isSeriesVisible(seriesKey) {
  return !hiddenSeriesSet.value.has(seriesKey);
}

function isLastVisibleSeries(seriesKey) {
  return isSeriesVisible(seriesKey) && visibleSeries.value.length <= 1;
}

function toggleSeries(seriesKey) {
  if (isLastVisibleSeries(seriesKey)) {
    return;
  }
  const next = new Set(hiddenSeriesKeys.value);
  if (next.has(seriesKey)) {
    next.delete(seriesKey);
  } else {
    next.add(seriesKey);
  }
  hiddenSeriesKeys.value = Array.from(next);
}
</script>

<style scoped src="./SampleDistributionPanel.css"></style>
