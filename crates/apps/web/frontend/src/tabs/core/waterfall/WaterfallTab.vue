<template>
  <section class="waterfall-panel">
    <div class="waterfall-toolbar">
      <span class="wf-count">
        {{ totalActions }} actions
        <template v-if="windowText"> · {{ windowText }}</template>
      </span>
      <div class="wf-actions">
        <button
          type="button"
          class="tree-action"
          :disabled="!hasTree || queryActive"
          @click="expandAll"
        >
          <ChevronsUpDown :size="15" aria-hidden="true" />
          Expand all
        </button>
        <button
          type="button"
          class="tree-action"
          :disabled="!hasTree || queryActive"
          @click="collapseAll"
        >
          <ChevronsDownUp :size="15" aria-hidden="true" />
          Collapse all
        </button>
      </div>
    </div>

    <div v-if="zoomLabel" class="waterfall-breadcrumb">
      <Search :size="14" aria-hidden="true" />
      <span class="wf-zoom-label">Zoomed: {{ zoomLabel }}</span>
      <button type="button" class="wf-zoom-reset" @click="resetZoom">Reset zoom</button>
    </div>

    <div v-if="groups.length" class="waterfall-legend">
      <button
        v-for="group in groups"
        :key="group.group"
        type="button"
        class="wf-chip"
        :class="[`wf-group-${group.group}`, { inactive: !isGroupActive(group.group) }]"
        @click="toggleGroup(group.group)"
      >
        <span class="wf-chip-dot"></span>
        {{ group.group }}
        <small>{{ group.count }}</small>
      </button>
    </div>

    <div v-if="rows.length" class="waterfall-scroll">
      <div class="waterfall-axis">
        <div class="wf-gutter">Action</div>
        <div class="wf-axis-track">
          <span v-for="tick in ticks" :key="tick.pct" class="wf-tick" :style="{ left: `${tick.pct}%` }">
            {{ tick.label }}
          </span>
        </div>
      </div>

      <div class="waterfall-rows">
        <div
          v-for="row in rows"
          :key="row.id"
          class="wf-row"
          :class="{ selected: row.id === selectedDetailId }"
          @click="select(row)"
          @dblclick="zoomTo(row)"
        >
          <div class="wf-label" :style="{ paddingLeft: `${row.depth * 16 + 10}px` }">
            <button
              v-if="row.hasChildren"
              type="button"
              class="wf-toggle"
              @click.stop="toggleRow(row)"
            >
              <ChevronDown v-if="row.expanded" :size="14" aria-hidden="true" />
              <ChevronRight v-else :size="14" aria-hidden="true" />
            </button>
            <span v-else class="wf-toggle-spacer"></span>
            <div class="wf-label-main">
              <div class="wf-label-line">
                <span class="wf-label-text" :title="row.label">{{ row.label }}</span>
                <span v-if="row.target" class="wf-label-target" :title="row.target">{{ row.target }}</span>
              </div>
              <div class="wf-label-meta">
                <span class="wf-meta-start" :title="`start +${formatOffset(row.startOffsetMs)}`">
                  {{ row.startClock || row.startOffsetLabel }}
                </span>
                <span class="wf-meta-sep">·</span>
                <span class="wf-meta-dur" :class="{ 'is-live': row.live }">{{ row.durationText }}</span>
              </div>
            </div>
            <button
              v-if="row.hasChildren"
              type="button"
              class="wf-zoom"
              title="Zoom to this subtree"
              @click.stop="zoomTo(row)"
            >
              <ZoomIn :size="13" aria-hidden="true" />
            </button>
          </div>
          <div class="wf-track">
            <div
              class="wf-bar"
              :class="[`wf-group-${row.kindGroup}`, `wf-status-${row.status}`, { live: row.live }]"
              :style="barStyle(row)"
              :title="barTitle(row)"
            >
              <span class="wf-bar-text">{{ barText(row) }}</span>
            </div>
          </div>
        </div>

        <div v-if="hasMoreRows" class="wf-load-more-row">
          <button type="button" class="wf-load-more" @click="loadMore">
            Load {{ nextBatchSize }} more ({{ remainingRows }} hidden)
          </button>
          <button type="button" class="wf-load-all" @click="loadAll">Load all</button>
        </div>
      </div>
    </div>

    <div v-else class="waterfall-empty">No actions to chart</div>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ChevronDown, ChevronRight, ChevronsDownUp, ChevronsUpDown, Search, ZoomIn } from '@lucide/vue';

