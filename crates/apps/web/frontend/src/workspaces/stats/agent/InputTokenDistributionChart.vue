<template>
  <EChart class="input-token-distribution-chart" :option="option" :aria-label="ariaLabel" />
</template>

<script setup>
import { computed } from 'vue';
import EChart from './EChart.vue';

const props = defineProps({
  samples: { type: Array, default: () => [] },
  ariaLabel: { type: String, required: true },
  xLabel: { type: String, required: true },
  yLabel: { type: String, required: true },
});

const option = computed(() => {
  const bins = equalWidthBins(props.samples);

  return {
    animationDuration: 240,
    grid: { top: 52, right: 32, bottom: 84, left: 66 },
    toolbox: {
      right: 12,
      top: 4,
      feature: { restore: { show: true } },
    },
    tooltip: {
      trigger: 'item',
      renderMode: 'html',
      transitionDuration: 0,
      confine: true,
      formatter: ({ data }) => [
        formatRange(data.start, data.end),
        `${props.yLabel}: ${Number(data.value[1]).toLocaleString()}`,
      ].join('<br>'),
    },
    xAxis: {
      type: 'value',
      name: props.xLabel,
      nameLocation: 'middle',
      nameGap: 46,
      min: bins.length ? bins[0].start : 0,
      max: bins.length ? bins.at(-1).end : undefined,
      minInterval: 1,
      axisLabel: { formatter: formatTokens, color: 'var(--stats-muted)' },
      axisLine: { lineStyle: { color: 'var(--stats-border-strong)' } },
      splitLine: { lineStyle: { color: 'var(--stats-border)' } },
      nameTextStyle: { color: 'var(--stats-muted)' },
    },
    yAxis: {
      type: 'value',
      name: props.yLabel,
      nameLocation: 'middle',
      nameGap: 46,
      min: 0,
      minInterval: 1,
      axisLabel: { formatter: formatTokens, color: 'var(--stats-muted)' },
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
      { type: 'slider', xAxisIndex: 0, filterMode: 'none', height: 18, bottom: 8 },
    ],
    series: [{
      type: 'bar',
      barMaxWidth: 52,
      itemStyle: {
        color: 'var(--stats-chart-input)',
        borderRadius: [3, 3, 0, 0],
      },
      data: bins.map((bin) => ({
        value: [bin.center, bin.count],
        start: bin.start,
        end: bin.end,
      })),
    }],
  };
});

function equalWidthBins(samples) {
  const values = [];
  let minimum = Infinity;
  let maximum = -Infinity;
  for (const sample of samples) {
    const value = Number(sample);
    if (!Number.isFinite(value) || value < 0) continue;
    values.push(value);
    minimum = Math.min(minimum, value);
    maximum = Math.max(maximum, value);
  }
  if (!values.length) return [];
  if (minimum === maximum) {
    return [{ start: minimum, end: minimum + 1, center: minimum + 0.5, count: values.length }];
  }

  const binCount = Math.min(20, Math.max(1, Math.ceil(Math.sqrt(values.length))));
  const width = (maximum - minimum) / binCount;
  const bins = Array.from({ length: binCount }, (_, index) => ({
    start: minimum + width * index,
    end: minimum + width * (index + 1),
    center: minimum + width * (index + 0.5),
    count: 0,
  }));
  for (const value of values) {
    const index = Math.min(binCount - 1, Math.floor((value - minimum) / width));
    bins[index].count += 1;
  }
  return bins;
}

function formatRange(start, end) {
  return `${formatTokens(start)}–${formatTokens(end)} ${props.xLabel}`;
}

function formatTokens(value) {
  const number = Number(value) || 0;
  return Math.round(number).toLocaleString(undefined, { notation: number >= 10_000 ? 'compact' : 'standard' });
}
</script>

<style scoped>
.input-token-distribution-chart {
  width: 100%;
  height: 300px;
  min-height: 300px;
}
</style>
