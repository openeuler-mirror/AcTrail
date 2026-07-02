<template>
  <div class="distribution-chart">
    <svg
      v-if="items.length"
      class="distribution-svg"
      viewBox="0 0 760 320"
      role="img"
      aria-label="Categorical distribution chart"
    >
      <g v-if="mode === 'pie'">
        <circle
          v-if="segments.length === 1"
          class="pie-slice"
          :style="{ fill: segments[0].color }"
          cx="170"
          cy="158"
          r="108"
        >
          <title>{{ segments[0].title }}</title>
        </circle>
        <template v-else>
          <path
            v-for="segment in segments"
            :key="segment.key"
            class="pie-slice"
            :d="segment.slicePath"
            :style="{ fill: segment.color }"
          >
            <title>{{ segment.title }}</title>
          </path>
        </template>
      </g>
      <g v-else-if="mode === 'donut'">
        <circle
          v-if="segments.length === 1"
          class="donut-arc"
          :style="{ stroke: segments[0].color }"
          cx="170"
          cy="158"
          r="88"
        >
          <title>{{ segments[0].title }}</title>
        </circle>
        <template v-else>
          <path
            v-for="segment in segments"
            :key="segment.key"
            class="donut-arc"
            :d="segment.arcPath"
            :style="{ stroke: segment.color }"
          >
            <title>{{ segment.title }}</title>
          </path>
        </template>
        <circle class="donut-hole" cx="170" cy="158" r="62" />
        <text class="donut-total" x="170" y="154" text-anchor="middle">{{ formattedTotal }}</text>
        <text class="donut-label" x="170" y="176" text-anchor="middle">tokens</text>
      </g>
      <g v-else>
        <g v-for="bar in bars" :key="bar.key">
          <text class="bar-label" x="274" :y="bar.labelY">{{ bar.label }}</text>
          <rect class="bar-track" x="420" :y="bar.y" width="280" height="14" rx="7" />
          <rect
            class="bar-fill"
            x="420"
            :y="bar.y"
            :width="bar.width"
            height="14"
            rx="7"
            :style="{ fill: bar.color }"
          >
            <title>{{ bar.title }}</title>
          </rect>
          <text class="bar-value" x="710" :y="bar.labelY" text-anchor="end">{{ bar.valueLabel }}</text>
        </g>
      </g>
      <g class="legend">
        <g v-for="item in legendItems" :key="item.key" :transform="item.transform">
          <circle r="5" :style="{ fill: item.color }" />
          <text x="12" y="4">{{ item.label }}</text>
        </g>
      </g>
    </svg>
    <ChartEmpty v-else message="No distribution data in this selection" />
  </div>
</template>

<script setup>
import { computed } from 'vue';

import ChartEmpty from './ChartEmpty.vue';
import { describeArc, describeSlice } from './chartMath';

const props = defineProps({
  items: {
    type: Array,
    required: true,
  },
  mode: {
    type: String,
    default: 'donut',
  },
  formatValue: {
    type: Function,
    required: true,
  },
});

const center = Object.freeze({ x: 170, y: 158, radius: 108 });
const total = computed(() =>
  props.items.reduce((sum, item) => sum + Math.max(0, Number(item.value ?? 0)), 0),
);
const formattedTotal = computed(() => props.formatValue(total.value));

const segments = computed(() => {
  let cursor = 0;
  return props.items.map((item) => {
    const fraction = total.value > 0 ? Math.max(0, Number(item.value ?? 0)) / total.value : 0;
    const startAngle = cursor;
    const endAngle = cursor + fraction * 360;
    cursor = endAngle;
    return {
      ...item,
      title: `${item.label}: ${props.formatValue(item.value)}`,
      slicePath: describeSlice(center.x, center.y, center.radius, startAngle, endAngle),
      arcPath: describeArc(center.x, center.y, center.radius - 20, startAngle, endAngle),
    };
  });
});

const bars = computed(() => {
  const maxValue = Math.max(1, ...props.items.map((item) => Number(item.value ?? 0)));
  return props.items.map((item, index) => {
    const y = 72 + index * 52;
    return {
      ...item,
      y,
      labelY: y + 12,
      width: Math.max(2, (Math.max(0, Number(item.value ?? 0)) / maxValue) * 280),
      valueLabel: props.formatValue(item.value),
      title: `${item.label}: ${props.formatValue(item.value)}`,
    };
  });
});

const legendItems = computed(() =>
  props.items.map((item, index) => ({
    ...item,
    transform: `translate(360 ${230 + index * 22})`,
  })),
);
</script>

<style scoped>
.distribution-chart {
  min-width: 0;
  min-height: 0;
}

.distribution-svg {
  width: 100%;
  min-height: var(--stats-distribution-min-height);
}

.pie-slice {
  stroke: var(--stats-surface-strong);
  stroke-width: var(--stats-chart-pie-stroke-width);
}

.donut-arc {
  fill: none;
  stroke-width: var(--stats-chart-donut-width);
  stroke-linecap: butt;
}

.donut-hole {
  fill: var(--stats-surface-strong);
  stroke: var(--stats-border);
}

.donut-total {
  fill: var(--stats-text);
  font-family: var(--stats-serif);
  font-size: var(--stats-font-display-md);
  font-weight: var(--stats-weight-medium);
}

.donut-label,
.legend text,
.bar-label,
.bar-value {
  fill: var(--stats-muted);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-regular);
}

.bar-track {
  fill: var(--stats-border);
}

.bar-fill {
  opacity: 0.86;
}
</style>
