<template>
  <section class="time-attribution-tab">
    <div v-if="!attribution" class="attribution-empty">
      Select a trace to calculate time attribution.
    </div>
    <template v-else>
      <header class="attribution-header">
        <div>
          <span class="attribution-kicker">Exclusive wall-clock attribution</span>
          <h2>Agent vs. model time</h2>
          <p>
            Model-side is the observable wait from request start through the final streamed
            response. It is not the model service's internal compute time.
          </p>
        </div>
        <span class="status-badge" :class="`status-${attribution.status}`">
          {{ attributionStatusLabel(attribution.status) }}
        </span>
      </header>

      <TimeAttributionBar :categories="attribution.categories" @select="openCategory" />

      <div class="category-grid">
        <button
          v-for="category in attribution.categories"
          :key="category.key"
          class="category-card"
          :class="{ focused: initialKey === category.key }"
          type="button"
          :disabled="!category.target"
          @click="openCategory(category)"
        >
          <span class="category-dot" :class="`dot-${category.key}`"></span>
          <span class="category-label">{{ category.label }}</span>
          <strong>{{ formatAttributionDuration(category.duration_nanos) }}</strong>
          <span>{{ formatAttributionPercent(category.percentage_bps) }}</span>
          <ExternalLink v-if="category.target" :size="14" aria-hidden="true" />
        </button>
      </div>

      <nav class="detail-tabs" aria-label="Attribution dimensions">
        <button
          v-for="tab in detailTabs"
          :key="tab.id"
          type="button"
          :class="{ active: activeDetail === tab.id }"
          @click="activeDetail = tab.id"
        >
          {{ tab.label }}
        </button>
      </nav>

      <p v-if="activeDetail === 'commands'" class="detail-note">
        Actual commands launched by an Agent Tool. A command includes its descendant process tree,
        so cargo time already includes rustc and linker work without double counting.
      </p>

      <section v-if="activeDetail === 'rounds'" class="detail-list">
        <article
          v-for="round in filteredRounds"
          :key="round.id"
          class="round-row"
          :class="{ focused: initialKey === round.id }"
        >
          <button class="row-heading" type="button" @click="openRound(round)">
            <span>
              <strong>{{ round.label }}</strong>
              <small class="round-boundary">{{ round.description }}</small>
              <small>
                {{ formatAttributionDuration(round.duration_nanos) }} total
                · {{ roundCallLabel(round) }}
              </small>
              <small>{{ roundCategorySummary(round) }}</small>
              <small v-if="round.models?.length || round.tools?.length" class="round-context">
                <template v-if="round.models?.length">
                  Models: {{ round.models.join(', ') }}
                </template>
                <template v-if="round.tools?.length">
                  <template v-if="round.models?.length"> · </template>
                  Tools: {{ round.tools.join(', ') }}
                </template>
              </small>
            </span>
            <ExternalLink :size="14" aria-hidden="true" />
          </button>
          <TimeAttributionBar
            :categories="round.categories"
            @select="(category) => openRoundCategory(round, category)"
          />
        </article>
        <div v-if="!filteredRounds.length" class="attribution-empty">No rounds match the filter.</div>
      </section>

      <section v-else class="detail-list">
        <button
          v-for="row in filteredBreakdown"
          :key="row.key"
          class="breakdown-row"
          :class="{ focused: initialKey === row.key }"
          type="button"
          :disabled="!row.target"
          @click="openBreakdown(row)"
        >
          <span>
            <strong>{{ row.label }}</strong>
            <small>{{ breakdownCountLabel(row) }}</small>
            <small v-if="activeDetail === 'commands' && row.agent_tools?.length">
              via Agent Tool: {{ row.agent_tools.join(', ') }}
            </small>
          </span>
          <span class="breakdown-duration">
            <strong>{{ formatAttributionDuration(row.duration_nanos) }}</strong>
            <small>{{ formatAttributionPercent(row.percentage_bps) }}</small>
          </span>
          <ExternalLink v-if="row.target" :size="14" aria-hidden="true" />
        </button>
        <div v-if="!filteredBreakdown.length" class="attribution-empty">
          No {{ activeDetail }} match the filter.
        </div>
      </section>

      <section v-if="attribution.issues?.length" class="issues-panel">
        <h3>Collection and attribution status</h3>
        <article
          v-for="(issue, index) in attribution.issues"
          :key="`${issue.code}-${issue.action_id ?? index}`"
          :class="`issue-${issue.severity}`"
        >
          <strong>{{ issue.code }}</strong>
          <span>{{ issue.message }}</span>
        </article>
      </section>

      <footer class="attribution-footnote">
        Agent-side, model-side observable, and unattributed always partition the selected Trace
        interval. Concurrent intervals are unioned before percentages are calculated.
      </footer>
    </template>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ExternalLink } from '@lucide/vue';

