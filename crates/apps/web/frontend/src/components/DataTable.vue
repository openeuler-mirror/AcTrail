<template>
  <div ref="scroller" class="data-table-shell" @scroll.passive="onScroll">
    <table v-if="rows.length" class="data-table">
      <thead>
        <tr>
          <th
            v-for="column in columns"
            :key="column.key"
            scope="col"
            :class="columnClass(column)"
          >
            {{ column.label }}
          </th>
        </tr>
      </thead>
      <tbody>
        <tr v-if="topSpacer" class="data-spacer" aria-hidden="true">
          <td :colspan="columns.length" :style="{ height: `${topSpacer}px` }" />
        </tr>
        <tr
          v-for="row in renderedRows"
          :key="row.id"
          class="data-row"
          :class="{ 'is-selected': selectedId === row.id }"
          :data-kind="rowKind(row)"
          tabindex="0"
          @click="select(row)"
          @keydown.enter.prevent="select(row)"
          @keydown.space.prevent="select(row)"
        >
          <td v-for="column in columns" :key="column.key" :class="columnClass(column)">
            <span class="cell-text">
              <span
                v-for="index in cellIndent(row.cells[column.key])"
                :key="index"
                class="tree-indent-unit"
                aria-hidden="true"
              />
              <template v-if="column.tree">
                <button
                  v-if="cellHasChildren(row.cells[column.key])"
                  type="button"
                  class="tree-toggle"
                  :aria-expanded="cellExpanded(row.cells[column.key])"
                  @click.stop="$emit('toggle', row)"
                  @keydown.enter.stop.prevent="$emit('toggle', row)"
                  @keydown.space.stop.prevent="$emit('toggle', row)"
                >
                  <ChevronDown v-if="cellExpanded(row.cells[column.key])" :size="14" />
                  <ChevronRight v-else :size="14" />
                </button>
                <span v-else class="tree-toggle-spacer" aria-hidden="true" />
                <span class="tree-label">{{ cellText(row.cells[column.key]) }}</span>
              </template>
              <span
                v-else-if="column.badge && hasCellText(row.cells[column.key])"
                class="cell-badge"
                :class="badgeClass(column, row.cells[column.key])"
              >
                {{ cellText(row.cells[column.key]) }}
              </span>
              <span
                v-else-if="!hasCellText(row.cells[column.key]) && !cellIndent(row.cells[column.key]).length"
                class="cell-empty"
                >—</span
              >
              <template v-else>
                <span class="cell-value" :title="cellTitle(row.cells[column.key])">
                  {{ cellText(row.cells[column.key]) }}
                </span>
              </template>
            </span>
          </td>
        </tr>
        <tr v-if="bottomSpacer" class="data-spacer" aria-hidden="true">
          <td :colspan="columns.length" :style="{ height: `${bottomSpacer}px` }" />
        </tr>
      </tbody>
    </table>
    <div v-if="hasMoreRows" class="table-more">
      <button class="load-more" type="button" @click="$emit('load-more')">
        {{ t('dataTable.loadMore', { count: nextBatchSize, hidden: remainingRows }) }}
      </button>
      <button v-if="canLoadAll" class="load-all" type="button" @click="$emit('load-all')">
        {{ t('dataTable.loadAll') }}
      </button>
    </div>
    <div v-if="!rows.length" class="empty-table">{{ emptyLabel || t('dataTable.empty') }}</div>
  </div>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { ChevronDown, ChevronRight } from '@lucide/vue';

import { useModuleLocale } from '../locale';
import strings from './locale';

const { t } = useModuleLocale(strings);

const props = defineProps({
  columns: {
    type: Array,
    required: true,
  },
  rows: {
    type: Array,
    required: true,
  },
  emptyLabel: {
    type: String,
    default: '',
  },
  totalRows: {
    type: Number,
    default: null,
  },
  nextBatchSize: {
    type: Number,
    default: 0,
  },
  canLoadMore: {
    type: Boolean,
    default: false,
  },
  canLoadAll: {
    type: Boolean,
    default: false,
  },
});

