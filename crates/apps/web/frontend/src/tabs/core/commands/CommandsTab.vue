<template>
  <section class="tab-detail-layout">
    <section class="commands-panel tab-detail-main">
      <div class="commands-toolbar">
        <span class="commands-count">{{ commandCount }} commands</span>
        <div class="commands-actions">
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
      <div class="commands-table">
        <DataTable
          :columns="columns"
          :rows="rows"
          empty-label="No commands"
          :total-rows="totalRows"
          :can-load-more="hasMoreRows"
          :can-load-all="hasMoreRows"
          :next-batch-size="nextBatchSize"
          @select="selectDetail"
          @toggle="toggleRow"
          @load-more="loadMore"
          @load-all="loadAll"
        />
      </div>
    </section>
    <DetailPanel :detail="selectedDetail" :trace-id="traceKey" @clear="clearDetail" />
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ChevronsDownUp, ChevronsUpDown } from '@lucide/vue';

import DataTable from '../../../components/DataTable.vue';
import DetailPanel from '../../../components/DetailPanel.vue';
import { TABLE_RENDER_LIMITS } from '../../tableConfig';
import { normalizeTableQuery } from '../../tableModel';
import {
  COMMAND_COLUMNS,
  buildCommandTree,
  collectDefaultExpandedIds,
  collectParentIds,
  flattenMatchingCommands,
  flattenVisibleCommands,
} from './model';

const props = defineProps({
  traceKey: {
    type: [String, Number],
    default: null,
  },
  traceDetail: {
    type: Object,
    default: null,
  },
  actionTree: {
    type: Object,
    required: true,
  },
  commands: {
    type: Object,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
});

const columns = COMMAND_COLUMNS;
const expandedIds = ref(new Set());
const visibleLimit = ref(TABLE_RENDER_LIMITS.initialRows);
const selectedDetail = ref(null);

const roots = computed(() => buildCommandTree(props.commands?.actions, props.commands?.links));
const parentIds = computed(() => collectParentIds(roots.value));
const hasTree = computed(() => parentIds.value.length > 0);
const normalizedQuery = computed(() => normalizeTableQuery(props.query));
const queryActive = computed(() => normalizedQuery.value.length > 0);
const commandCount = computed(() => props.commands?.actions?.length ?? 0);

const allRows = computed(() =>
  queryActive.value
    ? flattenMatchingCommands(roots.value, normalizedQuery.value)
    : flattenVisibleCommands(roots.value, expandedIds.value),
);

const totalRows = computed(() => allRows.value.length);
const rows = computed(() => allRows.value.slice(0, visibleLimit.value));
const remainingRows = computed(() => Math.max(totalRows.value - rows.value.length, 0));
const nextBatchSize = computed(() => Math.min(TABLE_RENDER_LIMITS.rowBatchSize, remainingRows.value));
const hasMoreRows = computed(() => remainingRows.value > 0 && nextBatchSize.value > 0);

watch(
  () => props.commands,
  () => {
    clearDetail();
    expandedIds.value = new Set(collectDefaultExpandedIds(roots.value));
  },
  { immediate: true },
);

watch([roots, normalizedQuery], () => {
  visibleLimit.value = TABLE_RENDER_LIMITS.initialRows;
});

function toggleRow(row) {
  const next = new Set(expandedIds.value);
  if (next.has(row.id)) {
    next.delete(row.id);
  } else {
    next.add(row.id);
  }
  expandedIds.value = next;
}

function selectDetail(detail) {
  selectedDetail.value = detail;
}

function clearDetail() {
  selectedDetail.value = null;
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
</script>

<style scoped>
.commands-panel {
  min-width: 0;
  min-height: 0;
  height: 100%;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  gap: 12px;
  padding: 18px;
  overflow: hidden;
}

.commands-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.commands-count {
  color: var(--muted);
  font-size: 12px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.commands-actions {
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
  background: var(--trace-interactive-bg);
}

.tree-action:disabled {
  color: var(--muted);
  cursor: not-allowed;
  opacity: 0.6;
}

.commands-table {
  min-height: 0;
}
</style>
