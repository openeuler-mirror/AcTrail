<template>
  <div ref="host" class="e-chart" role="img" :aria-label="ariaLabel" />
</template>

<script setup>
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';

const props = defineProps({
  option: { type: Object, required: true },
  ariaLabel: { type: String, required: true },
});

const host = ref(null);
let chart = null;
let resizeObserver = null;
let resizeFrame = null;
let disposed = false;

watch(() => props.option, (option) => {
  chart?.setOption(option, { notMerge: true, lazyUpdate: true });
});

onMounted(async () => {
  const runtime = await import('./echarts-runtime');
  if (disposed || !host.value) return;

  chart = runtime.init(host.value, null, { renderer: 'svg' });
  chart.setOption(props.option, { notMerge: true, lazyUpdate: true });
  resizeObserver = new ResizeObserver(scheduleResize);
  resizeObserver.observe(host.value);
  scheduleResize();
});

onBeforeUnmount(() => {
  disposed = true;
  resizeObserver?.disconnect();
  if (resizeFrame != null) cancelAnimationFrame(resizeFrame);
  chart?.dispose();
  chart = null;
});

function scheduleResize() {
  if (resizeFrame != null) cancelAnimationFrame(resizeFrame);
  resizeFrame = requestAnimationFrame(() => {
    resizeFrame = null;
    chart?.resize();
  });
}
</script>

<style scoped>
.e-chart {
  width: 100%;
  height: 100%;
}
</style>
