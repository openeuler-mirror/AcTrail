<template>
  <div class="timeline-overview">
    <div class="timeline-overview-gutter">
      <strong>{{ label }}</strong>
      <small>{{ hint }}</small>
    </div>
    <div
      ref="track"
      class="timeline-overview-track"
      title="Drag the window to pan; drag either edge to resize; scroll to zoom"
      @pointerdown="startDrag"
      @wheel.prevent="handleWheel"
    >
      <canvas ref="canvas" aria-hidden="true"></canvas>
      <span class="timeline-overview-shade shade-left" :style="leftShadeStyle"></span>
      <span class="timeline-overview-shade shade-right" :style="rightShadeStyle"></span>
      <span class="timeline-overview-window" :style="windowStyle">
        <i class="timeline-overview-handle handle-left" data-resize="left"></i>
        <i class="timeline-overview-handle handle-right" data-resize="right"></i>
      </span>
      <span class="timeline-overview-time time-start">{{ startLabel }}</span>
      <span class="timeline-overview-time time-end">{{ endLabel }}</span>
    </div>
  </div>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import {
  constrainTimeViewport,
  wheelZoomFactor,
} from '../../tabs/core/time-navigation/model.js';

const props = defineProps({
  bounds: {
    type: Object,
    required: true,
  },
  viewport: {
    type: Object,
    required: true,
  },
  lanes: {
    type: Array,
    default: () => [],
  },
  label: {
    type: String,
    default: 'Trace overview',
  },
  hint: {
    type: String,
    default: 'W/S zoom · A/D pan',
  },
  startLabel: {
    type: String,
    default: '0ms',
  },
  endLabel: {
    type: String,
    default: '',
  },
});

const emit = defineEmits(['update:viewport', 'zoom']);
const track = ref(null);
const canvas = ref(null);
let resizeObserver = null;
let themeObserver = null;
let dragState = null;
let dragFrame = null;

const viewportRatios = computed(() => {
  const boundsStart = Number(props.bounds.startMs) || 0;
  const boundsSpan = Math.max(Number(props.bounds.spanMs) || 1, 0.001);
  const current = constrainTimeViewport(props.viewport, props.bounds);
  return {
    left: ((current.startMs - boundsStart) / boundsSpan) * 100,
    width: (current.spanMs / boundsSpan) * 100,
  };
});
const windowStyle = computed(() => ({
  left: `${viewportRatios.value.left}%`,
  width: `${viewportRatios.value.width}%`,
}));
const leftShadeStyle = computed(() => ({ width: `${viewportRatios.value.left}%` }));
const rightShadeStyle = computed(() => ({
  left: `${viewportRatios.value.left + viewportRatios.value.width}%`,
}));

onMounted(() => {
  resizeObserver = new ResizeObserver(draw);
  resizeObserver.observe(track.value);
  themeObserver = new MutationObserver(draw);
  themeObserver.observe(track.value.closest('.stats-theme') ?? document.documentElement, {
    attributes: true,
    attributeFilter: ['class', 'data-theme'],
  });
  nextTick(draw);
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  themeObserver?.disconnect();
  stopDrag();
});

watch(() => props.lanes, () => nextTick(draw));

function draw() {
  const element = track.value;
  const surface = canvas.value;
  if (!element || !surface) {
    return;
  }
  const rect = element.getBoundingClientRect();
  if (!rect.width || !rect.height) {
    return;
  }
  const ratio = Math.min(globalThis.devicePixelRatio || 1, 2);
  surface.width = Math.round(rect.width * ratio);
  surface.height = Math.round(rect.height * ratio);
  const context = surface.getContext('2d');
  context.scale(ratio, ratio);
  context.clearRect(0, 0, rect.width, rect.height);

  const styles = getComputedStyle(element);
  const grid = cssColor(styles, '--trace-flame-grid', 'rgba(90, 104, 128, 0.18)');
  const error = cssColor(styles, '--trace-status-error', '#ff4d6d');
  const boundsStart = Number(props.bounds.startMs) || 0;
  const boundsSpan = Math.max(Number(props.bounds.spanMs) || 1, 0.001);
  const chartHeight = Math.max(rect.height - 17, 1);
  const laneHeight = chartHeight / Math.max(props.lanes.length, 1);

  context.strokeStyle = grid;
  context.lineWidth = 1;
  for (let index = 1; index < 6; index += 1) {
    const x = Math.round((rect.width * index) / 6) + 0.5;
    context.beginPath();
    context.moveTo(x, 0);
    context.lineTo(x, chartHeight);
    context.stroke();
  }

  props.lanes.forEach((lane, laneIndex) => {
    const color = lane.tone === 'harness'
      ? cssColor(styles, '--trace-flame-harness-accent', '#dc8500')
      : cssColor(styles, '--trace-flame-agent-accent', '#3271e8');
    const slotHeight = Math.max((laneHeight - 6) / 4, 1);
    for (const interval of lane.intervals ?? []) {
      const start = Number(interval.startOffsetMs);
      const end = start + Math.max(Number(interval.durMs) || 0, boundsSpan / rect.width);
      const left = ((start - boundsStart) / boundsSpan) * rect.width;
      const right = ((end - boundsStart) / boundsSpan) * rect.width;
      if (right < 0 || left > rect.width) {
        continue;
      }
      const depth = Math.abs(Number(interval.depth) || 0) % 4;
      const y = laneIndex * laneHeight + 3 + depth * slotHeight;
      context.globalAlpha = interval.aggregate ? 0.55 : 0.82;
      context.fillStyle = interval.status === 'error' ? error : color;
      context.fillRect(
        Math.max(left, 0),
        y,
        Math.max(Math.min(right, rect.width) - Math.max(left, 0), 1),
        Math.max(slotHeight - 1, 1),
      );
    }
  });
  context.globalAlpha = 1;
}

