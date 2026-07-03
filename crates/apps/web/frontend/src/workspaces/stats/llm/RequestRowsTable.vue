<template>
  <section class="rows-table">
    <header>
      <h3>{{ effectiveTitle }}</h3>
      <span>{{ filteredRows.length }} / {{ totalRows }}</span>
    </header>
    <div class="table-shell">
      <table v-if="filteredRows.length">
        <thead>
          <tr>
            <th>{{ t('stats.llm.rows.time') }}</th>
            <th>{{ t('stats.llm.rows.model') }}</th>
            <th>{{ t('stats.llm.rows.endpoint') }}</th>
            <th>{{ t('stats.llm.rows.app') }}</th>
            <th class="numeric">{{ t('stats.llm.rows.total') }}</th>
            <th class="numeric">{{ t('stats.llm.rows.input') }}</th>
            <th class="numeric">{{ t('stats.llm.rows.output') }}</th>
            <th class="numeric">{{ t('stats.llm.rows.reasoning') }}</th>
            <th>{{ t('stats.llm.rows.status') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in filteredRows" :key="row.response_action_id">
            <td>
              <button type="button" @click="$emit('open-trace', row.trace_id)">{{ formatTime(row.started_at_ms) }}</button>
              <small>{{ row.trace_name }}</small>
            </td>
            <td>{{ row.model || t('stats.llm.common.unknownModel') }}</td>
            <td :title="row.request_endpoint || ''">{{ row.endpoint_label || '-' }}</td>
            <td :title="row.app_executable || ''">{{ row.app_label || '-' }}</td>
            <td class="numeric">{{ formatOptional(row.total_tokens) }}</td>
            <td class="numeric">{{ formatOptional(row.input_tokens) }}</td>
            <td class="numeric">{{ formatOptional(row.output_tokens) }}</td>
            <td class="numeric">{{ formatOptional(row.reasoning_tokens) }}</td>
            <td>
              <span class="status" :class="{ missing: !row.has_usage }">
                {{ row.has_usage ? t('stats.llm.rows.completed') : t('stats.llm.rows.missingUsage') }}
              </span>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else class="empty">{{ emptyLabel }}</div>
    </div>
    <footer v-if="canLoadMore">
      <button type="button" @click="$emit('load-more')">{{ t('stats.llm.rows.loadMore') }}</button>
    </footer>
  </section>
</template>

<script setup>
import { computed } from 'vue';

import { useLocale } from '../../../locale';
import { formatOptional, formatTime } from './model';

const props = defineProps({
  rows: {
    type: Array,
    default: () => [],
  },
  totalRows: {
    type: Number,
    default: 0,
  },
  query: {
    type: String,
    default: '',
  },
  canLoadMore: {
    type: Boolean,
    default: false,
  },
  title: {
    type: String,
    default: '',
  },
});

defineEmits(['load-more', 'open-trace']);

const { t } = useLocale();
const effectiveTitle = computed(() => props.title || t('stats.llm.rows.requestRows'));
const filteredRows = computed(() => {
  const needle = props.query.trim().toLowerCase();
  if (!needle) {
    return props.rows;
  }
  return props.rows.filter((row) =>
    [
      row.trace_name,
      row.model,
      row.endpoint_label,
      row.request_endpoint,
      row.app_label,
      row.app_executable,
      row.response_action_id,
      row.request_action_id,
    ].some((value) => String(value ?? '').toLowerCase().includes(needle)),
  );
});

const emptyLabel = computed(() =>
  props.rows.length ? t('stats.llm.rows.filtersRemovedAll') : t('stats.llm.rows.emptyRange'),
);
</script>

<style scoped>
.rows-table {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-md);
}

header,
footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-md);
}

h3 {
  margin: 0;
  font-size: var(--stats-font-title);
}

header span {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.table-shell {
  min-width: 0;
  max-height: min(640px, 72vh);
  overflow: auto;
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
}

table {
  width: 100%;
  min-width: 980px;
  border-collapse: separate;
  border-spacing: 0;
  font-size: var(--stats-font-sm);
}

th,
td {
  padding: var(--stats-table-cell-padding);
  border-bottom: 1px solid var(--stats-border);
  text-align: left;
  vertical-align: top;
}

th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--stats-surface-strong);
  color: var(--stats-muted);
  font-weight: var(--stats-weight-medium);
}

td small {
  display: block;
  margin-top: var(--stats-table-subtext-gap);
  color: var(--stats-muted);
}

td button {
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--stats-accent);
  cursor: pointer;
  font: inherit;
}

.numeric {
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.status {
  display: inline-flex;
  min-height: 24px;
  align-items: center;
  padding: 0 var(--stats-space-sm);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-accent-muted);
  color: var(--stats-text);
}

.status.missing {
  background: rgba(190, 18, 60, 0.12);
  color: var(--stats-danger);
}

.empty {
  min-height: 150px;
  display: grid;
  place-items: center;
  color: var(--stats-muted);
}

footer button {
  min-height: var(--stats-control-height-md);
  padding: 0 var(--stats-action-padding-x);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
  color: var(--stats-text);
  cursor: pointer;
}
</style>
