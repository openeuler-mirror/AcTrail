<template>
  <canvas
    ref="canvas"
    class="flame-track-canvas"
    role="img"
    :aria-label="`${track.label}: ${track.activities.length} timeline activities`"
    @pointerdown="handlePointerDown"
    @pointermove="handlePointerMove"
    @pointerleave="handlePointerLeave"
    @click="handleClick"
    @dblclick="handleDoubleClick"
  ></canvas>
</template>

<script setup>
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';

import {
  buildFlameTrackFrame,
  flameActivityPaletteKey,
  flameActivityTitle,
  hitTestFlameTrack,
} from './canvas-renderer.js';
import {
  cancelFlameFrame,
  registerFlameDraw,
  scheduleFlameFrame,
} from './frame-scheduler.js';

const props = defineProps({
  track: {
    type: Object,
    required: true,
  },
  viewport: {
    type: Object,
    required: true,
  },
  selectedId: {
    type: [String, Number],
    default: null,
  },
});

const emit = defineEmits(['select', 'focus']);
const canvas = ref(null);
let resizeObserver = null;
let intersectionObserver = null;
let themeObserver = null;
let unregisterDraw = null;
let visible = typeof IntersectionObserver !== 'function';
let entries = [];
let hoveredActivity = null;
let pointerDown = null;
let palette = null;
let hatchPatterns = null;
let canvasSize = null;

watch(
  () => [props.track, props.viewport.startMs, props.viewport.spanMs, props.selectedId],
  scheduleDraw,
);

onMounted(() => {
  unregisterDraw = registerFlameDraw(draw);
  resizeObserver = new ResizeObserver(([entry]) => {
    const width = entry?.contentRect.width ?? 0;
    const height = entry?.contentRect.height ?? 0;
    if (canvasSize?.width === width && canvasSize?.height === height) {
      return;
    }
    canvasSize = { width, height };
    scheduleDraw();
  });
  resizeObserver.observe(canvas.value);

  if (typeof IntersectionObserver === 'function') {
    intersectionObserver = new IntersectionObserver(([entry]) => {
      visible = entry?.isIntersecting ?? true;
      if (visible) {
        scheduleDraw();
      }
    }, { rootMargin: '240px 0px' });
    intersectionObserver.observe(canvas.value);
  }

  themeObserver = new MutationObserver(() => {
    palette = null;
    hatchPatterns = null;
    scheduleDraw();
  });
  themeObserver.observe(canvas.value.closest('.stats-theme') ?? document.documentElement, {
    attributes: true,
    attributeFilter: ['class', 'data-theme'],
  });
  scheduleDraw();
});

onBeforeUnmount(() => {
  unregisterDraw?.();
  cancelFlameFrame(draw);
  resizeObserver?.disconnect();
  intersectionObserver?.disconnect();
  themeObserver?.disconnect();
});

function scheduleDraw() {
  if (!visible) {
    return;
  }
  scheduleFlameFrame(draw);
}

function draw() {
  const surface = canvas.value;
  if (!surface || !visible) {
    return;
  }
  const width = canvasSize?.width ?? 0;
  const height = canvasSize?.height ?? 0;
  if (!width || !height) {
    return;
  }

  const ratio = Math.min(globalThis.devicePixelRatio || 1, 2);
  const physicalWidth = Math.max(Math.round(width * ratio), 1);
  const physicalHeight = Math.max(Math.round(height * ratio), 1);
  if (surface.width !== physicalWidth || surface.height !== physicalHeight) {
    surface.width = physicalWidth;
    surface.height = physicalHeight;
  }

  const context = surface.getContext('2d');
  if (!context) {
    return;
  }
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, width, height);
  palette ??= readPalette(surface);
  hatchPatterns ??= createHatchPatterns(context, palette);
  entries = buildFlameTrackFrame(props.track, props.viewport, width);

  for (const entry of entries) {
    drawEntry(context, entry, width);
  }
}