function handleWheel(event) {
  const rect = track.value?.getBoundingClientRect();
  if (!rect?.width) {
    return;
  }
  emit('zoom', {
    factor: wheelZoomFactor(event.deltaY),
    anchor: Math.min(Math.max((event.clientX - rect.left) / rect.width, 0), 1),
  });
}

function startDrag(event) {
  if (event.button !== 0) {
    return;
  }
  const rect = track.value?.getBoundingClientRect();
  if (!rect?.width) {
    return;
  }
  event.preventDefault();
  const bounds = props.bounds;
  let viewport = constrainTimeViewport(props.viewport, bounds);
  let mode = event.target.dataset.resize ?? 'pan';
  if (!event.target.closest('.timeline-overview-window')) {
    const ratio = Math.min(Math.max((event.clientX - rect.left) / rect.width, 0), 1);
    viewport = constrainTimeViewport({
      startMs: (Number(bounds.startMs) || 0)
        + (Number(bounds.spanMs) || 1) * ratio
        - viewport.spanMs / 2,
      spanMs: viewport.spanMs,
    }, bounds);
    emit('update:viewport', viewport);
    mode = 'pan';
  }
  dragState = {
    pointerId: event.pointerId,
    clientX: event.clientX,
    startX: event.clientX,
    trackWidth: rect.width,
    viewport,
    mode,
  };
  globalThis.addEventListener('pointermove', moveDrag);
  globalThis.addEventListener('pointerup', stopDrag);
  globalThis.addEventListener('pointercancel', stopDrag);
}

function moveDrag(event) {
  if (!dragState || event.pointerId !== dragState.pointerId) {
    return;
  }
  dragState.clientX = event.clientX;
  if (dragFrame !== null) {
    return;
  }
  dragFrame = requestAnimationFrame(commitDrag);
}

function commitDrag() {
  dragFrame = null;
  if (!dragState) {
    return;
  }
  const boundsSpan = Math.max(Number(props.bounds.spanMs) || 1, 0.001);
  const deltaMs = (
    (dragState.clientX - dragState.startX)
    / dragState.trackWidth
  ) * boundsSpan;
  const original = dragState.viewport;
  if (dragState.mode === 'left') {
    emit('update:viewport', constrainTimeViewport({
      startMs: original.startMs + deltaMs,
      spanMs: original.spanMs - deltaMs,
    }, props.bounds));
    return;
  }
  if (dragState.mode === 'right') {
    emit('update:viewport', constrainTimeViewport({
      startMs: original.startMs,
      spanMs: original.spanMs + deltaMs,
    }, props.bounds));
    return;
  }
  emit('update:viewport', constrainTimeViewport({
    startMs: original.startMs + deltaMs,
    spanMs: original.spanMs,
  }, props.bounds));
}

function stopDrag() {
  dragState = null;
  if (dragFrame !== null) {
    cancelAnimationFrame(dragFrame);
    dragFrame = null;
  }
  globalThis.removeEventListener('pointermove', moveDrag);
  globalThis.removeEventListener('pointerup', stopDrag);
  globalThis.removeEventListener('pointercancel', stopDrag);
}

function cssColor(styles, name, fallback) {
  return styles.getPropertyValue(name).trim() || fallback;
}
</script>

<style scoped>
.timeline-overview {
  display: grid;
  grid-template-columns: var(--timeline-overview-gutter, 156px) minmax(620px, 1fr);
  min-height: 72px;
  border-bottom: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface-soft, var(--surface)) 66%, var(--surface));
}

.timeline-overview-gutter {
  display: flex;
  min-width: 0;
  flex-direction: column;
  justify-content: center;
  gap: 3px;
  padding: 0 12px 0 18px;
  border-right: 1px solid var(--border);
}

.timeline-overview-gutter strong {
  color: var(--text);
  font-size: 11px;
  letter-spacing: 0.02em;
  text-transform: uppercase;
}

.timeline-overview-gutter small {
  color: var(--muted);
  font-size: 10px;
}

.timeline-overview-track {
  position: relative;
  min-width: 0;
  overflow: hidden;
  cursor: crosshair;
  touch-action: none;
}

.timeline-overview-track canvas {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
}

.timeline-overview-shade {
  position: absolute;
  z-index: 1;
  top: 0;
  bottom: 16px;
  background: color-mix(in srgb, var(--surface) 66%, transparent);
  pointer-events: none;
}

.shade-left {
  left: 0;
}

.shade-right {
  right: 0;
}

.timeline-overview-window {
  position: absolute;
  z-index: 2;
  top: 2px;
  bottom: 17px;
  min-width: 4px;
  border: 1px solid color-mix(in srgb, var(--trace-flame-agent-accent) 82%, white);
  border-radius: 3px;
  background: color-mix(in srgb, var(--trace-flame-agent-accent) 6%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, white 28%, transparent);
  cursor: grab;
}

.timeline-overview-window:active {
  cursor: grabbing;
}

.timeline-overview-handle {
  position: absolute;
  z-index: 3;
  top: -1px;
  bottom: -1px;
  width: 7px;
  background: color-mix(in srgb, var(--trace-flame-agent-accent) 72%, transparent);
  cursor: ew-resize;
}

.handle-left {
  left: -3px;
  border-radius: 3px 0 0 3px;
}

.handle-right {
  right: -3px;
  border-radius: 0 3px 3px 0;
}

.timeline-overview-time {
  position: absolute;
  z-index: 4;
  bottom: 2px;
  color: var(--muted);
  font-size: 9px;
  font-variant-numeric: tabular-nums;
  pointer-events: none;
}

.time-start {
  left: 5px;
}

.time-end {
  right: 5px;
}
</style>
