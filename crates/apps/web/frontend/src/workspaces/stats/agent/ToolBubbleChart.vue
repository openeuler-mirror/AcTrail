<template>
  <div class="tool-bubble-layout">
    <EChart class="tool-bubble-chart" :option="option" :aria-label="ariaLabel" />
    <div class="tool-bubble-index" role="list" :aria-label="ariaLabel">
      <div v-for="row in rows" :key="row.key" class="tool-bubble-index-row" role="listitem">
        <strong><i :style="{ background: row.color }" />{{ row.label }}</strong>
        <span>{{ yLabel }}: {{ formatCount(row.callCount) }}</span>
        <span>{{ measuredLabel }}: {{ formatCount(row.measuredIntervalCount) }}</span>
        <span v-if="row.measuredDuration != null">{{ xLabel }}: {{ formatDurationNanos(row.averageDuration) }} · {{ totalLabel }}: {{ formatDurationNanos(row.measuredDuration) }}</span>
        <span v-else>{{ totalLabel }}: {{ unavailableLabel }}</span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue';
import EChart from './EChart.vue';
import { formatDurationNanos } from './model';

const props = defineProps({
  tools: { type: Array, default: () => [] },
  ariaLabel: { type: String, required: true },
  xLabel: { type: String, required: true },
  yLabel: { type: String, required: true },
  totalLabel: { type: String, required: true },
  measuredLabel: { type: String, required: true },
  unavailableLabel: { type: String, required: true },
});

const rows = computed(() => props.tools
  .map((row) => ({
    ...row,
    callCount: normalizedNumber(row.callCount),
    measuredIntervalCount: normalizedNumber(row.measuredIntervalCount),
    measuredDuration: nullableNumber(row.measuredDuration),
    averageDuration: nullableNumber(row.averageDuration),
  }))
  .filter((row) => row.callCount > 0));
const measuredRows = computed(() => rows.value.filter((row) => (
  row.measuredDuration != null && row.averageDuration != null && row.measuredIntervalCount > 0
)));

const option = computed(() => {
  let maximumDuration = 0;
  for (const row of measuredRows.value) maximumDuration = Math.max(maximumDuration, row.measuredDuration);

  return {
    animationDuration: 240,
    grid: { top: 52, right: 64, bottom: 72, left: 82, containLabel: false },
    toolbox: {
      right: 34,
      top: 4,
      feature: { restore: { show: true } },
    },
    tooltip: {
      trigger: 'item',
      renderMode: 'richText',
      formatter: ({ data }) => [
        data.name,
        `${props.yLabel}: ${formatCount(data.value[1])}`,
        `${props.xLabel}: ${formatDurationNanos(data.value[0])}`,
        `${props.totalLabel}: ${formatDurationNanos(data.value[2])}`,
        `${props.measuredLabel}: ${formatCount(data.measuredIntervalCount)}`,
      ].join('\n'),
    },
    xAxis: {
      type: 'value',
      name: props.xLabel,
      nameLocation: 'middle',
      nameGap: 48,
      min: 0,
      axisLabel: { formatter: formatDurationNanos, color: 'var(--stats-muted)' },
      axisLine: { lineStyle: { color: 'var(--stats-border-strong)' } },
      splitLine: { lineStyle: { color: 'var(--stats-border)' } },
      nameTextStyle: { color: 'var(--stats-muted)' },
    },
    yAxis: {
      type: 'value',
      name: props.yLabel,
      nameLocation: 'middle',
      nameGap: 52,
      min: 0,
      minInterval: 1,
      axisLabel: { formatter: formatCount, color: 'var(--stats-muted)' },
      axisLine: { lineStyle: { color: 'var(--stats-border-strong)' } },
      splitLine: { lineStyle: { color: 'var(--stats-border)' } },
      nameTextStyle: { color: 'var(--stats-muted)' },
    },
    dataZoom: [
      {
        type: 'inside',
        xAxisIndex: 0,
        filterMode: 'none',
        zoomOnMouseWheel: true,
        moveOnMouseMove: true,
        moveOnMouseWheel: false,
        preventDefaultMouseMove: true,
      },
      {
        type: 'inside',
        yAxisIndex: 0,
        filterMode: 'none',
        zoomOnMouseWheel: true,
        moveOnMouseMove: true,
        moveOnMouseWheel: false,
        preventDefaultMouseMove: true,
      },
      { type: 'slider', xAxisIndex: 0, filterMode: 'none', height: 18, bottom: 8 },
      { type: 'slider', yAxisIndex: 0, filterMode: 'none', width: 18, right: 8 },
    ],
    series: [{
      type: 'scatter',
      data: measuredRows.value.map((row) => ({
        name: row.label,
        value: [row.averageDuration, row.callCount, row.measuredDuration],
        measuredIntervalCount: row.measuredIntervalCount,
        itemStyle: { color: row.color },
      })),
      symbolSize: (value) => bubbleDiameter(value[2], maximumDuration),
      emphasis: { scale: 1.15 },
      label: {
        show: true,
        position: 'top',
        formatter: ({ name }) => name,
        color: 'var(--stats-text)',
        overflow: 'truncate',
        width: 150,
      },
      labelLayout: { hideOverlap: true },
    }],
  };
});

function bubbleDiameter(duration, maximumDuration) {
  if (maximumDuration <= 0) return 14;
  const minimumDiameterSquared = 14 ** 2;
  const maximumDiameterSquared = 56 ** 2;
  return Math.sqrt(
    minimumDiameterSquared
      + (duration / maximumDuration) * (maximumDiameterSquared - minimumDiameterSquared),
  );
}

function normalizedNumber(value) {
  const number = Number(value);
  return Number.isFinite(number) && number >= 0 ? number : 0;
}

function nullableNumber(value) {
  if (value == null) return null;
  const number = Number(value);
  return Number.isFinite(number) && number >= 0 ? number : null;
}

function formatCount(value) {
  return Math.round(Number(value) || 0).toLocaleString();
}
</script>

<style scoped>
.tool-bubble-layout {
  display: grid;
  gap: 16px;
}

.tool-bubble-chart {
  width: 100%;
  height: 380px;
  min-height: 380px;
}

.tool-bubble-index {
  display: grid;
  max-height: 180px;
  gap: 6px;
  overflow: auto;
}

.tool-bubble-index-row {
  display: grid;
  grid-template-columns: minmax(160px, 1.2fr) repeat(3, minmax(150px, 1fr));
  gap: 12px;
  padding: 8px 10px;
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}

.tool-bubble-index-row strong {
  display: inline-flex;
  min-width: 0;
  align-items: center;
  gap: 7px;
  color: var(--stats-text);
  overflow-wrap: anywhere;
}

.tool-bubble-index-row i {
  width: 9px;
  height: 9px;
  flex: 0 0 9px;
  border-radius: 50%;
}

@media (max-width: 760px) {
  .tool-bubble-index-row {
    grid-template-columns: 1fr;
  }
}
</style>