import TimeAttributionBar from '../../../components/time-attribution/TimeAttributionBar.vue';
import {
  ATTRIBUTION_COLORS,
  attributionStatusLabel,
  formatAttributionDuration,
  formatAttributionPercent,
  normalizeAttributionTarget,
  targetFromInterval,
} from '../../../components/time-attribution/model';

const props = defineProps({
  attribution: {
    type: Object,
    default: null,
  },
  query: {
    type: String,
    default: '',
  },
  initialDetail: {
    type: String,
    default: '',
  },
  initialKey: {
    type: String,
    default: '',
  },
});

const emit = defineEmits(['open-waterfall']);
const detailTabs = Object.freeze([
  { id: 'rounds', label: 'Rounds' },
  { id: 'models', label: 'Models' },
  { id: 'tools', label: 'Agent Tools' },
  { id: 'commands', label: 'Commands' },
]);
const activeDetail = ref('rounds');
const normalizedQuery = computed(() => props.query.trim().toLowerCase());
const filteredRounds = computed(() =>
  (props.attribution?.rounds ?? []).filter((round) =>
    matchesQuery([
      round.label,
      round.description,
      round.kind,
      ...(round.models ?? []),
      ...(round.tools ?? []),
    ]),
  ),
);
const filteredBreakdown = computed(() => {
  const rows = {
    models: props.attribution?.models,
    tools: props.attribution?.tools,
    commands: props.attribution?.commands,
  }[activeDetail.value];
  return (rows ?? []).filter((row) =>
    matchesQuery([row.label, row.key, row.kind, ...(row.agent_tools ?? [])]),
  );
});

watch(
  () => props.initialDetail,
  (dimension) => {
    const detail = {
      category: 'rounds',
      round: 'rounds',
      rounds: 'rounds',
      model: 'models',
      models: 'models',
      model_request: 'models',
      tool: 'tools',
      tools: 'tools',
      command: 'commands',
      commands: 'commands',
      command_occurrence: 'commands',
      unattributed_gap: 'rounds',
    }[dimension];
    if (detail) {
      activeDetail.value = detail;
    }
  },
  { immediate: true },
);

function matchesQuery(values) {
  if (!normalizedQuery.value) {
    return true;
  }
  return values.filter(Boolean).join(' ').toLowerCase().includes(normalizedQuery.value);
}

function openCategory(row) {
  const target = normalizeAttributionTarget(row?.target, {
    source: 'Trace Time Attribution',
    dimension: 'category',
    key: row.key,
    label: row.label,
    description: dominantIntervalDescription(row),
  });
  if (target) {
    emit('open-waterfall', target);
  }
}

function openRound(round) {
  const target = targetFromInterval(round, {
    source: 'Trace Time Attribution',
    dimension: 'round',
    key: round.id,
    label: round.label,
    description: round.description,
  });
  if (target) {
    emit('open-waterfall', target);
  }
}

function openRoundCategory(round, category) {
  const target = normalizeAttributionTarget(category?.target, {
    source: 'Trace Time Attribution',
    dimension: 'round',
    key: round.id,
    label: `${round.label} · ${category.label}`,
    description: [round.description, dominantIntervalDescription(category)]
      .filter(Boolean)
      .join(' · '),
  });
  if (target) {
    emit('open-waterfall', target);
  }
}

