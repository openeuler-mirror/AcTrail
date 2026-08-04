<template>
  <section class="attribution-stats-workspace">
    <header class="stats-header">
      <div>
        <span class="stats-kicker">Exclusive wall-clock statistics</span>
        <h2>Agent / model time attribution</h2>
        <p>Trace intervals that overlap the selected range are clipped before aggregation.</p>
      </div>
      <div class="range-controls">
        <div class="quick-ranges">
          <button type="button" @click="setQuickRange(1)">24h</button>
          <button type="button" @click="setQuickRange(7)">7d</button>
          <button type="button" @click="setQuickRange(30)">30d</button>
        </div>
        <label>
          <span>From</span>
          <input v-model="range.fromDate" type="date" />
        </label>
        <label>
          <span>To</span>
          <input v-model="range.toDate" type="date" />
        </label>
        <button class="refresh-button" type="button" :disabled="loading" @click="reload">
          <RefreshCw :size="16" aria-hidden="true" />
          Refresh
        </button>
      </div>
    </header>

    <div v-if="error" class="stats-error">{{ error }}</div>

    <template v-if="activity">
      <section class="summary-panel">
        <div class="summary-heading">
          <span>
            <small>Total attributed scope</small>
            <strong>{{ formatAttributionDuration(activity.total_duration_nanos) }}</strong>
          </span>
          <span class="status-badge" :class="`status-${activity.status}`">
            {{ attributionStatusLabel(activity.status) }}
          </span>
        </div>
        <TimeAttributionBar :categories="activity.categories" @select="selectCategory" />
        <div class="category-grid">
          <button
            v-for="category in activity.categories"
            :key="category.key"
            type="button"
            :class="{ selected: selectedFilter?.dimension === 'category' && selectedFilter.key === category.key }"
            @click="selectCategory(category)"
          >
            <span class="category-dot" :class="`dot-${category.key}`"></span>
            <span>{{ category.label }}</span>
            <strong>{{ formatAttributionDuration(category.duration_nanos) }}</strong>
            <small>{{ formatAttributionPercent(category.percentage_bps) }}</small>
          </button>
        </div>
        <div class="coverage-line">
          {{ activity.coverage.trace_count }} traces ·
          {{ activity.coverage.llm_call_count }} model calls ·
          {{ activity.coverage.tool_interval_count }} Agent Tool intervals ·
          {{ activity.coverage.command_interval_count ?? 0 }} command processes
        </div>
      </section>

      <section class="breakdown-grid">
        <article class="breakdown-panel">
          <header>
            <h3>Models</h3>
            <span>Observable model-side wall time</span>
          </header>
          <button
            v-for="row in filteredModels"
            :key="row.key"
            type="button"
            :class="{ selected: selectedFilter?.dimension === 'model' && selectedFilter.key === row.key }"
            @click="selectBreakdown('model', row)"
          >
            <span>
              <strong>{{ row.label }}</strong>
              <small>{{ row.action_count }} calls · {{ row.segment_count }} intervals</small>
            </span>
            <span class="measure">
              <strong>{{ formatAttributionDuration(row.duration_nanos) }}</strong>
              <small>{{ formatAttributionPercent(row.percentage_bps) }}</small>
            </span>
          </button>
          <div v-if="!filteredModels.length" class="empty-panel">No model time in range.</div>
        </article>

        <article class="breakdown-panel">
          <header>
            <h3>Agent Tools</h3>
            <span>Logical tools requested by the model, plus local work</span>
          </header>
          <button
            v-for="row in filteredTools"
            :key="row.key"
            type="button"
            :class="{ selected: selectedFilter?.dimension === 'tool' && selectedFilter.key === row.key }"
            @click="selectBreakdown('tool', row)"
          >
            <span>
              <strong>{{ row.label }}</strong>
              <small>{{ row.action_count }} actions · {{ row.segment_count }} intervals</small>
            </span>
            <span class="measure">
              <strong>{{ formatAttributionDuration(row.duration_nanos) }}</strong>
              <small>{{ formatAttributionPercent(row.percentage_bps) }}</small>
            </span>
          </button>
          <div v-if="!filteredTools.length" class="empty-panel">No Agent-side time in range.</div>
        </article>

        <article class="breakdown-panel">
          <header>
            <h3>Commands</h3>
            <span>Actual command process trees, counted exclusively</span>
          </header>
          <button
            v-for="row in filteredCommands"
            :key="row.key"
            type="button"
            :class="{ selected: selectedFilter?.dimension === 'command' && selectedFilter.key === row.key }"
            @click="selectBreakdown('command', row)"
          >
            <span>
              <strong>{{ row.label }}</strong>
              <small>{{ commandCountLabel(row) }}</small>
              <small v-if="row.agent_tools?.length">
                via Agent Tool: {{ row.agent_tools.join(', ') }}
              </small>
            </span>
            <span class="measure">
              <strong>{{ formatAttributionDuration(row.duration_nanos) }}</strong>
              <small>{{ formatAttributionPercent(row.percentage_bps) }}</small>
            </span>
          </button>
          <div v-if="!filteredCommands.length" class="empty-panel">
            No actual commands in range.
          </div>
        </article>
      </section>

      <section class="trace-results">
        <header>
          <div>
            <h3>Matching traces</h3>
            <p v-if="selectedFilter">
              {{ selectedFilter.label }} · {{ rowTotal }} traces
            </p>
            <p v-else>Select a category, model, Agent Tool, or command to drill down.</p>
          </div>
        </header>
        <div v-if="rowLoading" class="empty-panel">Loading trace intervals…</div>
        <div v-else-if="selectedFilter && !filteredRows.length" class="empty-panel">
          No traces match this item and the global filter.
        </div>
        <div v-else-if="!selectedFilter" class="empty-panel">
          Aggregates remain query-light until a drill-down item is selected.
        </div>
        <div v-else class="trace-table">
          <button
            v-for="row in filteredRows"
            :key="row.trace.id"
            type="button"
            @click="openTrace(row)"
          >
            <span>
              <strong>{{ row.trace.name }}</strong>
              <small>Trace {{ row.trace.id }} · {{ attributionStatusLabel(row.status) }}</small>
            </span>
            <span class="measure">
              <strong>{{ formatAttributionDuration(row.contribution_duration_nanos) }}</strong>
              <small>{{ formatAttributionPercent(row.percentage_bps) }} of clipped Trace</small>
            </span>
            <ExternalLink :size="15" aria-hidden="true" />
          </button>
          <button v-if="rows.length < rowTotal" class="load-more" type="button" @click="loadMore">
            Load more
          </button>
        </div>
      </section>

      <section v-if="activity.issues?.length" class="aggregate-issues">
        <h3>Collection status</h3>
        <span v-for="issue in activity.issues" :key="issue.code">
          <strong>{{ issue.code }} × {{ issue.count }}</strong>
          {{ issue.message }}
        </span>
      </section>
    </template>
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { ExternalLink, RefreshCw } from '@lucide/vue';