const emit = defineEmits(['select', 'load-more', 'load-all', 'toggle']);

const selectedId = ref(null);

const totalRowCount = computed(() =>
  Number.isInteger(props.totalRows) && props.totalRows >= 0 ? props.totalRows : props.rows.length,
);
const remainingRows = computed(() => Math.max(totalRowCount.value - props.rows.length, 0));
const nextBatchSize = computed(() => Math.min(positiveInteger(props.nextBatchSize), remainingRows.value));
const hasMoreRows = computed(() => props.canLoadMore && remainingRows.value > 0 && nextBatchSize.value > 0);

watch(
  () => props.rows,
  (rows) => {
    if (selectedId.value && !rows.some((row) => row.id === selectedId.value)) {
      selectedId.value = null;
    }
  },
);

function select(row) {
  selectedId.value = row.id;
  emit('select', row.detail);
}

function positiveInteger(value) {
  const number = Number(value);
  return Number.isInteger(number) && number > 0 ? number : 0;
}

function cellText(cell) {
  if (cell && typeof cell === 'object' && Object.prototype.hasOwnProperty.call(cell, 'text')) {
    return String(cell.text ?? '');
  }
  return String(cell ?? '');
}

function hasCellText(cell) {
  return cellText(cell).trim().length > 0;
}

function cellIndent(cell) {
  if (!cell || typeof cell !== 'object' || !cell.indent) {
    return [];
  }
  return Array.from({ length: cell.indent }, (_, index) => index);
}

function cellHasChildren(cell) {
  return Boolean(cell && typeof cell === 'object' && cell.hasChildren);
}

function cellExpanded(cell) {
  return Boolean(cell && typeof cell === 'object' && cell.expanded);
}

function columnClass(column) {
  return {
    'col-numeric': column.align === 'numeric',
    'col-right': column.align === 'right',
    'col-badge': Boolean(column.badge),
  };
}

function badgeClass(column, cell) {
  const slug = cellText(cell)
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return [`badge-${column.badge}`, slug ? `badge-${column.badge}-${slug}` : ''];
}

/**
 * Row kind drives the coloured rail. Models already carry it on the detail or
 * the kind cell, so the table does not need a per-view mapping.
 */
function rowKind(row) {
  const raw = row?.detail?.kind ?? row?.detail?.domain ?? row?.cells?.kind ?? '';
  return String(raw).trim().toLowerCase().replace(/\s+/g, '-');
}

/** Full text on hover once the visible cell had to be truncated. */
function cellTitle(cell) {
  const text = cellText(cell);
  return text.length > 48 ? text : '';
}

/*
 * Row windowing. Small ledgers render in full; once a ledger grows past the
 * threshold only the visible slice is mounted and spacer rows keep the
 * scrollbar honest. The row height is measured from the rendered table so the
 * window stays aligned with the real rhythm.
 */
const WINDOW_THRESHOLD = 400;
const OVERSCAN_ROWS = 12;
const FALLBACK_ROW_HEIGHT = 30;

const scroller = ref(null);
const scrollTop = ref(0);
const viewportHeight = ref(0);
const rowHeight = ref(FALLBACK_ROW_HEIGHT);
let measureFrame = null;

const windowingEnabled = computed(() => props.rows.length > WINDOW_THRESHOLD);
const windowRange = computed(() => {
  if (!windowingEnabled.value) {
    return { start: 0, end: props.rows.length };
  }
  const size = rowHeight.value || FALLBACK_ROW_HEIGHT;
  const height = viewportHeight.value || 600;
  return {
    start: Math.max(0, Math.floor(scrollTop.value / size) - OVERSCAN_ROWS),
    end: Math.min(props.rows.length, Math.ceil((scrollTop.value + height) / size) + OVERSCAN_ROWS),
  };
});
const renderedRows = computed(() =>
  (windowingEnabled.value ? props.rows.slice(windowRange.value.start, windowRange.value.end) : props.rows),
);
const topSpacer = computed(() => (windowingEnabled.value ? windowRange.value.start * rowHeight.value : 0));
const bottomSpacer = computed(() =>
  (windowingEnabled.value ? (props.rows.length - windowRange.value.end) * rowHeight.value : 0),
);