import { TABLE_RENDER_LIMITS } from '../../tableConfig';
import { normalizeTableQuery } from '../../tableModel';
import {
  actionDetail,
  buildWaterfall,
  collectDefaultExpandedIds,
  collectParentIds,
  findWaterfallNode,
  flattenMatchingWaterfall,
  flattenVisibleWaterfall,
  formatOffset,
  subtreeWindow,
  windowLabel,
} from './model';

const props = defineProps({
  traceDetail: {
    type: Object,
    default: null,
  },
  actionTree: {
    type: Object,
    default: null,
  },
  waterfall: {
    type: Object,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
  selectedDetailId: {
    type: String,
    default: null,
  },
});

const emit = defineEmits(['select-detail']);

const expandedIds = ref(new Set());
const activeGroups = ref(new Set());
const zoomId = ref(null);
const visibleLimit = ref(TABLE_RENDER_LIMITS.initialRows);

const model = computed(() => buildWaterfall(props.waterfall?.actions, props.waterfall?.links));
const roots = computed(() => model.value.roots);
const groups = computed(() => model.value.groups);
const window = computed(() => model.value.window);
const totalActions = computed(() => model.value.totalActions);
const windowText = computed(() => windowLabel(window.value));
const parentIds = computed(() => collectParentIds(roots.value));
const hasTree = computed(() => parentIds.value.length > 0);
const normalizedQuery = computed(() => normalizeTableQuery(props.query));
const queryActive = computed(() => normalizedQuery.value.length > 0);

const zoomNode = computed(() =>
  zoomId.value ? findWaterfallNode(roots.value, zoomId.value) : null,
);
const zoomLabel = computed(() => {
  const node = zoomNode.value;
  if (!node) {
    return '';
  }
  return [node.label, node.target].filter(Boolean).join(' ');
});
const displayRoots = computed(() => (zoomNode.value ? [zoomNode.value] : roots.value));
const axisWindow = computed(() =>
  zoomNode.value
    ? subtreeWindow(zoomNode.value, window.value.spanMs)
    : { startMs: 0, spanMs: window.value.spanMs },
);

const ticks = computed(() => {
  const { startMs, spanMs } = axisWindow.value;
  return Array.from({ length: 5 }, (_, index) => {
    const fraction = index / 4;
    return { pct: fraction * 100, label: formatOffset(startMs + spanMs * fraction) };
  });
});

const allRows = computed(() =>
  queryActive.value
    ? flattenMatchingWaterfall(displayRoots.value, normalizedQuery.value, activeGroups.value)
    : flattenVisibleWaterfall(displayRoots.value, expandedIds.value, activeGroups.value),
);

const totalRows = computed(() => allRows.value.length);
const rows = computed(() => allRows.value.slice(0, visibleLimit.value));
const remainingRows = computed(() => Math.max(totalRows.value - rows.value.length, 0));
const nextBatchSize = computed(() => Math.min(TABLE_RENDER_LIMITS.rowBatchSize, remainingRows.value));
const hasMoreRows = computed(() => remainingRows.value > 0 && nextBatchSize.value > 0);

watch(
  () => props.waterfall,
  () => {
    expandedIds.value = new Set(collectDefaultExpandedIds(roots.value));
    activeGroups.value = new Set(groups.value.map((group) => group.group));
    zoomId.value = null;
  },
  { immediate: true },
);

watch([displayRoots, normalizedQuery, activeGroups], () => {
  visibleLimit.value = TABLE_RENDER_LIMITS.initialRows;
});

function barStyle(row) {
  const { startMs, spanMs } = axisWindow.value;
  const left = clampPct(((row.startOffsetMs - startMs) / spanMs) * 100);
  const endMs = row.live ? startMs + spanMs : row.startOffsetMs + (row.durMs ?? 0);
  const width = Math.max(((endMs - row.startOffsetMs) / spanMs) * 100, 0.5);
  return { left: `${left}%`, width: `${Math.min(width, 100 - left)}%` };
}

function barText(row) {
  if (row.live) {
    return 'running…';
  }
  if (row.durMs === null) {
    return '';
  }
  return row.durationLabel ?? formatOffset(row.durMs);
}

function barTitle(row) {
  const lines = [row.label];
  if (row.target) {
    lines.push(row.target);
  }
  lines.push(`start +${formatOffset(row.startOffsetMs)}`);
  for (const metric of row.metrics) {
    lines.push(`${metric.label}: ${metric.value}`);
  }
  lines.push(`status: ${row.status}`);
  return lines.join('\n');
}

function select(row) {
  emit('select-detail', actionDetail(row.action));
}

function isGroupActive(group) {
  return activeGroups.value.has(group);
}

function toggleGroup(group) {
  const next = new Set(activeGroups.value);
  if (next.has(group)) {
    next.delete(group);
  } else {
    next.add(group);
  }
  activeGroups.value = next;
}

function toggleRow(row) {
  const next = new Set(expandedIds.value);
  if (next.has(row.id)) {
    next.delete(row.id);
  } else {
    next.add(row.id);
  }
  expandedIds.value = next;
}

function expandAll() {
  expandedIds.value = new Set(parentIds.value);
}

function collapseAll() {
  expandedIds.value = new Set();
}

function loadMore() {
  visibleLimit.value += TABLE_RENDER_LIMITS.rowBatchSize;
}

function loadAll() {
  visibleLimit.value = totalRows.value;
}

function zoomTo(row) {
  if (!row.hasChildren) {
    return;
  }
  zoomId.value = row.id;
  const next = new Set(expandedIds.value);
  next.add(row.id);
  expandedIds.value = next;
}

function resetZoom() {
  zoomId.value = null;
}

function clampPct(value) {
  return Math.min(Math.max(value, 0), 100);
}
</script>

<style scoped>
.waterfall-panel {
  min-width: 0;
  min-height: 0;
  height: 100%;
  display: grid;
  grid-template-rows: auto auto minmax(0, 1fr);
  gap: 12px;
  padding: 18px;
  overflow: hidden;
}

.waterfall-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.wf-count {
  color: var(--muted);
  font-size: 12px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.wf-actions {
  display: flex;
  gap: 8px;
}

.tree-action {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  padding: 0 12px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface);
  color: var(--teal-deep);
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
  transition: border-color 0.12s ease, background-color 0.12s ease;
}

.tree-action:hover:not(:disabled) {
  border-color: var(--teal);
  background: #eef7f5;
}

.tree-action:disabled {
  color: var(--muted);
  cursor: not-allowed;
  opacity: 0.6;
}

.waterfall-breadcrumb {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface-muted);
  color: var(--teal-deep);
}