import { readTimeAttributionActivity, readTimeAttributionRows } from '../../../api';
import TimeAttributionBar from '../../../components/time-attribution/TimeAttributionBar.vue';
import {
  attributionStatusLabel,
  formatAttributionDuration,
  formatAttributionPercent,
  normalizeAttributionTarget,
} from '../../../components/time-attribution/model';
import { defaultRange, quickRange, rangeToMillis } from '../llm/model';

const props = defineProps({
  query: {
    type: String,
    default: '',
  },
  refreshNonce: {
    type: Number,
    default: 0,
  },
});

const emit = defineEmits(['loading', 'open-trace']);
const range = ref(defaultRange());
const activity = ref(null);
const rows = ref([]);
const rowTotal = ref(0);
const selectedFilter = ref(null);
const error = ref('');
const activityLoading = ref(false);
const rowLoading = ref(false);
const rowLimit = 50;
let activityController = null;
let rowController = null;

const loading = computed(() => activityLoading.value || rowLoading.value);
const parsedRange = computed(() => rangeToMillis(range.value));
const normalizedQuery = computed(() => props.query.trim().toLowerCase());
const filteredModels = computed(() => filterRows(activity.value?.models ?? []));
const filteredTools = computed(() => filterRows(activity.value?.tools ?? []));
const filteredCommands = computed(() => filterRows(activity.value?.commands ?? []));
const filteredRows = computed(() =>
  rows.value.filter((row) =>
    matchesQuery([row.trace?.name, row.trace?.id, row.trace?.state, row.status]),
  ),
);