watch(() => props.rows, () => {
  scrollTop.value = scroller.value?.scrollTop ?? 0;
  scheduleMeasure();
}, { flush: 'post' });

onMounted(() => {
  measureViewport();
  window.addEventListener('resize', measureViewport);
  scheduleMeasure();
});

onBeforeUnmount(() => {
  window.removeEventListener('resize', measureViewport);
  if (measureFrame !== null) {
    window.cancelAnimationFrame(measureFrame);
  }
});

function onScroll(event) {
  if (!windowingEnabled.value) {
    return;
  }
  scrollTop.value = event.target.scrollTop;
}

function measureViewport() {
  viewportHeight.value = scroller.value?.clientHeight ?? 0;
}

function scheduleMeasure() {
  if (!windowingEnabled.value) {
    return;
  }
  if (measureFrame !== null) {
    window.cancelAnimationFrame(measureFrame);
  }
  measureFrame = window.requestAnimationFrame(async () => {
    measureFrame = null;
    await nextTick();
    measureViewport();
    const firstRow = scroller.value?.querySelector('.data-row');
    if (firstRow?.getBoundingClientRect().height) {
      rowHeight.value = firstRow.getBoundingClientRect().height;
    }
  });
}
</script>

<style scoped>
.data-table-shell {
  min-width: 0;
  height: 100%;
  overflow: auto;
  border: 1px solid var(--ui-border);
  border-radius: 12px;
  background: var(--ui-surface);
  box-shadow: var(--ui-shadow);
}

.data-table {
  width: 100%;
  min-width: 760px;
  border-collapse: separate;
  border-spacing: 0;
  font-size: 13px;
}

.data-table th,
.data-table td {
  padding: 6px 14px;
  border-bottom: 1px solid var(--ui-border);
  text-align: left;
  vertical-align: middle;
  line-height: 1.5;
}

.data-table tbody tr:last-child td {
  border-bottom: 0;
}

.data-spacer td {
  padding: 0;
  border: 0;
}

.data-table th {
  position: sticky;
  height: 30px;
  top: 0;
  z-index: 1;
  background: var(--trace-table-header-bg);
  color: var(--ui-muted);
  font-size: 12px;
  font-weight: 500;
  white-space: nowrap;
  box-shadow: inset 0 -1px 0 var(--ui-border);
  border-bottom: 0;
}

