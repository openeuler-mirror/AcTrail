<template>
  <section class="attribution-stats-workspace">
    <header class="stats-header">
      <div>
        <span class="stats-kicker">{{ t('header.kicker') }}</span>
        <h2>{{ t('header.title') }}</h2>
        <p>{{ t('header.note') }}</p>
      </div>
      <div class="range-controls">
        <div class="quick-ranges">
          <button type="button" @click="setQuickRange(1)">24h</button>
          <button type="button" @click="setQuickRange(7)">7d</button>
          <button type="button" @click="setQuickRange(30)">30d</button>
        </div>
        <label>
          <span>{{ t('header.from') }}</span>
          <input v-model="range.fromDate" type="date" />
        </label>
        <label>
          <span>{{ t('header.to') }}</span>
          <input v-model="range.toDate" type="date" />
        </label>
        <button class="refresh-button" type="button" :disabled="loading" @click="reload">
          <RefreshCw :size="16" aria-hidden="true" />
          {{ t('header.refresh') }}
        </button>
      </div>
    </header>

    <div v-if="error" class="stats-error">{{ error }}</div>

    <template v-if="activity">
      <section class="summary-panel">
        <div class="summary-heading">
          <span>
            <small>{{ t('summary.totalScope') }}</small>
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
        <div class="coverage-line">{{ coverageLine }}</div>
      </section>

      <section class="breakdown-grid">
        <article class="breakdown-panel">
          <header>
            <h3>{{ t('models.title') }}</h3>
            <span>{{ t('models.note') }}</span>
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
              <small>{{ t('models.callsIntervals', { calls: row.action_count, intervals: row.segment_count }) }}</small>
            </span>
            <span class="measure">
              <strong>{{ formatAttributionDuration(row.duration_nanos) }}</strong>
              <small>{{ formatAttributionPercent(row.percentage_bps) }}</small>
            </span>
          </button>
          <div v-if="!filteredModels.length" class="empty-panel">{{ t('models.empty') }}</div>
        </article>

        <article class="breakdown-panel">
          <header>
            <h3>{{ t('tools.title') }}</h3>
            <span>{{ t('tools.note') }}</span>
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
              <small>{{ t('tools.actionsIntervals', { actions: row.action_count, intervals: row.segment_count }) }}</small>
            </span>
            <span class="measure">
              <strong>{{ formatAttributionDuration(row.duration_nanos) }}</strong>
              <small>{{ formatAttributionPercent(row.percentage_bps) }}</small>
            </span>
          </button>
          <div v-if="!filteredTools.length" class="empty-panel">{{ t('tools.empty') }}</div>
        </article>

        <article class="breakdown-panel">
          <header>
            <h3>{{ t('commands.title') }}</h3>
            <span>{{ t('commands.note') }}</span>
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
              <small>{{ commandCountLabel(row, t) }}</small>
              <small v-if="row.agent_tools?.length">
                {{ t('commands.viaTool', { tools: row.agent_tools.join(', ') }) }}
              </small>
            </span>
            <span class="measure">
              <strong>{{ formatAttributionDuration(row.duration_nanos) }}</strong>
              <small>{{ formatAttributionPercent(row.percentage_bps) }}</small>
            </span>
          </button>
          <div v-if="!filteredCommands.length" class="empty-panel">
            {{ t('commands.empty') }}
          </div>
        </article>
      </section>

      <section class="trace-results">
        <header>
          <div>
            <h3>{{ t('traces.title') }}</h3>
            <p v-if="selectedFilter">
              {{ t('traces.selected', { label: selectedFilter.label, count: rowTotal }) }}
            </p>
            <p v-else>{{ t('traces.selectHint') }}</p>
          </div>
        </header>
        <div v-if="rowLoading" class="empty-panel">{{ t('traces.loading') }}</div>
        <div v-else-if="selectedFilter && !filteredRows.length" class="empty-panel">
          {{ t('traces.noMatch') }}
        </div>
        <div v-else-if="!selectedFilter" class="empty-panel">
          {{ t('traces.idle') }}
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
              <small>{{ t('traces.row', { id: row.trace.id, status: statusLabel(row.status) }) }}</small>
            </span>
            <span class="measure">
              <strong>{{ formatAttributionDuration(row.contribution_duration_nanos) }}</strong>
              <small v-if="selectedFilter?.dimension === 'tool'">
                {{ t('traces.overlapShare', { percent: formatAttributionPercent(row.percentage_bps) }) }}
              </small>
              <small v-else>
                {{ t('traces.clippedShare', { percent: formatAttributionPercent(row.percentage_bps) }) }}
              </small>
            </span>
            <ExternalLink :size="15" aria-hidden="true" />
          </button>
          <button v-if="rows.length < rowTotal" class="load-more" type="button" @click="loadMore">
            {{ t('traces.loadMore') }}
          </button>
        </div>
      </section>

      <section v-if="activity.issues?.length" class="aggregate-issues">
        <h3>{{ t('issues.title') }}</h3>
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
} from '../../../components/time-attribution/model';
import { useModuleLocale } from '../../../locale';
import { defaultRange, quickRange, rangeToMillis } from '../llm/model';
import {
  commandCountLabel,
  filterBreakdownRows,
  matchesAttributionQuery,
  openTraceEvent,
} from './workspace-model';
import strings from './locale';