function openBreakdown(row) {
  const target = normalizeAttributionTarget(row?.target, {
    source: 'Trace Time Attribution',
    dimension: {
      models: 'model',
      tools: 'tool',
      commands: 'command',
    }[activeDetail.value],
    key: row.key,
    label: row.label,
    description: dominantIntervalDescription(row),
  });
  if (target) {
    emit('open-waterfall', target);
  }
}

function roundCallLabel(round) {
  const count = Number(round?.call_count ?? round?.action_ids?.length ?? 0);
  if (!count) {
    return 'no model calls';
  }
  return `${count} model ${count === 1 ? 'call' : 'calls'}`;
}

function roundCategorySummary(round) {
  return (round?.categories ?? [])
    .filter((category) => BigInt(category.duration_nanos ?? 0) > 0n)
    .map((category) => {
      const label = {
        agent_side: 'Agent',
        model_side: 'Model',
        unattributed: 'Unattributed',
      }[category.key] ?? category.label;
      return `${label} ${formatAttributionDuration(category.duration_nanos)} (${formatAttributionPercent(category.percentage_bps)})`;
    })
    .join(' · ');
}

function breakdownCountLabel(row) {
  if (activeDetail.value === 'commands' && row.kind === 'tool_overhead') {
    return `${row.segment_count} intervals · Agent Tool self-time`;
  }
  const noun = activeDetail.value === 'models'
    ? 'calls'
    : activeDetail.value === 'commands'
      ? 'command processes'
      : 'actions';
  return `${row.segment_count} intervals · ${row.action_count} ${noun}`;
}

function dominantIntervalDescription(row) {
  const count = Number(row?.segment_count ?? 0);
  if (count <= 1) {
    return '';
  }
  return `Longest contiguous interval shown · ${formatAttributionDuration(row.duration_nanos)} aggregate across ${count} intervals`;
}
</script>

<style scoped>
.time-attribution-tab {
  min-width: 0;
  min-height: 0;
  overflow: auto;
  display: grid;
  align-content: start;
  gap: var(--stats-space-xl, 20px);
  padding: var(--stats-viewport-padding, 24px);
  background: var(--stats-bg-gradient, none), var(--stats-bg-base, var(--bg));
}

.attribution-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--stats-space-xl, 20px);
}

.attribution-kicker {
  color: var(--stats-accent, var(--accent));
  font-size: var(--stats-font-xs, 12px);
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.attribution-header h2 {
  margin: var(--stats-space-xs, 4px) 0;
  color: var(--stats-text, var(--text));
}

.attribution-header p,
.attribution-footnote,
.detail-note {
  max-width: 760px;
  margin: 0;
  color: var(--stats-muted, var(--muted));
  font-size: var(--stats-font-sm, 13px);
  line-height: 1.55;
}

.detail-note {
  max-width: none;
  padding: var(--stats-space-md, 10px) var(--stats-space-lg, 14px);
  border-left: 3px solid var(--stats-accent, var(--accent));
  background: var(--stats-accent-muted, rgb(123 140 255 / 10%));
}

.status-badge {
  flex: 0 0 auto;
  padding: var(--stats-space-xs, 4px) var(--stats-space-md, 10px);
  border: 1px solid var(--stats-border, var(--border));
  border-radius: 999px;
  font-size: var(--stats-font-xs, 12px);
  text-transform: uppercase;
}

.status-complete {
  color: var(--stats-success, #45b783);
}

.status-provisional {
  color: var(--stats-accent, #7b8cff);
}

.status-partial,
.status-invalid {
  color: var(--stats-danger, #dc6673);
}

.category-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--stats-space-md, 10px);
}

.category-card,
.breakdown-row,
.row-heading {
  border: 1px solid var(--stats-border, var(--border));
  background: var(--stats-surface, var(--surface));
  color: var(--stats-text, var(--text));
  cursor: pointer;
}

.category-card {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--stats-space-sm, 8px);
  padding: var(--stats-space-lg, 14px);
  border-radius: var(--stats-radius-md, 10px);
  text-align: left;
}

.category-card > span:nth-of-type(2) {
  grid-column: 2;
  color: var(--stats-muted, var(--muted));
  font-size: var(--stats-font-sm, 13px);
}

.category-card > strong {
  grid-column: 2;
  font-size: var(--stats-font-display-sm, 20px);
}

.category-card > svg {
  grid-column: 3;
  grid-row: 1 / 4;
}

.category-card:disabled,
.breakdown-row:disabled {
  cursor: default;
}

.category-card:not(:disabled):hover,
.breakdown-row:not(:disabled):hover,
.row-heading:hover {
  border-color: var(--stats-accent-soft, var(--accent));
}

.category-card.focused,
.round-row.focused,
.breakdown-row.focused {
  border-color: var(--stats-accent, var(--accent));
  box-shadow: 0 0 0 2px var(--stats-accent-muted, rgb(123 140 255 / 15%));
}

.category-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
}

