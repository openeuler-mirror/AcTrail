<template>
  <div class="pulse-curve" :style="{ color }">
    <div class="pulse-curve-plot">
      <div class="pulse-curve-bars" aria-hidden="true">
        <span
          v-for="(value, index) in values"
          :key="`${index}-${value}`"
          class="pulse-curve-bar-slot"
        >
          <span class="pulse-curve-bar" :style="{ height: `${barHeight(value)}%` }" />
        </span>
      </div>
      <svg
        class="pulse-curve-canvas"
        :viewBox="`0 0 ${VIEW_WIDTH} ${VIEW_HEIGHT}`"
        preserveAspectRatio="none"
        role="img"
        :aria-label="ariaLabel"
      >
        <defs>
          <linearGradient :id="gradientId" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="currentColor" stop-opacity="0.16" />
            <stop offset="100%" stop-color="currentColor" stop-opacity="0" />
          </linearGradient>
        </defs>
        <path v-if="!flat" class="pulse-curve-area" :d="areaPath" :fill="`url(#${gradientId})`" />
        <path class="pulse-curve-line" :class="{ 'is-flat': flat }" :d="linePath" />
      </svg>
      <span
        v-if="!flat"
        class="pulse-curve-head"
        :style="{ left: `${headLeft}%`, top: `${headTop}%` }"
        aria-hidden="true"
      />
      <div v-if="labels.length" class="pulse-curve-hits" @mouseleave="hovered = null">
        <span
          v-for="(label, index) in labels"
          :key="`${index}-${label}`"
          class="pulse-curve-hit"
          :class="{ 'is-hovered': hovered === index }"
          :title="`${label} · ${formatValue(values[index])}`"
          @mouseenter="hovered = index"
        />
      </div>
    </div>
    <div v-if="labels.length > 1" class="pulse-curve-axis" aria-hidden="true">
      <span>{{ labels[0] }}</span>
      <span>{{ labels[labels.length - 1] }}</span>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, useId } from 'vue';

const VIEW_WIDTH = 100;
const VIEW_HEIGHT = 40;
/** Headroom so a peak never clips against the top edge. */
const TOP_PADDING = 6;
/** Bars stop where the curve peaks, so both read off one scale. */
const PEAK_HEIGHT_PERCENT = ((VIEW_HEIGHT - TOP_PADDING) / VIEW_HEIGHT) * 100;

const props = defineProps({
  points: {
    type: Array,
    default: () => [],
  },
  labels: {
    type: Array,
    default: () => [],
  },
  color: {
    type: String,
    default: 'var(--ui-accent-strong)',
  },
  ariaLabel: {
    type: String,
    default: '',
  },
  formatValue: {
    type: Function,
    default: (value) => Number(value ?? 0).toLocaleString(),
  },
});

const gradientId = `pulse-curve-${useId()}`;
const hovered = ref(null);

const values = computed(() =>
  props.points.map((value) => {
    const numeric = Number(value);
    return Number.isFinite(numeric) ? numeric : 0;
  }),
);

const peak = computed(() => values.value.reduce((max, value) => Math.max(max, value), 0));
/** A flat series keeps the baseline instead of inventing a shape. */
const flat = computed(() => values.value.length < 2 || peak.value <= 0);

const coordinates = computed(() => {
  const count = values.value.length;
  const usableHeight = VIEW_HEIGHT - TOP_PADDING;
  // Points sit at bucket centres so the curve reads through the bar tops.
  const stepX = count > 0 ? VIEW_WIDTH / count : 0;
  return values.value.map((value, index) => ({
    x: (index + 0.5) * stepX,
    y: VIEW_HEIGHT - (peak.value > 0 ? value / peak.value : 0) * usableHeight,
  }));
});

function barHeight(value) {
  if (peak.value <= 0) {
    return 0;
  }
  return (value / peak.value) * PEAK_HEIGHT_PERCENT;
}

const line = computed(() =>
  coordinates.value.length < 2 ? '' : smoothLinePath(coordinates.value),
);

const linePath = computed(() =>
  flat.value ? `M0 ${VIEW_HEIGHT - 1} L${VIEW_WIDTH} ${VIEW_HEIGHT - 1}` : line.value,
);

const areaPath = computed(() => {
  if (flat.value) {
    return '';
  }
  const points = coordinates.value;
  const first = points[0];
  const last = points[points.length - 1];
  return `${line.value} L${last.x.toFixed(2)} ${VIEW_HEIGHT} L${first.x.toFixed(2)} ${VIEW_HEIGHT} Z`;
});

