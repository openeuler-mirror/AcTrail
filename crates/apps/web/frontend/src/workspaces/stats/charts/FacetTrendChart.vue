<template>
  <div class="facet-trend-chart">
    <svg
      v-if="facets.length && points.length && selectedModes.length"
      class="facet-svg"
      viewBox="0 0 960 520"
      role="img"
      aria-label="Faceted trend chart"
    >
      <g v-for="facet in renderedFacets" :key="facet.key">
        <text class="facet-title" :x="chart.left" :y="facet.titleY">{{ facet.label }}</text>
        <text class="facet-max" :x="chart.left - 10" :y="facet.top + 4" text-anchor="end">
          {{ formatValue(facet.max) }}
        </text>
        <text class="facet-zero" :x="chart.left - 10" :y="facet.bottom + 4" text-anchor="end">0</text>
        <line class="facet-grid" :x1="chart.left" :x2="chart.right" :y1="facet.top" :y2="facet.top" />
        <line
          class="facet-grid baseline"
          :x1="chart.left"
          :x2="chart.right"
          :y1="facet.bottom"
          :y2="facet.bottom"
        />

        <g v-if="selectedModes.includes('bar')">
          <rect
            v-for="bar in facet.bars"
            :key="bar.key"
            class="facet-bar"
            :x="bar.x"
            :y="bar.y"
            :width="bar.width"
            :height="bar.height"
            :style="{ fill: facet.color }"
          >
            <title>{{ bar.title }}</title>
          </rect>
        </g>
        <path
          v-if="selectedModes.includes('kde')"
          class="facet-line density"
          :style="{ stroke: facet.color }"
          :d="facet.densityPath"
        />
        <path
          v-if="selectedModes.includes('line')"
          class="facet-line"
          :style="{ stroke: facet.color }"
          :d="facet.linePath"
        />

        <g v-if="selectedModes.includes('line')">
          <circle
            v-for="point in facet.linePoints"
            :key="point.key"
            class="facet-point"
            :cx="point.x"
            :cy="point.y"
            :style="{ fill: facet.color }"
            r="2.8"
          >
            <title>{{ point.title }}</title>
          </circle>
        </g>
      </g>
      <g class="x-axis">
        <text v-for="tick in xTicks" :key="tick.label" :x="tick.x" y="500" text-anchor="middle">
          {{ tick.label }}
        </text>
      </g>
    </svg>
    <ChartEmpty v-else message="No chart data in this selection" />
  </div>
</template>

<script setup>
import { computed } from 'vue';

import ChartEmpty from './ChartEmpty.vue';
import { pathForPoints, scalePoint, scaleValue, smoothDensity } from './chartMath';

const props = defineProps({
  points: {
    type: Array,
    required: true,
  },
  facets: {
    type: Array,
    required: true,
  },
  modes: {
    type: Array,
    default: () => ['line'],
  },
  formatValue: {
    type: Function,
    required: true,
  },
});

const chart = Object.freeze({
  left: 86,
  right: 930,
  laneHeight: 82,
  laneGap: 38,
  firstTop: 46,
});

const selectedModes = computed(() =>
  props.modes.filter((mode) => ['line', 'bar', 'kde'].includes(mode)),
);

const renderedFacets = computed(() =>
  props.facets.map((facet, index) => {
    const top = chart.firstTop + index * (chart.laneHeight + chart.laneGap);
    const bottom = top + chart.laneHeight;
    const values = props.points.map((point) => Number(point[facet.field] ?? 0));
    const max = Math.max(1, ...values);
    const linePoints = props.points.map((point, pointIndex) => ({
      key: `${facet.key}:${point.key}`,
      title: `${point.label}: ${props.formatValue(point[facet.field])} ${facet.label}`,
      x: scalePoint(pointIndex, props.points.length, chart.left, chart.right),
      y: scaleValue(point[facet.field], max, top, bottom),
    }));
    return {
      ...facet,
      top,
      bottom,
      titleY: top - 10,
      max,
      linePoints,
      linePath: pathForPoints(linePoints),
      bars: barsForFacet(facet, max, top, bottom),
      densityPath: densityPathForFacet(facet, top, bottom),
    };
  }),
);

const xTicks = computed(() => {
  if (!props.points.length) {
    return [];
  }
  const indexes = new Set([0, Math.floor((props.points.length - 1) / 2), props.points.length - 1]);
  return Array.from(indexes).map((index) => ({
    x: scalePoint(index, props.points.length, chart.left, chart.right),
    label: props.points[index].label,
  }));
});

function barsForFacet(facet, max, top, bottom) {
  const availableWidth = chart.right - chart.left;
  const gap = 4;
  const width = Math.max(4, Math.min(28, availableWidth / Math.max(1, props.points.length) - gap));
  return props.points.map((point, index) => {
    const x = scalePoint(index, props.points.length, chart.left, chart.right) - width / 2;
    const y = scaleValue(point[facet.field], max, top, bottom);
    return {
      key: `${facet.key}:${point.key}`,
      title: `${point.label}: ${props.formatValue(point[facet.field])} ${facet.label}`,
      x,
      y,
      width,
      height: Math.max(1, bottom - y),
    };
  });
}

function densityPathForFacet(facet, top, bottom) {
  const density = smoothDensity(props.points.map((point) => point[facet.field]));
  const maxDensity = Math.max(1, ...density.map((point) => point.value));
  const densityPoints = density.map((point, index) => ({
    x: scalePoint(index, density.length, chart.left, chart.right),
    y: scaleValue(point.value, maxDensity, top, bottom),
  }));
  return pathForPoints(densityPoints);
}
</script>

<style scoped>
.facet-trend-chart {
  min-width: 0;
  min-height: 0;
  height: 100%;
}

.facet-svg {
  width: 100%;
  height: 100%;
  min-height: var(--stats-chart-min-height);
}

.facet-title {
  fill: var(--stats-text);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-title);
  font-weight: var(--stats-weight-medium);
}

.facet-max,
.facet-zero,
.x-axis {
  fill: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-regular);
}

.facet-grid {
  stroke: var(--stats-border);
  stroke-width: var(--stats-chart-grid-width);
}

.facet-grid.baseline {
  stroke: var(--stats-border-strong);
}

.facet-line {
  fill: none;
  stroke-width: var(--stats-chart-line-width);
  stroke-linecap: round;
  stroke-linejoin: round;
}

.facet-line.density {
  opacity: 0.7;
  stroke-dasharray: 8 5;
}

.facet-point {
  stroke: var(--stats-surface-strong);
  stroke-width: var(--stats-chart-point-stroke-width);
}

.facet-bar {
  opacity: 0.34;
  rx: var(--stats-chart-bar-radius);
}
</style>
