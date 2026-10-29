<!-- Draw one resource track (areas and lines with gaps) on the shared flame graph viewport. -->
<template>
  <canvas
    ref="canvas"
    class="flame-track-canvas res-track-canvas"
    role="img"
    :aria-label="label"
    @pointermove="handlePointerMove"
    @pointerleave="emit('hover', null)"
  ></canvas>
</template>

<script setup>
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';

import { cancelFlameFrame, registerFlameDraw, scheduleFlameFrame } from '../frame-scheduler.js';

const props = defineProps({
  track: { type: Object, required: true },
  viewport: { type: Object, required: true },
  hoverMs: { type: Number, default: null },
  label: { type: String, default: '' },
});
const emit = defineEmits(['hover']);

const PAD_TOP = 4;
const PAD_BOTTOM = 3;
const canvas = ref(null);
let size = null;
let palette = null;
let gapPattern = null;
let resizeObserver = null;
let themeObserver = null;
let unregisterDraw = null;

watch(() => [props.track, props.hoverMs], scheduleDraw);

onMounted(() => {
  unregisterDraw = registerFlameDraw(draw);
  resizeObserver = new ResizeObserver(([entry]) => {
    size = { width: entry?.contentRect.width ?? 0, height: entry?.contentRect.height ?? 0 };
    scheduleDraw();
  });
  resizeObserver.observe(canvas.value);
  themeObserver = new MutationObserver(() => {
    palette = null;
    gapPattern = null;
    scheduleDraw();
  });
  themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['class', 'data-palette'] });
  scheduleDraw();
});

onBeforeUnmount(() => {
  unregisterDraw?.();
  cancelFlameFrame(draw);
  resizeObserver?.disconnect();
  themeObserver?.disconnect();
});

function scheduleDraw() {
  scheduleFlameFrame(draw);
}

function draw() {
  const surface = canvas.value;
  const width = size?.width ?? 0;
  const height = size?.height ?? 0;
  if (!surface || !width || !height) return;
  const ratio = Math.min(globalThis.devicePixelRatio || 1, 2);
  const physicalWidth = Math.max(Math.round(width * ratio), 1);
  const physicalHeight = Math.max(Math.round(height * ratio), 1);
  if (surface.width !== physicalWidth || surface.height !== physicalHeight) {
    surface.width = physicalWidth;
    surface.height = physicalHeight;
  }
  const context = surface.getContext('2d');
  if (!context) return;
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, width, height);
  palette ??= readPalette(surface);
  gapPattern ??= createGapPattern(context, palette.muted);

  const { startMs, spanMs } = props.viewport;
  const x = (ms) => ((ms - startMs) / spanMs) * width;
  const plotHeight = height - PAD_TOP - PAD_BOTTOM;
  const yMax = props.track.yMax || 1;
  const y = (value) => PAD_TOP + plotHeight * (1 - Math.min(Math.max(value / yMax, 0), 1));

  for (const [from, to] of props.track.gaps) {
    const left = Math.max(x(from), 0);
    const right = Math.min(x(to), width);
    if (right > left) {
      context.fillStyle = gapPattern;
      context.fillRect(left, 0, right - left, height);
    }
  }

  context.strokeStyle = palette.grid;
  context.lineWidth = 1;
  context.beginPath();
  context.moveTo(0, y(0) + 0.5);
  context.lineTo(width, y(0) + 0.5);
  context.stroke();

  for (const series of props.track.series) {
    const color = palette.colors[series.colorVar] ?? palette.muted;
    for (const segment of series.segments) {
      const visible = segment.filter(([ms], index) => {
        const next = segment[index + 1]?.[0] ?? ms;
        const previous = segment[index - 1]?.[0] ?? ms;
        return next >= startMs && previous <= startMs + spanMs;
      });
      if (!visible.length) continue;
      context.beginPath();
      visible.forEach(([ms, value], index) => {
        if (index === 0) context.moveTo(x(ms), y(value));
        else context.lineTo(x(ms), y(value));
      });
      if (visible.length === 1) {
        // A lone sample is a short tick so it stays visible.
        context.lineTo(x(visible[0][0]) + 2, y(visible[0][1]));
      }
      if (series.style === 'area') {
        context.save();
        context.lineTo(x(visible[visible.length - 1][0]), y(0));
        context.lineTo(x(visible[0][0]), y(0));
        context.closePath();
        context.globalAlpha = 0.2;
        context.fillStyle = color;
        context.fill();
        context.restore();
        context.beginPath();
        visible.forEach(([ms, value], index) => {
          if (index === 0) context.moveTo(x(ms), y(value));
          else context.lineTo(x(ms), y(value));
        });
      }
      context.strokeStyle = color;
      context.globalAlpha = series.style === 'area' ? 1 : 0.85;
      context.lineWidth = series.style === 'area' ? 1.4 : 1.1;
      context.setLineDash(series.key === 'cpuMax' ? [3, 3] : []);
      context.stroke();
      context.setLineDash([]);
      context.globalAlpha = 1;
    }
  }

  if (props.hoverMs != null) {
    const hoverX = Math.round(x(props.hoverMs)) + 0.5;
    if (hoverX >= 0 && hoverX <= width) {
      context.strokeStyle = palette.text;
      context.globalAlpha = 0.35;
      context.beginPath();
      context.moveTo(hoverX, 0);
      context.lineTo(hoverX, height);
      context.stroke();
      context.globalAlpha = 1;
    }
  }
}

function handlePointerMove(event) {
  const width = size?.width ?? 0;
  if (!width) return;
  const { startMs, spanMs } = props.viewport;
  emit('hover', startMs + (event.offsetX / width) * spanMs);
}

function readPalette(surface) {
  const styles = getComputedStyle(surface);
  const color = (name, fallback) => styles.getPropertyValue(name).trim() || fallback;
  return {
    colors: {
      '--trace-flame-res-cpu': color('--trace-flame-res-cpu', '#0f9f8f'),
      '--trace-flame-res-cpu-max': color('--trace-flame-res-cpu-max', '#0f9f8f'),
      '--trace-flame-res-memory': color('--trace-flame-res-memory', '#c65746'),
      '--trace-flame-res-memory-anon': color('--trace-flame-res-memory-anon', '#8b2f22'),
      '--trace-flame-res-io-read': color('--trace-flame-res-io-read', '#6366f1'),
      '--trace-flame-res-io-write': color('--trace-flame-res-io-write', '#d97706'),
      '--trace-flame-res-pids': color('--trace-flame-res-pids', '#64748b'),
      '--trace-flame-res-host': color('--trace-flame-res-host', '#64748b'),
    },
    grid: color('--trace-flame-grid', 'rgba(100, 116, 139, 0.14)'),
    muted: color('--ui-muted', '#64748b'),
    text: color('--ui-text', '#111827'),
  };
}

function createGapPattern(context, color) {
  const tile = document.createElement('canvas');
  tile.width = 6;
  tile.height = 6;
  const tileContext = tile.getContext('2d');
  if (!tileContext) return 'transparent';
  tileContext.strokeStyle = color;
  tileContext.globalAlpha = 0.18;
  tileContext.beginPath();
  tileContext.moveTo(0, 6);
  tileContext.lineTo(6, 0);
  tileContext.stroke();
  return context.createPattern(tile, 'repeat') ?? 'transparent';
}
</script>