const { t } = useModuleLocale(strings);

const STATUS_VALUES = Object.freeze(['complete', 'provisional', 'partial', 'invalid']);
const coverageLine = computed(() =>
  t('summary.coverage', {
    traces: activity.value.coverage.trace_count,
    paired: activity.value.coverage.paired_llm_call_count,
    attributed: activity.value.coverage.attributed_llm_call_count,
    toolIntervals: activity.value.coverage.tool_interval_count,
    commands: activity.value.coverage.command_interval_count ?? 0,
  }),
);

/** Attribution status words are shared vocabulary, so they live in the base dictionary. */
function statusLabel(status) {
  return STATUS_VALUES.includes(status) ? t(`common.status.${status}`) : attributionStatusLabel(status);
}

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
const filteredModels = computed(() =>
  filterBreakdownRows(activity.value?.models ?? [], normalizedQuery.value));
const filteredTools = computed(() =>
  filterBreakdownRows(activity.value?.tools ?? [], normalizedQuery.value));
const filteredCommands = computed(() =>
  filterBreakdownRows(activity.value?.commands ?? [], normalizedQuery.value));
const filteredRows = computed(() =>
  rows.value.filter((row) =>
    matchesAttributionQuery(
      [row.trace?.name, row.trace?.id, row.trace?.state, row.status],
      normalizedQuery.value,
    ),
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
  const activeActivityController = activityController;
  const activeRowController = rowController;
  activityController = null;
  rowController = null;
  activeActivityController?.abort();
  activeRowController?.abort();
  emit('loading', false);
});

function setQuickRange(days) { range.value = quickRange(days); }

async function reload() {
  const parsed = parsedRange.value;
  const previousActivityController = activityController;
  const previousRowController = rowController;
  activityController = null;
  rowController = null;
  previousActivityController?.abort();
  previousRowController?.abort();
  activityLoading.value = false;
  rowLoading.value = false;
  if (!parsed.ok) {
    error.value = parsed.error;
    activity.value = null;
    return;
  }
  const activeController = new AbortController();
  activityController = activeController;
  activityLoading.value = true;
  selectedFilter.value = null;
  rows.value = [];
  rowTotal.value = 0;
  error.value = '';
  try {
    const result = await readTimeAttributionActivity({
      fromMs: parsed.fromMs,
      toMs: parsed.toMs,
      signal: activeController.signal,
    });
    if (activityController === activeController) activity.value = result;
  } catch (err) {
    if (activityController === activeController && err?.name !== 'AbortError') {
      error.value = String(err.message ?? err);
      activity.value = null;
    }
  } finally {
    if (activityController === activeController) {
      activityController = null;
      activityLoading.value = false;
    }
  }
}

function selectCategory(row) { selectBreakdown('category', row); }

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
  const activeController = new AbortController();
  rowController = activeController;
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
      signal: activeController.signal,
    });
    if (rowController !== activeController) return;
    const nextRows = Array.isArray(response.rows) ? response.rows : [];
    rows.value = offset === 0 ? nextRows : rows.value.concat(nextRows);
    rowTotal.value = Number(response.page?.total ?? rows.value.length);
  } catch (err) {
    if (rowController === activeController && err?.name !== 'AbortError') {
      error.value = String(err.message ?? err);
    }
  } finally {
    if (rowController === activeController) {
      rowController = null;
      rowLoading.value = false;
    }
  }
}

