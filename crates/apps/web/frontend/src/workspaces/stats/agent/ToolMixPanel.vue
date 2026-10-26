<template>
  <div class="tool-mix">
    <div class="mix-bar" role="img" :aria-label="ariaLabel">
      <span
        v-for="tool in tools"
        :key="tool.key"
        class="mix-slice"
        :style="{ width: `${Math.max(tool.share * 100, 0.6)}%`, background: tool.color }"
        :title="`${tool.label} · ${formatCount(tool.callCount)}`"
      />
    </div>
    <ul class="mix-rows">
      <li v-for="tool in tools" :key="tool.key">
        <span class="mix-dot" :style="{ background: tool.color }" aria-hidden="true" />
        <span class="mix-name" :title="tool.label">{{ tool.label }}</span>
        <span class="mix-count">{{ formatCount(tool.callCount) }}</span>
        <span class="mix-share">{{ formatPercent(tool.share) }}</span>
        <span v-if="tool.averageDuration != null" class="mix-duration">
          {{ formatDurationNanos(tool.averageDuration) }}
        </span>
        <span v-else class="mix-duration" :title="unavailableHint">—</span>
      </li>
    </ul>
    <p class="mix-total">{{ totalLabel }}</p>
  </div>
</template>

<script setup>
import { formatDurationNanos } from './model';

defineProps({
  tools: { type: Array, default: () => [] },
  ariaLabel: { type: String, required: true },
  totalLabel: { type: String, required: true },
  unavailableHint: { type: String, required: true },
});

function formatCount(value) {
  return Number(value ?? 0).toLocaleString();
}

function formatPercent(share) {
  const percent = Number(share ?? 0) * 100;
  return percent >= 10 ? `${percent.toFixed(0)}%` : `${percent.toFixed(1)}%`;
}
</script>

<style scoped>
.tool-mix {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-sm);
}

.mix-bar {
  display: flex;
  height: 10px;
  overflow: hidden;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-surface-soft);
}

.mix-slice {
  min-width: 2px;
  height: 100%;
}

.mix-slice + .mix-slice {
  box-shadow: inset 1px 0 0 var(--ui-surface-strong);
}

.mix-rows {
  max-height: 210px;
  margin: 0;
  padding: 0;
  overflow-y: auto;
  list-style: none;
}

.mix-rows li {
  display: grid;
  grid-template-columns: 8px minmax(0, 1fr) 52px 48px 78px;
  align-items: center;
  gap: var(--ui-space-sm);
  padding: 6px 2px;
  border-bottom: 1px solid var(--ui-border);
}

.mix-rows li:last-child {
  border-bottom: 0;
}

.mix-dot {
  width: 8px;
  height: 8px;
  border-radius: 2px;
}

.mix-name {
  min-width: 0;
  overflow: hidden;
  font-family: var(--ui-mono);
  font-size: var(--ui-font-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.mix-count,
.mix-share,
.mix-duration {
  font-family: var(--ui-value-font);
  font-size: var(--ui-font-xs);
  font-variant-numeric: tabular-nums;
  text-align: right;
}

.mix-share,
.mix-duration {
  color: var(--ui-muted);
}

.mix-total {
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}
</style>
