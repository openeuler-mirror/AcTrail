<template>
  <div class="attribution-bar" role="img" :aria-label="ariaLabel">
    <button
      v-for="category in visibleCategories"
      :key="category.key"
      class="attribution-bar-segment"
      :class="`category-${category.key}`"
      type="button"
      :style="segmentStyle(category)"
      :title="segmentTitle(category)"
      @click="$emit('select', category)"
    >
      <span v-if="Number(category.percentage_bps) >= 700">
        {{ formatAttributionPercent(category.percentage_bps) }}
      </span>
    </button>
  </div>
</template>

<script setup>
import { computed } from 'vue';

import {
  ATTRIBUTION_COLORS,
  formatAttributionDuration,
  formatAttributionPercent,
} from './model';

const props = defineProps({
  categories: {
    type: Array,
    default: () => [],
  },
});

defineEmits(['select']);

const visibleCategories = computed(() =>
  props.categories.filter((category) => Number(category.percentage_bps ?? 0) > 0),
);
const ariaLabel = computed(() =>
  props.categories
    .map((category) => `${category.label} ${formatAttributionPercent(category.percentage_bps)}`)
    .join(', '),
);

function segmentStyle(category) {
  return {
    width: `${Number(category.percentage_bps ?? 0) / 100}%`,
    background: ATTRIBUTION_COLORS[category.key] ?? 'var(--stats-muted)',
  };
}

function segmentTitle(category) {
  return `${category.label}: ${formatAttributionDuration(category.duration_nanos)} (${formatAttributionPercent(category.percentage_bps)})`;
}
</script>

<style scoped>
.attribution-bar {
  width: 100%;
  min-height: 42px;
  display: flex;
  overflow: hidden;
  border: 1px solid var(--stats-border, var(--border));
  border-radius: var(--stats-radius-md, 10px);
  background: var(--stats-surface, var(--surface));
}

.attribution-bar-segment {
  min-width: 2px;
  min-height: 42px;
  display: grid;
  place-items: center;
  padding: 0;
  border: 0;
  color: #fff;
  cursor: pointer;
  font: inherit;
  font-size: var(--stats-font-xs, 12px);
  font-weight: 600;
  text-shadow: 0 1px 2px rgb(0 0 0 / 35%);
  transition: filter 120ms ease;
}

.attribution-bar-segment:hover {
  filter: brightness(1.12);
}

.attribution-bar-segment:focus-visible {
  position: relative;
  z-index: 1;
  outline: 2px solid var(--stats-text, #fff);
  outline-offset: -3px;
}
</style>