.wf-zoom-label {
  min-width: 0;
  overflow: hidden;
  font-size: 12px;
  font-weight: 700;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wf-zoom-reset {
  margin-left: auto;
  flex: 0 0 auto;
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--surface);
  color: var(--teal-deep);
  font-size: 11px;
  font-weight: 700;
  cursor: pointer;
}

.wf-zoom-reset:hover {
  border-color: var(--teal);
  background: #eef7f5;
}

.waterfall-legend {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.wf-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: 999px;
  background: var(--surface);
  color: var(--text);
  font-size: 11px;
  font-weight: 700;
  cursor: pointer;
}

.wf-chip small {
  color: var(--muted);
  font-weight: 600;
}

.wf-chip.inactive {
  opacity: 0.4;
}

.wf-chip-dot {
  width: 10px;
  height: 10px;
  border-radius: 3px;
  background: var(--wf-color, var(--muted));
}

.waterfall-scroll {
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--surface);
}

.waterfall-axis {
  position: sticky;
  top: 0;
  z-index: 3;
  display: grid;
  grid-template-columns: var(--wf-gutter-width) minmax(0, 1fr);
  align-items: center;
  border-bottom: 1px solid var(--border);
  background: var(--surface-muted);
}

.wf-gutter {
  padding: 8px 12px;
  color: var(--muted);
  font-size: 11px;
  font-weight: 800;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-right: 1px solid var(--border);
}

.wf-axis-track {
  position: relative;
  height: 32px;
}

