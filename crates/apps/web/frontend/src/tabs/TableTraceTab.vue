<template>
  <section class="table-panel">
    <DataTable
      :columns="view.columns"
      :rows="view.rows"
      :empty-label="view.emptyLabel"
      :total-rows="view.totalRows"
      :can-load-more="hasMoreRows"
      :next-batch-size="nextBatchSize"
      @select="$emit('select-detail', $event)"
      @load-more="loadMore"
    />
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

import DataTable from '../components/DataTable.vue';
import { TABLE_RENDER_LIMITS } from './tableConfig';
import { filterTableRows, normalizeTableQuery, positiveInteger } from './tableModel';

const props = defineProps({
  traceDetail: {
    type: Object,
    default: null,
  },
  actionTree: {
    type: Object,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
  projector: {
    type: Function,
    required: true,
  },
  initialRows: {
    type: Number,
    default: TABLE_RENDER_LIMITS.initialRows,
  },
  rowBatchSize: {
    type: Number,
    default: TABLE_RENDER_LIMITS.rowBatchSize,
  },
});

defineEmits(['select-detail']);

const visibleLimit = ref(0);
const effectiveInitialRows = computed(() => positiveInteger(props.initialRows));
const effectiveBatchSize = computed(() => positiveInteger(props.rowBatchSize));
const batchingEnabled = computed(() => effectiveInitialRows.value > 0 && effectiveBatchSize.value > 0);
const normalizedQuery = computed(() => normalizeTableQuery(props.query));

const view = computed(() => {
  const nextView = props.projector({
    traceDetail: props.traceDetail,
    actionTree: props.actionTree,
    query: normalizedQuery.value,
    rowLimit: batchingEnabled.value ? visibleLimit.value : 0,
  });
  const rows = nextView.queryApplied ? nextView.rows : filterTableRows(nextView.rows, normalizedQuery.value);
  return {
    ...nextView,
    rows,
    totalRows: nextView.totalRows ?? rows.length,
  };
});
const remainingRows = computed(() => Math.max(view.value.totalRows - view.value.rows.length, 0));
const nextBatchSize = computed(() => Math.min(effectiveBatchSize.value, remainingRows.value));
const hasMoreRows = computed(() => batchingEnabled.value && remainingRows.value > 0 && nextBatchSize.value > 0);

watch(
  () => [
    props.traceDetail,
    props.actionTree,
    normalizedQuery.value,
    props.projector,
    effectiveInitialRows.value,
    effectiveBatchSize.value,
  ],
  () => {
    visibleLimit.value = effectiveInitialRows.value;
  },
  { immediate: true },
);

function loadMore() {
  visibleLimit.value += effectiveBatchSize.value;
}
</script>

<style scoped>
.table-panel {
  min-width: 0;
  min-height: 0;
  height: 100%;
  padding: 18px;
  overflow: hidden;
}
</style>