function openTrace(row) {
  emit('open-trace', openTraceEvent(row, selectedFilter.value, t));
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
  gap: var(--ui-section-gap);
  padding: var(--ui-viewport-padding);
  color: var(--ui-text);
  font-family: var(--ui-body-font);
}

.stats-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--ui-space-xl);
}

.stats-kicker {
  color: var(--ui-accent);
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-medium);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.stats-header h2,
.summary-heading strong {
  margin: var(--ui-space-xs) 0;
  font-family: var(--ui-heading-font);
}

.stats-header p,
.trace-results p {
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.range-controls {
  display: flex;
  align-items: flex-end;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: var(--ui-space-sm);
}

.quick-ranges {
  display: flex;
  padding: var(--ui-space-2xs);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
}

.quick-ranges button,
.refresh-button {
  min-height: var(--ui-control-height-md);
  border: 0;
  border-radius: var(--ui-radius-sm);
  background: transparent;
  color: var(--ui-text);
  cursor: pointer;
}

.quick-ranges button {
  padding: 0 var(--ui-segment-padding-x);
}

.range-controls label {
  display: grid;
  gap: var(--ui-space-2xs);
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.range-controls input {
  height: var(--ui-control-height-md);
  padding: 0 var(--ui-space-sm);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
}

.refresh-button {
  display: inline-flex;
  align-items: center;
  gap: var(--ui-space-xs);
  padding: 0 var(--ui-space-md);
  border: 1px solid var(--ui-border);
}

.summary-panel,
.breakdown-panel,
.trace-results,
.aggregate-issues {
  display: grid;
  gap: var(--ui-space-lg);
  padding: var(--ui-space-xl);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface);
}

.summary-heading,
.breakdown-panel header,
.trace-results > header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--ui-space-lg);
}

.summary-heading > span:first-child {
  display: grid;
  gap: var(--ui-space-xs);
}

.summary-heading small,
.coverage-line,
.breakdown-panel header span,
.breakdown-panel button small,
.trace-table button small {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.summary-heading strong {
  font-size: var(--ui-font-display-lg);
}

.status-badge {
  padding: var(--ui-space-xs) var(--ui-space-md);
  border: 1px solid var(--ui-border);
  border-radius: 999px;
  font-size: var(--ui-font-xs);
  text-transform: uppercase;
}

.status-complete {
  color: var(--ui-success);
}

.status-provisional {
  color: var(--ui-accent);
}

.status-partial,
.status-invalid {
  color: var(--ui-danger);
}

.category-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--ui-space-md);
}

.category-grid button {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--ui-space-sm);
  padding: var(--ui-space-lg);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  cursor: pointer;
  text-align: left;
}

.category-grid button.selected,
.breakdown-panel button.selected {
  border-color: var(--ui-accent);
  background: var(--ui-accent-muted);
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
  background: var(--ui-chart-cache-hit, #48b89f);
}

.dot-model_side {
  background: var(--ui-chart-output, #7b8cff);
}

.dot-unattributed {
  background: var(--ui-chart-reasoning, #9aa0aa);
}

.breakdown-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--ui-section-gap);
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
  gap: var(--ui-space-md);
  padding: var(--ui-space-md);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  cursor: pointer;
  text-align: left;
}

.breakdown-panel button > span,
.trace-table button > span {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-2xs);
}

.measure {
  justify-items: end;
}

.trace-table {
  display: grid;
  gap: var(--ui-space-sm);
}

.trace-table button {
  grid-template-columns: minmax(0, 1fr) auto auto;
}

.trace-table .load-more {
  display: block;
  text-align: center;
}

.empty-panel {
  padding: var(--ui-space-xl);
  color: var(--ui-muted);
  text-align: center;
}

.aggregate-issues span {
  display: grid;
  grid-template-columns: minmax(180px, auto) minmax(0, 1fr);
  gap: var(--ui-space-md);
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.aggregate-issues strong {
  color: var(--ui-text);
}

.stats-error {
  padding: var(--ui-space-md);
  border: 1px solid var(--ui-danger);
  border-radius: var(--ui-radius-sm);
  color: var(--ui-danger);
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