const headTop = computed(() => {
  const points = coordinates.value;
  const last = points[points.length - 1];
  return last ? (last.y / VIEW_HEIGHT) * 100 : 0;
});

const headLeft = computed(() => {
  const points = coordinates.value;
  const last = points[points.length - 1];
  return last ? (last.x / VIEW_WIDTH) * 100 : 0;
});

/**
 * Catmull-Rom through every point, emitted as cubic Béziers. Control points are
 * clamped vertically so a spike cannot overshoot the plot edges.
 */
function smoothLinePath(points) {
  let path = `M${points[0].x.toFixed(2)} ${points[0].y.toFixed(2)}`;
  for (let index = 0; index < points.length - 1; index += 1) {
    const p0 = points[Math.max(0, index - 1)];
    const p1 = points[index];
    const p2 = points[index + 1];
    const p3 = points[Math.min(points.length - 1, index + 2)];
    const cp1x = p1.x + (p2.x - p0.x) / 6;
    const cp1y = clampY(p1.y + (p2.y - p0.y) / 6);
    const cp2x = p2.x - (p3.x - p1.x) / 6;
    const cp2y = clampY(p2.y - (p3.y - p1.y) / 6);
    path += ` C${cp1x.toFixed(2)} ${cp1y.toFixed(2)}, ${cp2x.toFixed(2)} ${cp2y.toFixed(2)}, ${p2.x.toFixed(2)} ${p2.y.toFixed(2)}`;
  }
  return path;
}

function clampY(value) {
  return Math.max(TOP_PADDING, Math.min(VIEW_HEIGHT, value));
}
</script>

<style scoped>
.pulse-curve {
  display: grid;
  gap: 8px;
  min-width: 0;
}

.pulse-curve-plot {
  position: relative;
  height: var(--pulse-curve-height, 104px);
  /* A left-to-right wipe keeps the stroke continuous; a dash draw has to guess
     the path length and can end up rendering as loose segments. */
  animation: pulse-curve-sweep 1.2s cubic-bezier(0.25, 0.6, 0.35, 1) 0.1s backwards;
}

.pulse-curve-canvas {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
}

.pulse-curve-bars {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: flex-end;
  border-bottom: 1px solid color-mix(in srgb, currentColor 18%, transparent);
}

.pulse-curve-bar-slot {
  box-sizing: border-box;
  flex: 1 1 0;
  min-width: 0;
  height: 100%;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding-inline: 1px;
}

.pulse-curve-bar {
  width: 100%;
  max-width: 48px;
  border-radius: 2px 2px 0 0;
  background: color-mix(in srgb, currentColor 30%, transparent);
}

.pulse-curve-line {
  fill: none;
  stroke: currentColor;
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
  vector-effect: non-scaling-stroke;
}

.pulse-curve-line.is-flat {
  stroke: color-mix(in srgb, currentColor 45%, transparent);
  stroke-width: 1.5;
  stroke-dasharray: 3 4;
}

.pulse-curve-head {
  position: absolute;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: currentColor;
  box-shadow: 0 0 0 3px color-mix(in srgb, currentColor 20%, transparent);
  transform: translate(-50%, -50%);
  transition:
    top 400ms var(--ui-ease-out),
    left 400ms var(--ui-ease-out);
  animation: pulse-curve-breath 2.4s ease-out 1.2s infinite;
}

.pulse-curve-hits {
  position: absolute;
  inset: 0;
  display: flex;
}

.pulse-curve-hit {
  flex: 1 1 0;
  min-width: 0;
}

.pulse-curve-hit.is-hovered {
  background: color-mix(in srgb, currentColor 9%, transparent);
}

.pulse-curve-axis {
  display: flex;
  justify-content: space-between;
  color: var(--ui-muted);
  font-family: var(--ui-value-font);
  font-size: var(--ui-font-xs);
}

@keyframes pulse-curve-sweep {
  from {
    clip-path: inset(0 100% 0 0);
  }

  to {
    clip-path: inset(0 0 0 0);
  }
}

@keyframes pulse-curve-breath {
  0% {
    box-shadow: 0 0 0 3px color-mix(in srgb, currentColor 28%, transparent);
  }

  70%,
  100% {
    box-shadow: 0 0 0 8px transparent;
  }
}

@media (prefers-reduced-motion: reduce) {
  .pulse-curve-plot {
    animation: none;
  }

  .pulse-curve-head {
    transition: none;
  }
}
</style>