.col-numeric {
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.col-right {
  text-align: right;
}

.data-row {
  cursor: pointer;
  transition: background-color 0.12s ease;
}

.data-table tbody tr:nth-child(even) td {
  background: var(--trace-table-row-alt-bg);
}

.data-row:hover td,
.data-row:focus td {
  background: var(--trace-table-row-hover-bg);
}

.data-row.is-selected td {
  background: var(--trace-table-row-selected-bg);
  box-shadow: inset 2px 0 0 var(--ui-accent);
}

/*
 * Kind rail: the first cell carries a 2px stripe so a dense ledger reads by
 * category instead of by text alone.
 */
.data-row[data-kind] td:first-child {
  box-shadow: inset 2px 0 0 var(--row-kind-color, var(--ui-border-strong));
}

.data-row[data-kind='process'] {
  --row-kind-color: var(--wf-color-process, var(--ui-chart-4));
}

.data-row[data-kind='file'],
.data-row[data-kind='fs'],
.data-row[data-kind='filesystem'] {
  --row-kind-color: var(--wf-color-file, var(--ui-chart-8));
}

.data-row[data-kind='net'],
.data-row[data-kind='network'],
.data-row[data-kind='http'],
.data-row[data-kind='sse'] {
  --row-kind-color: var(--wf-color-http, var(--ui-chart-2));
}

.data-row[data-kind='llm'],
.data-row[data-kind='model'],
.data-row[data-kind='llm.call'] {
  --row-kind-color: var(--wf-color-llm, var(--ui-chart-3));
}

.data-row[data-kind='command'],
.data-row[data-kind='tool'],
.data-row[data-kind='payload'] {
  --row-kind-color: var(--wf-color-command, var(--ui-chart-4));
}

.data-row[data-kind='error'],
.data-row[data-kind='enforcement'],
.data-row[data-kind='critical'] {
  --row-kind-color: var(--wf-color-enforcement, var(--ui-chart-5));
}

.data-row:focus {
  outline: none;
}

.cell-text {
  display: inline-block;
  min-width: 0;
  overflow-wrap: anywhere;
}

.cell-value {
  display: block;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tree-indent-unit {
  display: inline-block;
  width: var(--table-indent-step);
}

.tree-toggle {
  display: inline-grid;
  place-items: center;
  width: 18px;
  height: 18px;
  margin-right: 6px;
  padding: 0;
  border: 1px solid var(--trace-table-toggle-border);
  border-radius: 5px;
  background: var(--trace-table-toggle-bg);
  color: var(--trace-interactive-text);
  vertical-align: -3px;
  cursor: pointer;
  transition: border-color 0.12s ease, background-color 0.12s ease;
}

.tree-toggle:hover {
  border-color: var(--ui-accent);
  background: var(--trace-table-toggle-hover-bg);
}

.tree-toggle-spacer {
  display: inline-block;
  width: 18px;
  margin-right: 6px;
}

.tree-label {
  overflow-wrap: anywhere;
}

.cell-empty {
  color: var(--trace-table-empty-text);
}

.cell-badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 9px;
  border: 1px solid transparent;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 700;
  line-height: 1.5;
  white-space: nowrap;
}

.badge-kind {
  border-color: var(--trace-badge-kind-border);
  background: var(--trace-badge-kind-bg);
  color: var(--trace-badge-kind-text);
  font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
  font-size: 11px;
}

.badge-status {
  border-color: var(--ui-border);
  background: var(--ui-surface-soft);
  color: var(--ui-muted);
  text-transform: capitalize;
}

.badge-status-success,
.badge-status-started {
  border-color: var(--trace-badge-success-border);
  background: var(--trace-badge-success-bg);
  color: var(--trace-badge-success-text);
}

.badge-status-error,
.badge-status-start-failed {
  border-color: var(--trace-badge-error-border);
  background: var(--trace-badge-error-bg);
  color: var(--trace-badge-error-text);
}

.badge-status-in-progress {
  border-color: var(--trace-badge-progress-border);
  background: var(--trace-badge-progress-bg);
  color: var(--trace-badge-progress-text);
}

.badge-status-unknown {
  border-color: var(--ui-border);
  background: var(--ui-surface-soft);
  color: var(--ui-muted);
}

.badge-duration {
  border-color: var(--trace-badge-duration-border);
  background: var(--trace-badge-duration-bg);
  color: var(--trace-badge-duration-text);
  font-variant-numeric: tabular-nums;
}

.empty-table {
  padding: 40px 18px;
  color: var(--ui-muted);
  text-align: center;
  font-weight: 600;
}

.table-more {
  display: flex;
  justify-content: center;
  flex-wrap: wrap;
  gap: 10px;
  padding: 14px;
  border-top: 1px solid var(--ui-border);
  background: var(--ui-surface);
}

.load-more,
.load-all {
  height: 34px;
  padding: 0 16px;
  border: 1px solid var(--trace-interactive-border);
  border-radius: 8px;
  background: var(--trace-interactive-bg);
  color: var(--trace-interactive-text);
  font-weight: 700;
  cursor: pointer;
  transition: border-color 0.12s ease, background-color 0.12s ease;
}

.load-all {
  border-style: dashed;
}

.load-more:hover,
.load-all:hover {
  border-color: var(--ui-accent);
  background: var(--trace-interactive-hover-bg);
}
</style>
