<template>
  <div class="data-table-shell">
    <table v-if="rows.length" class="data-table">
      <thead>
        <tr>
          <th v-for="column in columns" :key="column.key" scope="col">{{ column.label }}</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="row in rows"
          :key="row.id"
          class="data-row"
          tabindex="0"
          @click="select(row)"
          @keydown.enter.prevent="select(row)"
          @keydown.space.prevent="select(row)"
        >
          <td v-for="column in columns" :key="column.key">
            <span class="cell-text">
              <span
                v-for="index in cellIndent(row.cells[column.key])"
                :key="index"
                class="tree-indent-unit"
                aria-hidden="true"
              />
              {{ cellText(row.cells[column.key]) }}
            </span>
          </td>
        </tr>
      </tbody>
    </table>
    <div v-if="hasMoreRows" class="table-more">
      <button class="load-more" type="button" @click="$emit('load-more')">
        Load {{ nextBatchSize }} more of {{ remainingRows }}
      </button>
    </div>
    <div v-if="!rows.length" class="empty-table">{{ emptyLabel }}</div>
  </div>
</template>

<script setup>
import { computed } from 'vue';

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
});

const emit = defineEmits(['select', 'load-more']);

const totalRowCount = computed(() =>
  Number.isInteger(props.totalRows) && props.totalRows >= 0 ? props.totalRows : props.rows.length,
);
const remainingRows = computed(() => Math.max(totalRowCount.value - props.rows.length, 0));
const nextBatchSize = computed(() => Math.min(positiveInteger(props.nextBatchSize), remainingRows.value));
const hasMoreRows = computed(() => props.canLoadMore && remainingRows.value > 0 && nextBatchSize.value > 0);

function select(row) {
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

function cellIndent(cell) {
  if (!cell || typeof cell !== 'object' || !cell.indent) {
    return [];
  }
  return Array.from({ length: cell.indent }, (_, index) => index);
}
</script>

<style scoped>
.data-table-shell {
  min-width: 0;
  height: 100%;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface);
}

.data-table {
  width: 100%;
  min-width: 760px;
  border-collapse: collapse;
}

.data-table th,
.data-table td {
  padding: 10px 12px;
  border-bottom: 1px solid var(--border);
  text-align: left;
  vertical-align: top;
}

.data-table th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: #f8fbfa;
  color: var(--muted);
  font-size: 12px;
  font-weight: 800;
  text-transform: uppercase;
}

.data-row {
  cursor: pointer;
}

.data-row:hover td,
.data-row:focus td {
  background: #eef7f5;
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

.empty-table {
  padding: 32px 18px;
  color: var(--muted);
  text-align: center;
}

.table-more {
  display: flex;
  justify-content: center;
  padding: 14px;
  border-top: 1px solid var(--border);
}

.load-more {
  height: 34px;
  padding: 0 14px;
  border: 1px solid #bdd7d2;
  border-radius: 8px;
  background: #eef7f5;
  color: var(--teal-deep);
  cursor: pointer;
}

.load-more:hover {
  border-color: var(--teal);
}
</style>