function drawEntry(context, entry, surfaceWidth) {
  const activity = entry.activity;
  const left = Math.max(entry.left, -20);
  const right = Math.min(entry.left + entry.width, surfaceWidth + 20);
  const width = Math.max(right - left, 1);
  if (right <= 0 || left >= surfaceWidth) {
    return;
  }

  const colors = palette[flameActivityPaletteKey(activity)] ?? palette.agent;
  const radius = entry.instant || activity.density ? 2 : 5;
  const opacity = activity.density ? 0.72 : activityOpacity(activity);
  context.globalAlpha = opacity;
  roundedRect(context, left, entry.top, width, entry.height, radius);
  if (activity.synthetic) {
    const gradient = context.createLinearGradient(left, 0, left + width, 0);
    gradient.addColorStop(0, colors.fill);
    gradient.addColorStop(1, palette.syntheticEnd[activity.layer] ?? colors.fill);
    context.fillStyle = gradient;
  } else {
    context.fillStyle = colors.fill;
  }
  context.fill();

  context.globalAlpha = opacity * 0.34;
  context.strokeStyle = colors.text;
  context.lineWidth = 1;
  context.stroke();
  context.globalAlpha = 1;

  if (activity.aggregate && !activity.synthetic) {
    drawHatch(
      context,
      left,
      entry.top,
      width,
      entry.height,
      hatchPatterns.aggregate(colors.text),
    );
  }
  if (activity.status === 'error') {
    drawHatch(context, left, entry.top, width, entry.height, hatchPatterns.error);
  }

  const highlighted = props.selectedId === activity.id || hoveredActivity?.id === activity.id;
  if (highlighted && !activity.synthetic) {
    roundedRect(context, left + 1, entry.top + 1, Math.max(width - 2, 1), entry.height - 2, radius);
    context.strokeStyle = activity.status === 'error' ? palette.error : palette.highlight;
    context.globalAlpha = props.selectedId === activity.id ? 0.95 : 0.58;
    context.lineWidth = 2;
    context.stroke();
    context.globalAlpha = 1;
  }

  if ((entry.showLabel || activity.synthetic) && width >= 12) {
    context.save();
    context.beginPath();
    context.rect(Math.max(left, 0), entry.top, Math.min(width, surfaceWidth), entry.height);
    context.clip();
    context.fillStyle = colors.text;
    context.globalAlpha = activityOpacity(activity);
    context.font = palette.font;
    context.textAlign = 'left';
    context.textBaseline = 'middle';
    context.fillText(activity.label, Math.max(left, 0) + 8, entry.top + entry.height / 2 + 0.5);
    context.restore();
  }
}

function handlePointerDown(event) {
  const activity = activityAt(event);
  pointerDown = {
    activityId: activity?.id ?? null,
    clientX: event.clientX,
    clientY: event.clientY,
  };
  if (activity && !activity.synthetic) {
    event.stopPropagation();
  }
}

function handlePointerMove(event) {
  const activity = activityAt(event);
  if (activity?.id === hoveredActivity?.id) {
    return;
  }
  hoveredActivity = activity;
  const surface = canvas.value;
  if (surface) {
    surface.title = activity ? flameActivityTitle(activity) : '';
    surface.style.cursor = activity && !activity.synthetic ? 'pointer' : '';
  }
  scheduleDraw();
}

function handlePointerLeave() {
  pointerDown = null;
  if (!hoveredActivity) {
    return;
  }
  hoveredActivity = null;
  if (canvas.value) {
    canvas.value.title = '';
    canvas.value.style.cursor = '';
  }
  scheduleDraw();
}

function handleClick(event) {
  const activity = activityAt(event);
  const movement = pointerDown
    ? Math.hypot(event.clientX - pointerDown.clientX, event.clientY - pointerDown.clientY)
    : Number.POSITIVE_INFINITY;
  if (
    activity
    && !activity.synthetic
    && pointerDown?.activityId === activity.id
    && movement <= 4
  ) {
    emit('select', activity);
  }
  pointerDown = null;
}

function handleDoubleClick(event) {
  const activity = activityAt(event);
  if (activity && !activity.synthetic) {
    emit('focus', activity);
  }
}

function activityAt(event) {
  return hitTestFlameTrack(entries, event.offsetX, event.offsetY);
}

