<template>
  <div class="data-table-shell">
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
        <tr
          v-for="row in rows"
          :key="row.id"
          class="data-row"
          :class="{ 'is-selected': selectedId === row.id }"
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
              <template v-else>{{ cellText(row.cells[column.key]) }}</template>
            </span>
          </td>
        </tr>
      </tbody>
    </table>
    <div v-if="hasMoreRows" class="table-more">
      <button class="load-more" type="button" @click="$emit('load-more')">
        Load {{ nextBatchSize }} more ({{ remainingRows }} hidden)
      </button>
      <button v-if="canLoadAll" class="load-all" type="button" @click="$emit('load-all')">
        Load all
      </button>
    </div>
    <div v-if="!rows.length" class="empty-table">{{ emptyLabel }}</div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ChevronDown, ChevronRight } from '@lucide/vue';

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
    default: 'No rows',
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
</script>

<style scoped>
.data-table-shell {
  min-width: 0;
  height: 100%;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--surface);
  box-shadow: var(--shadow);
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
  padding: 11px 16px;
  border-bottom: 1px solid var(--border);
  text-align: left;
  vertical-align: top;
}

.data-table tbody tr:last-child td {
  border-bottom: 0;
}

.data-table th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--trace-table-header-bg);
  color: var(--muted);
  font-size: 11px;
  font-weight: 800;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  white-space: nowrap;
  box-shadow: inset 0 -1px 0 var(--border);
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
  box-shadow: inset 2px 0 0 var(--teal);
}

.data-row:focus {
  outline: none;
}

.cell-text {
  display: inline-block;
  min-width: 0;
  overflow-wrap: anywhere;
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
  border-color: var(--teal);
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
  border-color: var(--border);
  background: var(--surface-muted);
  color: var(--muted);
  text-transform: capitalize;
}

.badge-status-success {
  border-color: var(--trace-badge-success-border);
  background: var(--trace-badge-success-bg);
  color: var(--trace-badge-success-text);
}

.badge-status-error {
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
  border-color: var(--border);
  background: var(--surface-muted);
  color: var(--muted);
}

.badge-duration {
  border-color: var(--trace-badge-duration-border);
  background: var(--trace-badge-duration-bg);
  color: var(--trace-badge-duration-text);
  font-variant-numeric: tabular-nums;
}

.empty-table {
  padding: 40px 18px;
  color: var(--muted);
  text-align: center;
  font-weight: 600;
}

.table-more {
  display: flex;
  justify-content: center;
  flex-wrap: wrap;
  gap: 10px;
  padding: 14px;
  border-top: 1px solid var(--border);
  background: var(--surface);
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
  border-color: var(--teal);
  background: var(--trace-interactive-hover-bg);
}
</style>