watch(
  () => [range.value.fromDate, range.value.toDate],
  reload,
);
watch(
  () => props.refreshNonce,
  reload,
);
watch(
  loading,
  (value) => emit('loading', value),
  { immediate: true },
);

onMounted(reload);
onBeforeUnmount(() => {
  activityController?.abort();
  rowController?.abort();
  emit('loading', false);
});

function setQuickRange(days) {
  range.value = quickRange(days);
}

async function reload() {
  const parsed = parsedRange.value;
  if (!parsed.ok) {
    error.value = parsed.error;
    activity.value = null;
    return;
  }
  activityController?.abort();
  rowController?.abort();
  activityController = new AbortController();
  activityLoading.value = true;
  rowLoading.value = false;
  selectedFilter.value = null;
  rows.value = [];
  rowTotal.value = 0;
  error.value = '';
  try {
    activity.value = await readTimeAttributionActivity({
      fromMs: parsed.fromMs,
      toMs: parsed.toMs,
      signal: activityController.signal,
    });
  } catch (err) {
    if (err?.name !== 'AbortError') {
      error.value = String(err.message ?? err);
      activity.value = null;
    }
  } finally {
    activityLoading.value = false;
  }
}

function selectCategory(row) {
  selectBreakdown('category', row);
}

async function selectBreakdown(dimension, row) {
  if (!row?.key) {
    return;
  }
  selectedFilter.value = {
    dimension,
    key: row.key,
    label: row.label,
  };
  rows.value = [];
  rowTotal.value = 0;
  await loadRows(0);
}

async function loadMore() {
  await loadRows(rows.value.length);
}

async function loadRows(offset) {
  const parsed = parsedRange.value;
  const filter = selectedFilter.value;
  if (!parsed.ok || !filter) {
    return;
  }
  rowController?.abort();
  rowController = new AbortController();
  rowLoading.value = true;
  error.value = '';
  try {
    const response = await readTimeAttributionRows({
      fromMs: parsed.fromMs,
      toMs: parsed.toMs,
      offset,
      limit: rowLimit,
      dimension: filter.dimension,
      key: filter.key,
      signal: rowController.signal,
    });
    const nextRows = Array.isArray(response.rows) ? response.rows : [];
    rows.value = offset === 0 ? nextRows : rows.value.concat(nextRows);
    rowTotal.value = Number(response.page?.total ?? rows.value.length);
  } catch (err) {
    if (err?.name !== 'AbortError') {
      error.value = String(err.message ?? err);
    }
  } finally {
    rowLoading.value = false;
  }
}

function openTrace(row) {
  const focus = normalizeAttributionTarget(row.target, {
    source: 'Stats Time Attribution',
    dimension: selectedFilter.value?.dimension,
    key: selectedFilter.value?.key,
    label: selectedFilter.value?.label,
    description: `Longest contiguous interval in this Trace · ${formatAttributionDuration(row.contribution_duration_nanos)} total contribution`,
  });
  emit('open-trace', {
    traceId: row.trace.id,
    tabId: focus ? 'waterfall' : 'time_attribution',
    focus,
  });
}

function filterRows(values) {
  return values.filter((row) =>
    matchesQuery([row.label, row.key, row.kind, ...(row.agent_tools ?? [])]),
  );
}

function commandCountLabel(row) {
  if (row.kind === 'tool_overhead') {
    return `${row.segment_count} intervals · Agent Tool self-time`;
  }
  return `${row.action_count} command processes · ${row.segment_count} intervals`;
}

function matchesQuery(values) {
  if (!normalizedQuery.value) {
    return true;
  }
  return values
    .filter((value) => value !== null && value !== undefined)
    .join(' ')
    .toLowerCase()
    .includes(normalizedQuery.value);
}
</script>

<style scoped>
.attribution-stats-workspace {
  min-width: 0;
  min-height: 0;
  width: 100%;
  height: 100%;
  overflow: auto;
  display: grid;
  align-content: start;
  gap: var(--stats-section-gap);
  padding: var(--stats-viewport-padding);
  color: var(--stats-text);
  font-family: var(--stats-body-font);
}

.stats-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--stats-space-xl);
}