function readPalette(surface) {
  const styles = getComputedStyle(surface);
  const pair = (fillName, textName, fillFallback, textFallback) => ({
    fill: cssColor(styles, fillName, fillFallback),
    text: cssColor(styles, textName, textFallback),
  });
  return {
    agent: pair('--trace-flame-agent-bar', '--trace-flame-agent-bar-text', '#2563eb', '#ffffff'),
    harness: pair('--trace-flame-harness-bar', '--trace-flame-harness-bar-text', '#f59e0b', '#4b2b00'),
    tool: pair('--trace-flame-tool-bar', '--trace-flame-tool-bar-text', '#0f9f8f', '#ffffff'),
    file: pair('--trace-flame-file-bar', '--trace-flame-file-bar-text', '#38a3a5', '#ffffff'),
    llmCall: pair('--trace-flame-llm-call-bar', '--trace-flame-llm-call-bar-text', '#2563eb', '#ffffff'),
    llmRequest: pair('--trace-flame-llm-request-bar', '--trace-flame-llm-request-bar-text', '#4f46e5', '#ffffff'),
    llmResponse: pair('--trace-flame-llm-response-bar', '--trace-flame-llm-response-bar-text', '#60a5fa', '#102a56'),
    toolCall: pair('--trace-flame-tool-call-bar', '--trace-flame-tool-call-bar-text', '#0f9f8f', '#ffffff'),
    toolResult: pair('--trace-flame-tool-result-bar', '--trace-flame-tool-result-bar-text', '#5eead4', '#134e4a'),
    process: pair('--trace-flame-process-bar', '--trace-flame-process-bar-text', '#a855f7', '#ffffff'),
    protocol: pair('--trace-flame-protocol-bar', '--trace-flame-protocol-bar-text', '#64748b', '#ffffff'),
    error: cssColor(styles, '--trace-flame-error-hatch', '#ff4d6d'),
    highlight: cssColor(styles, '--text', '#111827'),
    syntheticEnd: {
      agent: cssColor(styles, '--trace-flame-agent-accent', '#3271e8'),
      harness: cssColor(styles, '--trace-flame-harness-accent', '#dc8500'),
    },
    font: `650 10px ${styles.fontFamily || 'sans-serif'}`,
  };
}

function createHatchPatterns(context, colors) {
  const aggregatePatterns = new Map();
  return {
    aggregate(color) {
      if (!aggregatePatterns.has(color)) {
        aggregatePatterns.set(color, createHatchPattern(context, color, 0.12, 8, 1));
      }
      return aggregatePatterns.get(color);
    },
    error: createHatchPattern(context, colors.error, 1, 7, 2),
  };
}

function createHatchPattern(context, color, alpha, gap, lineWidth) {
  const tile = document.createElement('canvas');
  tile.width = gap;
  tile.height = gap;
  const tileContext = tile.getContext('2d');
  if (!tileContext) {
    return color;
  }
  tileContext.strokeStyle = color;
  tileContext.globalAlpha = alpha;
  tileContext.lineWidth = lineWidth;
  tileContext.beginPath();
  tileContext.moveTo(-gap / 2, gap / 2);
  tileContext.lineTo(gap / 2, -gap / 2);
  tileContext.moveTo(0, gap);
  tileContext.lineTo(gap, 0);
  tileContext.moveTo(gap / 2, gap * 1.5);
  tileContext.lineTo(gap * 1.5, gap / 2);
  tileContext.stroke();
  return context.createPattern(tile, 'repeat') ?? color;
}

function drawHatch(context, left, top, width, height, pattern) {
  roundedRect(context, left, top, width, height, 4);
  context.fillStyle = pattern;
  context.fill();
}

function roundedRect(context, x, y, width, height, radius) {
  const clampedRadius = Math.min(radius, width / 2, height / 2);
  context.beginPath();
  if (typeof context.roundRect === 'function') {
    context.roundRect(x, y, width, height, clampedRadius);
    return;
  }
  context.moveTo(x + clampedRadius, y);
  context.lineTo(x + width - clampedRadius, y);
  context.arcTo(x + width, y, x + width, y + clampedRadius, clampedRadius);
  context.lineTo(x + width, y + height - clampedRadius);
  context.arcTo(x + width, y + height, x + width - clampedRadius, y + height, clampedRadius);
  context.lineTo(x + clampedRadius, y + height);
  context.arcTo(x, y + height, x, y + height - clampedRadius, clampedRadius);
  context.lineTo(x, y + clampedRadius);
  context.arcTo(x, y, x + clampedRadius, y, clampedRadius);
  context.closePath();
}

function activityOpacity(activity) {
  if (activity.group === 'runtime' || activity.group === 'protocol') {
    return 0.68;
  }
  return 1;
}

function cssColor(styles, name, fallback) {
  return styles.getPropertyValue(name).trim() || fallback;
}
</script>
