<template>
  <section class="chart-panel" :class="{ expanded }">
    <header class="chart-head">
      <div>
        <h3>{{ title }}</h3>
        <p v-if="subtitle">{{ subtitle }}</p>
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

    <div v-if="hasSeriesLimit" class="chart-controls">
      <SingleSelectControl
        v-model="seriesLimit"
        :title="effectiveSeriesLimitTitle"
        :options="seriesLimitOptions"
      />
    </div>

    <div v-if="normalizedSeries.length" class="chart-body" :class="bodyClass">
      <div v-if="!visibleSeries.length" class="chart-empty compact">{{ t('stats.llm.chartPanel.noVisibleSeries') }}</div>
      <div v-else-if="!activeModes.length" class="chart-empty compact">{{ t('stats.llm.chartPanel.selectChartMode') }}</div>

      <svg
        v-else-if="usesTimeChart"
        class="chart-svg time-svg"
        viewBox="0 0 820 320"
        role="img"
        :aria-label="title"
      >
        <line class="axis" :x1="timeChart.left" :x2="timeChart.right" :y1="timeChart.bottom" :y2="timeChart.bottom" />
        <line class="axis" :x1="timeChart.left" :x2="timeChart.left" :y1="timeChart.top" :y2="timeChart.bottom" />
        <text class="axis-label" :x="timeChart.left - 8" :y="timeChart.top + 4" text-anchor="end">
          {{ formatValue(maxTimeValue) }}
        </text>
        <text class="axis-label" :x="timeChart.left - 8" :y="timeChart.bottom + 4" text-anchor="end">0</text>
        <g class="x-axis">
          <line
            v-for="tick in xTicks"
            :key="`${tick.key}:line`"
            class="x-tick"
            :x1="tick.x"
            :x2="tick.x"
            :y1="timeChart.bottom"
            :y2="timeChart.bottom + 5"
          />
          <text
            v-for="tick in xTicks"
            :key="`${tick.key}:label`"
            class="x-axis-label"
            :x="tick.x"
            :y="timeChart.bottom + 27"
            text-anchor="middle"
          >
            {{ tick.label }}
          </text>
        </g>
        <g v-if="hasTimeBars" class="time-bars">
          <rect
            v-for="bar in timeBars"
            :key="bar.key"
            class="time-bar"
            :x="bar.x"
            :y="bar.y"
            :width="bar.width"
            :height="bar.height"
            rx="3"
            :style="{ fill: bar.color }"
          >
            <title>{{ bar.label }}: {{ formatValue(bar.value) }}</title>
          </rect>
        </g>
        <template v-if="hasLine">
          <path
            v-for="series in lineSeries"
            :key="series.key"
            class="line-path"
            :d="series.path"
            :style="{ stroke: series.color }"
          />
          <g v-for="series in lineSeries" :key="`${series.key}:points`">
            <circle
              v-for="point in series.points"
              :key="point.key"
              r="3"
              class="line-point"
              :cx="point.x"
              :cy="point.y"
              :style="{ fill: series.color }"
            >
              <title>{{ series.label }} / {{ point.label }}: {{ formatValue(point.value) }}</title>
            </circle>
          </g>
        </template>
      </svg>

      <DonutChart
        v-else-if="mode === 'donut'"
        :aria-label="title"
        :series="visibleSeries"
        :total-value="total"
        :format-value="formatValue"
      />

      <div v-else-if="hasBar" class="bar-list" role="img" :aria-label="title">
        <div v-for="bar in barRows" :key="bar.key" class="bar-row">
          <div class="bar-name" :title="bar.key">{{ bar.label }}</div>
          <div class="bar-track" aria-hidden="true">
            <div class="bar-fill" :style="{ width: bar.width, background: bar.color }" />
          </div>
          <div class="bar-value">{{ formatValue(bar.total) }}</div>
        </div>
      </div>

      <div v-else class="chart-empty compact">{{ t('stats.llm.chartPanel.unsupportedMode', { mode }) }}</div>

      <div class="legend" :aria-label="t('stats.llm.chartPanel.legend')">
        <button
          v-for="series in normalizedSeries"
          :key="series.key"
          type="button"
          :class="{ muted: isSeriesMuted(series), child: series.parentKey }"
          :disabled="Boolean(series.parentKey && hiddenKeys.has(series.parentKey))"
          :title="series.key"
          @click="toggleSeries(series.key)"
        >
          <span class="swatch" :style="{ background: series.color }" />
          <span>{{ series.label }}</span>
        </button>
      </div>
    </div>

    <div v-else class="chart-empty">{{ effectiveEmptyLabel }}</div>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue';