.wf-tick {
  position: absolute;
  top: 9px;
  transform: translateX(-50%);
  padding: 0 4px;
  color: var(--muted);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.wf-tick:first-child {
  transform: translateX(0);
}

.wf-tick:last-child {
  transform: translateX(-100%);
}

.waterfall-rows {
  display: flex;
  flex-direction: column;
}

.wf-row {
  display: grid;
  grid-template-columns: var(--wf-gutter-width) minmax(0, 1fr);
  align-items: center;
  min-height: 40px;
  border-bottom: 1px solid var(--surface-muted);
  cursor: pointer;
}

.wf-row:hover {
  background: #f3faf8;
}

.wf-row.selected {
  background: #e7f5f1;
}

.wf-label {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding-right: 10px;
  border-right: 1px solid var(--border);
  height: 100%;
}

.wf-toggle {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  flex: 0 0 auto;
  padding: 0;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
}

.wf-toggle:hover {
  background: var(--surface-muted);
  color: var(--teal-deep);
}

.wf-toggle-spacer {
  width: 18px;
  flex: 0 0 auto;
}

.wf-label-main {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.wf-label-line {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}

.wf-label-text {
  flex: 0 1 auto;
  max-width: 170px;
  overflow: hidden;
  color: var(--teal-deep);
  font-size: 12px;
  font-weight: 700;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wf-label-target {
  min-width: 0;
  overflow: hidden;
  color: var(--muted);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wf-label-meta {
  display: flex;
  align-items: center;
  gap: 5px;
  color: var(--muted);
  font-size: 10px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.wf-meta-sep {
  opacity: 0.5;
}

.wf-meta-dur {
  color: var(--teal-deep);
  font-weight: 700;
}

.wf-meta-dur.is-live {
  color: var(--amber, #d97706);
}

.wf-zoom {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  flex: 0 0 auto;
  margin-left: auto;
  padding: 0;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--muted);
  cursor: pointer;
  opacity: 0.35;
}

.wf-row:hover .wf-zoom {
  opacity: 1;
}

.wf-zoom:hover {
  background: var(--surface-muted);
  color: var(--teal-deep);
}

.wf-track {
  position: relative;
  height: 100%;
  min-height: 40px;
}

.wf-bar {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  height: 16px;
  min-width: 2px;
  display: flex;
  align-items: center;
  padding: 0 6px;
  border-radius: 5px;
  background: var(--wf-color, var(--muted));
  overflow: hidden;
}

.wf-bar-text {
  color: #ffffff;
  font-size: 10px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
  text-shadow: 0 1px 1px rgba(0, 0, 0, 0.25);
}

.wf-bar.live {
  background-image: repeating-linear-gradient(
    45deg,
    rgba(255, 255, 255, 0.28) 0,
    rgba(255, 255, 255, 0.28) 4px,
    transparent 4px,
    transparent 8px
  );
}

.wf-bar.wf-status-error {
  outline: 1.5px solid var(--rose);
  outline-offset: -1.5px;
}

.wf-group-llm {
  --wf-color: #6366f1;
}

.wf-group-command {
  --wf-color: #d97706;
}

.wf-group-process {
  --wf-color: #b45309;
}

.wf-group-file {
  --wf-color: #0f766e;
}

.wf-group-sse {
  --wf-color: #2563eb;
}

.wf-group-http {
  --wf-color: #0891b2;
}

.wf-group-enforcement {
  --wf-color: #be123c;
}

.wf-group-other {
  --wf-color: #64748b;
}

.wf-load-more-row {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  margin: 10px;
}

.wf-load-more,
.wf-load-all {
  flex: 1 1 auto;
  height: 32px;
  border: 1px dashed var(--border);
  border-radius: 8px;
  background: var(--surface);
  color: var(--teal-deep);
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
}

.wf-load-more:hover,
.wf-load-all:hover {
  border-color: var(--teal);
  background: #eef7f5;
}

.waterfall-empty {
  display: grid;
  place-items: center;
  height: 100%;
  border: 1px dashed var(--border);
  border-radius: 12px;
  color: var(--muted);
  font-size: 13px;
  font-weight: 700;
}

.waterfall-panel {
  --wf-gutter-width: 300px;
}
</style>