.stats-kicker {
  color: var(--stats-accent);
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.stats-header h2,
.summary-heading strong {
  margin: var(--stats-space-xs) 0;
  font-family: var(--stats-heading-font);
}

.stats-header p,
.trace-results p {
  margin: 0;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.range-controls {
  display: flex;
  align-items: flex-end;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--stats-space-sm);
}

.quick-ranges {
  display: flex;
  padding: var(--stats-space-2xs);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
}

.quick-ranges button,
.refresh-button {
  min-height: var(--stats-control-height-md);
  border: 0;
  border-radius: var(--stats-radius-sm);
  background: transparent;
  color: var(--stats-text);
  cursor: pointer;
}

.quick-ranges button {
  padding: 0 var(--stats-segment-padding-x);
}

.range-controls label {
  display: grid;
  gap: var(--stats-space-2xs);
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}

.range-controls input {
  height: var(--stats-control-height-md);
  padding: 0 var(--stats-space-sm);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
}

.refresh-button {
  display: inline-flex;
  align-items: center;
  gap: var(--stats-space-xs);
  padding: 0 var(--stats-space-md);
  border: 1px solid var(--stats-border);
}

.summary-panel,
.breakdown-panel,
.trace-results,
.aggregate-issues {
  display: grid;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-xl);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface);
}

.summary-heading,
.breakdown-panel header,
.trace-results > header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--stats-space-lg);
}

.summary-heading > span:first-child {
  display: grid;
  gap: var(--stats-space-xs);
}

.summary-heading small,
.coverage-line,
.breakdown-panel header span,
.breakdown-panel button small,
.trace-table button small {
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}

.summary-heading strong {
  font-size: var(--stats-font-display-lg);
}

.status-badge {
  padding: var(--stats-space-xs) var(--stats-space-md);
  border: 1px solid var(--stats-border);
  border-radius: 999px;
  font-size: var(--stats-font-xs);
  text-transform: uppercase;
}

.status-complete {
  color: var(--stats-success);
}

.status-provisional {
  color: var(--stats-accent);
}

.status-partial,
.status-invalid {
  color: var(--stats-danger);
}

.category-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--stats-space-md);
}

.category-grid button {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--stats-space-sm);
  padding: var(--stats-space-lg);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  cursor: pointer;
  text-align: left;
}

.category-grid button.selected,
.breakdown-panel button.selected {
  border-color: var(--stats-accent);
  background: var(--stats-accent-muted);
}

.category-grid button > strong,
.category-grid button > small {
  grid-column: 2;
}

.category-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}

.dot-agent_side {
  background: var(--stats-chart-cache-hit, #48b89f);
}

.dot-model_side {
  background: var(--stats-chart-output, #7b8cff);
}

.dot-unattributed {
  background: var(--stats-chart-reasoning, #9aa0aa);
}

.breakdown-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--stats-section-gap);
}

.breakdown-panel h3,
.trace-results h3,
.aggregate-issues h3 {
  margin: 0;
}

.breakdown-panel button,
.trace-table button {
  width: 100%;
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--stats-space-md);
  padding: var(--stats-space-md);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  cursor: pointer;
  text-align: left;
}

.breakdown-panel button > span,
.trace-table button > span {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-2xs);
}

.measure {
  justify-items: end;
}

.trace-table {
  display: grid;
  gap: var(--stats-space-sm);
}

.trace-table button {
  grid-template-columns: minmax(0, 1fr) auto auto;
}

.trace-table .load-more {
  display: block;
  text-align: center;
}

.empty-panel {
  padding: var(--stats-space-xl);
  color: var(--stats-muted);
  text-align: center;
}

.aggregate-issues span {
  display: grid;
  grid-template-columns: minmax(180px, auto) minmax(0, 1fr);
  gap: var(--stats-space-md);
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.aggregate-issues strong {
  color: var(--stats-text);
}

.stats-error {
  padding: var(--stats-space-md);
  border: 1px solid var(--stats-danger);
  border-radius: var(--stats-radius-sm);
  color: var(--stats-danger);
}

@media (max-width: 920px) {
  .stats-header {
    display: grid;
  }

  .range-controls {
    justify-content: flex-start;
  }

  .breakdown-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (max-width: 700px) {
  .category-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