import { Maximize2, Minimize2 } from '@lucide/vue';

import { useLocale } from '../../../locale';
import DonutChart from './DonutChart.vue';
import SingleSelectControl from './SingleSelectControl.vue';

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
  mode: {
    type: String,
    default: 'bar',
  },
  modes: {
    type: Array,
    default: null,
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
  seriesLimitOptions: {
    type: Array,
    default: () => [],
  },
  seriesLimitTitle: {
    type: String,
    default: '',
  },
  defaultSeriesLimit: {
    type: [String, Number],
    default: 'all',
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
const seriesLimit = ref(props.defaultSeriesLimit);
const { t } = useLocale();
const timeChart = Object.freeze({
  left: 62,
  right: 790,
  top: 34,
  bottom: 252,
});

const hasSeriesLimit = computed(() => Array.isArray(props.seriesLimitOptions) && props.seriesLimitOptions.length > 0);
const effectiveSeriesLimitTitle = computed(() => props.seriesLimitTitle || t('stats.llm.trends.displayedSeries'));
const limitedInputSeries = computed(() => {
  if (!hasSeriesLimit.value || String(seriesLimit.value) === 'all') {
    return props.series ?? [];
  }
  const limit = Number(seriesLimit.value);
  if (!Number.isFinite(limit) || limit <= 0) {
    return props.series ?? [];
  }
  return (props.series ?? []).slice(0, limit);
});

const normalizedSeries = computed(() =>
  limitedInputSeries.value
    .filter((series) => Number(series.total ?? series.value ?? 0) > 0 || (series.points ?? []).length)
    .map((series, index) => ({
      key: String(series.key ?? index),
      label: String(series.label ?? series.key ?? t('stats.llm.chartPanel.seriesFallback', { index: index + 1 })),
      total: Number(series.total ?? series.value ?? 0),
      points: Array.isArray(series.points) ? series.points : [],
      parentKey: series.parentKey ? String(series.parentKey) : null,
      color: series.color || palette[index % palette.length],
    })),
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
const activeModes = computed(() => {
  const modes = Array.isArray(props.modes) && props.modes.length ? props.modes : [props.mode];
  return modes.filter((mode) => ['line', 'histogram', 'bar', 'donut'].includes(mode));
});
const bodyClass = computed(() => activeModes.value.map((mode) => `mode-${mode}`));
const hasLine = computed(() => activeModes.value.includes('line'));
const hasTimeBars = computed(() => activeModes.value.includes('histogram'));
const hasBar = computed(() => activeModes.value.includes('bar'));
const usesTimeChart = computed(() => hasLine.value || hasTimeBars.value);
const total = computed(() => visibleSeries.value.reduce((sum, series) => sum + Math.max(0, series.total), 0));
const effectiveEmptyLabel = computed(() => props.emptyLabel || t('stats.llm.chartPanel.empty'));
const timeBuckets = computed(() => {
  const bucketMap = new Map();
  for (const series of visibleSeries.value) {
    for (const point of series.points) {
      const key = String(point.bucket_key ?? point.bucket_label ?? `${series.key}:bucket`);
      if (!bucketMap.has(key)) {
        bucketMap.set(key, {
          key,
          label: String(point.bucket_label ?? point.bucket_key ?? key),
          startMs: Number(point.bucket_start_ms ?? 0),
        });
      }
    }
  }
  const rows = Array.from(bucketMap.values());
  if (rows.length) {
    return rows.sort((left, right) => left.startMs - right.startMs || left.key.localeCompare(right.key));
  }
  return visibleSeries.value.map((series) => ({
    key: series.key,
    label: series.label,
    startMs: 0,
  }));
});
const timeBucketTotals = computed(() =>
  timeBuckets.value.map((bucket) =>
    visibleSeries.value.reduce((sum, series) => sum + pointValueForBucket(series, bucket), 0),
  ),
);
const timePointValues = computed(() =>
  visibleSeries.value.flatMap((series) => timeBuckets.value.map((bucket) => pointValueForBucket(series, bucket))),
);
const maxTimeValue = computed(() =>
  Math.max(1, ...timePointValues.value, ...timeBucketTotals.value),
);

const barRows = computed(() => {
  const max = Math.max(1, ...visibleSeries.value.map((series) => Number(series.total ?? 0)));
  return visibleSeries.value.map((series) => ({
    ...series,
    width: `${Math.max(2, (Math.max(0, series.total) / max) * 100)}%`,
  }));
});

const timeBars = computed(() => {
  const width = timeBarWidth(timeBuckets.value.length);
  const chartHeight = timeChart.bottom - timeChart.top;
  return timeBuckets.value.flatMap((bucket, bucketIndex) => {
    const x = scalePoint(bucketIndex, timeBuckets.value.length, timeChart.left, timeChart.right) - width / 2;
    let cursor = timeChart.bottom;
    return visibleSeries.value
      .map((series) => {
        const value = pointValueForBucket(series, bucket);
        if (value <= 0) {
          return null;
        }
        const height = Math.max(1, (value / maxTimeValue.value) * chartHeight);
        cursor -= height;
        return {
          key: `${bucket.key}:${series.key}`,
          label: `${bucket.label} / ${series.label}`,
          value,
          x,
          y: Math.max(timeChart.top, cursor),
          width,
          height: Math.min(height, timeChart.bottom - timeChart.top),
          color: series.color,
        };
      })
      .filter(Boolean);
  });
});

const lineSeries = computed(() => {
  return visibleSeries.value.map((series) => {
    const points = timeBuckets.value.map((bucket, index) => {
      const source = pointForBucket(series, bucket);
      const value = pointValueForBucket(series, bucket);
      return {
        key: `${series.key}:${bucket.key}`,
        label: source?.bucket_label ?? bucket.label,
        value,
        x: scalePoint(index, timeBuckets.value.length, timeChart.left, timeChart.right),
        y: scaleValue(value, maxTimeValue.value, timeChart.top, timeChart.bottom),
      };
    });
    return { ...series, points, path: pathForPoints(points) };
  });
});
const xTicks = computed(() => {
  const buckets = timeBuckets.value;
  if (!buckets.length) {
    return [];
  }
  const maxTicks = Math.min(6, buckets.length);
  const indexes = new Set();
  for (let index = 0; index < maxTicks; index += 1) {
    indexes.add(Math.round((index / Math.max(1, maxTicks - 1)) * (buckets.length - 1)));
  }
  return Array.from(indexes)
    .sort((left, right) => left - right)
    .map((index) => ({
      ...buckets[index],
      x: scalePoint(index, buckets.length, timeChart.left, timeChart.right),
    }));
});

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

function scalePoint(index, count, min, max) {
  if (count <= 1) {
    return (min + max) / 2;
  }
  return min + (index / (count - 1)) * (max - min);
}

function scaleValue(value, max, top, bottom) {
  return bottom - (Math.max(0, Number(value ?? 0)) / Math.max(1, max)) * (bottom - top);
}

function timeBarWidth(count) {
  const availableWidth = timeChart.right - timeChart.left;
  return Math.max(6, Math.min(30, availableWidth / Math.max(1, count) - 6));
}

function pointForBucket(series, bucket) {
  return series.points.find(
    (point) => String(point.bucket_key ?? point.bucket_label ?? `${series.key}:bucket`) === bucket.key,
  );
}

function pointValueForBucket(series, bucket) {
  const source = pointForBucket(series, bucket);
  if (source) {
    return Math.max(0, Number(source.value ?? 0));
  }
  if (!series.points.length && series.key === bucket.key) {
    return Math.max(0, Number(series.total ?? 0));
  }
  return 0;
}

function pathForPoints(points) {
  if (!points.length) {
    return '';
  }
  return points.map((point, index) => `${index === 0 ? 'M' : 'L'} ${point.x} ${point.y}`).join(' ');
}
</script>

<style scoped src="./ChartPanel.css"></style>
