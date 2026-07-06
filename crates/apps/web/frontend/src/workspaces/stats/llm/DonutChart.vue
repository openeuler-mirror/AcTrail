<template>
  <div class="donut-chart" role="img" :aria-label="effectiveAriaLabel">
    <svg class="donut-svg" viewBox="0 0 240 220" aria-hidden="true">
      <g transform="translate(120 110)">
        <circle
          v-if="slices.length === 1"
          r="74"
          class="donut-full"
          :class="{ active: hoveredSliceKey === slices[0].key }"
          :style="{ stroke: slices[0].color }"
          @pointerenter="setDonutHover(slices[0], $event)"
          @pointermove="moveDonutTooltip"
          @pointerleave="clearDonutHover"
        />
        <path
          v-for="slice in slices"
          v-else
          :key="slice.key"
          class="donut-slice"
          :class="{ active: hoveredSliceKey === slice.key }"
          :d="hoveredSliceKey === slice.key ? slice.activePath : slice.path"
          :style="{ stroke: slice.color }"
          @pointerenter="setDonutHover(slice, $event)"
          @pointermove="moveDonutTooltip"
          @pointerleave="clearDonutHover"
        >
          <title>{{ slice.label }}: {{ formatValue(slice.total) }}</title>
        </path>
        <circle r="48" class="donut-hole" />
        <text class="donut-total" y="-4" text-anchor="middle">{{ formatValue(centerTotal) }}</text>
        <text class="donut-label" y="18" text-anchor="middle">{{ effectiveCenterLabel }}</text>
      </g>
    </svg>
    <div
      v-if="hoveredSlice"
      class="donut-tooltip"
      :style="{ left: `${tooltip.x}px`, top: `${tooltip.y}px` }"
    >
      <strong>{{ hoveredSlice.label }}</strong>
      <span>{{ formatValue(hoveredSlice.total) }} / {{ formatShare(hoveredSlice.share) }}</span>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';

import { useLocale } from '../../../locale';

const props = defineProps({
  ariaLabel: {
    type: String,
    default: '',
  },
  series: {
    type: Array,
    default: () => [],
  },
  totalValue: {
    type: Number,
    default: Number.NaN,
  },
  centerLabel: {
    type: String,
    default: '',
  },
  formatValue: {
    type: Function,
    required: true,
  },
});

const { t } = useLocale();
const hoveredSliceKey = ref(null);
const tooltip = ref({ x: 120, y: 24 });
const effectiveAriaLabel = computed(() => props.ariaLabel || t('stats.llm.chartPanel.donutChart'));
const effectiveCenterLabel = computed(() => props.centerLabel || t('stats.llm.common.total'));

const positiveSeries = computed(() =>
  (props.series ?? [])
    .filter((series) => Math.max(0, Number(series.total ?? series.value ?? 0)) > 0)
    .map((series, index) => ({
      key: String(series.key ?? index),
      label: String(series.label ?? series.key ?? t('stats.llm.chartPanel.seriesFallback', { index: index + 1 })),
      total: Math.max(0, Number(series.total ?? series.value ?? 0)),
      color: series.color ?? 'var(--stats-chart-total)',
    })),
);
const sliceTotal = computed(() => positiveSeries.value.reduce((sum, series) => sum + series.total, 0));
const centerTotal = computed(() => {
  const configured = Number(props.totalValue);
  if (Number.isFinite(configured) && configured >= 0) {
    return configured;
  }
  return sliceTotal.value;
});
const slices = computed(() => correctedSlices(positiveSeries.value, sliceTotal.value));
const hoveredSlice = computed(() => slices.value.find((slice) => slice.key === hoveredSliceKey.value));

function correctedSlices(seriesRows, total) {
  if (!seriesRows.length || total <= 0) {
    return [];
  }
  if (seriesRows.length === 1) {
    return [{ ...seriesRows[0], share: 1, path: '', activePath: '' }];
  }
  const rawAngles = seriesRows.map((series) => (series.total / total) * 360);
  const corrected = correctSmallAngles(rawAngles, Math.min(8, (360 / seriesRows.length) * 0.45));
  let cursor = 0;
  return seriesRows.map((series, index) => {
    const start = cursor;
    const end = index === seriesRows.length - 1 ? 360 : cursor + corrected[index];
    cursor = end;
    return {
      ...series,
      share: series.total / total,
      path: arcPath(0, 0, 74, start, end),
      activePath: arcPath(0, 0, 80, start, end),
    };
  });
}

function correctSmallAngles(rawAngles, minAngle) {
  const smallAngles = rawAngles.filter((angle) => angle > 0 && angle < minAngle);
  if (!smallAngles.length) {
    return rawAngles;
  }
  const borrowed = smallAngles.reduce((sum, angle) => sum + (minAngle - angle), 0);
  const adjustable = rawAngles.reduce((sum, angle) => sum + (angle >= minAngle ? angle : 0), 0);
  if (adjustable <= borrowed) {
    return rawAngles;
  }
  return rawAngles.map((angle) => {
    if (angle > 0 && angle < minAngle) {
      return minAngle;
    }
    return angle - borrowed * (angle / adjustable);
  });
}

function setDonutHover(slice, event) {
  hoveredSliceKey.value = slice.key;
  moveDonutTooltip(event);
}

function moveDonutTooltip(event) {
  const container = event.currentTarget.closest('.donut-chart');
  if (!container) {
    return;
  }
  const rect = container.getBoundingClientRect();
  tooltip.value = {
    x: event.clientX - rect.left + 12,
    y: event.clientY - rect.top + 12,
  };
}

function clearDonutHover() {
  hoveredSliceKey.value = null;
}

function formatShare(value) {
  const share = Number(value ?? 0);
  if (!Number.isFinite(share) || share <= 0) {
    return '0.0%';
  }
  return `${(share * 100).toFixed(share < 0.001 ? 2 : 1)}%`;
}

function arcPath(cx, cy, radius, startAngle, endAngle) {
  const start = polarToCartesian(cx, cy, radius, endAngle);
  const end = polarToCartesian(cx, cy, radius, startAngle);
  const large = endAngle - startAngle <= 180 ? 0 : 1;
  return ['M', start.x, start.y, 'A', radius, radius, 0, large, 0, end.x, end.y].join(' ');
}

function polarToCartesian(cx, cy, radius, angle) {
  const radians = ((angle - 90) * Math.PI) / 180;
  return { x: cx + radius * Math.cos(radians), y: cy + radius * Math.sin(radians) };
}
</script>

<style scoped src="./DonutChart.css"></style>
