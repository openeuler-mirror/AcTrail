<template>
  <div class="tool-duration-layout">
    <EChart v-if="measuredRows.length" class="tool-duration-chart" :option="option" :aria-label="ariaLabel" />
    <div v-if="unmeasuredRows.length" class="unmeasured-tools" role="list">
      <span v-for="row in unmeasuredRows" :key="row.key" role="listitem">
        <strong>{{ row.label }}</strong> · {{ countLabel }}: {{ formatCount(row.callCount) }} · {{ unavailableLabel }}
      </span>
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
  countLabel: { type: String, required: true },
  measuredLabel: { type: String, required: true },
  unavailableLabel: { type: String, required: true },
});

const rows = computed(() => props.tools.map((row) => ({
  ...row,
  callCount: normalizedNumber(row.callCount),
  measuredIntervalCount: normalizedNumber(row.measuredIntervalCount),
  measuredDuration: nullableNumber(row.measuredDuration),
})));
const measuredRows = computed(() => rows.value
  .filter((row) => row.measuredDuration != null && row.measuredIntervalCount > 0)
  .sort((left, right) => right.measuredDuration - left.measuredDuration));
const unmeasuredRows = computed(() => rows.value.filter((row) => row.measuredDuration == null));

const option = computed(() => {
  const chartRows = measuredRows.value;

  return {
    animationDuration: 240,
    grid: { top: 18, right: 30, bottom: 62, left: 170 },
    tooltip: {
      trigger: 'item',
      renderMode: 'richText',
      formatter: ({ data }) => [
        data.name,
        `${props.xLabel}: ${formatDurationNanos(data.value)}`,
        `${props.countLabel}: ${Number(data.count).toLocaleString()}`,
        `${props.measuredLabel}: ${Number(data.measuredIntervalCount).toLocaleString()}`,
      ].join('\n'),
    },
    xAxis: {
      type: 'value',
      name: props.xLabel,
      nameLocation: 'middle',
      nameGap: 44,
      min: 0,
      axisLabel: { formatter: formatDurationNanos, color: 'var(--stats-muted)' },
      axisLine: { lineStyle: { color: 'var(--stats-border-strong)' } },
      splitLine: { lineStyle: { color: 'var(--stats-border)' } },
      nameTextStyle: { color: 'var(--stats-muted)' },
    },
    yAxis: {
      type: 'category',
      name: props.yLabel,
      inverse: true,
      data: chartRows.map((row) => row.label),
      axisLabel: {
        color: 'var(--stats-text)',
        width: 142,
        overflow: 'truncate',
      },
      axisLine: { lineStyle: { color: 'var(--stats-border-strong)' } },
      axisTick: { show: false },
      nameTextStyle: { color: 'var(--stats-muted)' },
    },
    dataZoom: [
      {
        type: 'inside',
        yAxisIndex: 0,
        filterMode: 'filter',
        startValue: 0,
        endValue: Math.min(11, Math.max(0, chartRows.length - 1)),
      },
      {
        type: 'slider',
        yAxisIndex: 0,
        filterMode: 'filter',
        width: 15,
        right: 4,
        startValue: 0,
        endValue: Math.min(11, Math.max(0, chartRows.length - 1)),
        show: chartRows.length > 12,
      },
    ],
    series: [{
      type: 'bar',
      barMaxWidth: 28,
      data: chartRows.map((row) => ({
        name: row.label,
        value: row.measuredDuration,
        count: row.callCount,
        measuredIntervalCount: row.measuredIntervalCount,
        itemStyle: { color: row.color },
      })),
    }],
  };
});

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
.tool-duration-layout { display: grid; gap: 12px; }
.tool-duration-chart {
  width: 100%;
  height: 340px;
  min-height: 340px;
}
.unmeasured-tools { display: flex; flex-wrap: wrap; gap: 8px; color: var(--stats-muted); font-size: var(--stats-font-xs); }
.unmeasured-tools span { padding: 7px 9px; border: 1px solid var(--stats-border); border-radius: var(--stats-radius-sm); }
.unmeasured-tools strong { color: var(--stats-text); }
</style>