.dot-agent_side {
  background: v-bind("ATTRIBUTION_COLORS.agent_side");
}

.dot-model_side {
  background: v-bind("ATTRIBUTION_COLORS.model_side");
}

.dot-unattributed {
  background: v-bind("ATTRIBUTION_COLORS.unattributed");
}

.category-label {
  min-width: 0;
  font-size: var(--stats-font-sm, 13px);
}

.detail-tabs {
  display: inline-flex;
  width: fit-content;
  padding: var(--stats-space-2xs, 2px);
  border: 1px solid var(--stats-border, var(--border));
  border-radius: var(--stats-radius-sm, 8px);
  background: var(--stats-surface, var(--surface));
}

.detail-tabs button {
  min-height: 34px;
  padding: 0 var(--stats-space-lg, 14px);
  border: 0;
  border-radius: var(--stats-radius-sm, 8px);
  background: transparent;
  color: var(--stats-muted, var(--muted));
  cursor: pointer;
}

.detail-tabs button.active {
  background: var(--stats-accent-muted, rgb(123 140 255 / 15%));
  color: var(--stats-text, var(--text));
}

.detail-list {
  display: grid;
  gap: var(--stats-space-sm, 8px);
}

.round-row {
  display: grid;
  gap: var(--stats-space-md, 10px);
  padding: var(--stats-space-lg, 14px);
  border: 1px solid var(--stats-border, var(--border));
  border-radius: var(--stats-radius-md, 10px);
  background: var(--stats-surface, var(--surface));
}

.round-row :deep(.attribution-bar),
.round-row :deep(.attribution-bar-segment) {
  min-height: 24px;
}

.row-heading,
.breakdown-row {
  width: 100%;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--stats-space-md, 10px);
  text-align: left;
}

.row-heading {
  padding: 0;
  border: 0;
}

.row-heading span,
.breakdown-row span {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-2xs, 2px);
}

.row-heading small,
.breakdown-row small {
  color: var(--stats-muted, var(--muted));
}

.row-heading .round-boundary {
  color: var(--stats-text, var(--text));
  font-weight: 500;
}

.row-heading .round-context {
  text-transform: none;
}

.breakdown-row {
  grid-template-columns: minmax(0, 1fr) auto auto;
  padding: var(--stats-space-lg, 14px);
  border-radius: var(--stats-radius-md, 10px);
}

.breakdown-duration {
  justify-items: end;
}

.issues-panel {
  display: grid;
  gap: var(--stats-space-sm, 8px);
  padding: var(--stats-space-lg, 14px);
  border: 1px solid var(--stats-border, var(--border));
  border-radius: var(--stats-radius-md, 10px);
  background: var(--stats-surface, var(--surface));
}

.issues-panel h3 {
  margin: 0 0 var(--stats-space-xs, 4px);
}

.issues-panel article {
  display: grid;
  grid-template-columns: minmax(170px, auto) minmax(0, 1fr);
  gap: var(--stats-space-md, 10px);
  color: var(--stats-muted, var(--muted));
  font-size: var(--stats-font-sm, 13px);
}

.issues-panel article strong {
  color: var(--stats-text, var(--text));
}

.issue-error strong {
  color: var(--stats-danger, #dc6673) !important;
}

.attribution-empty {
  padding: var(--stats-space-2xl, 28px);
  color: var(--stats-muted, var(--muted));
  text-align: center;
}

@media (max-width: 760px) {
  .category-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .attribution-header {
    display: grid;
  }
}
</style>
