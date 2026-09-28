<template>
  <section class="time-attribution-tab">
    <div v-if="!attribution" class="attribution-empty">
      {{ t('selectTrace') }}
    </div>
    <template v-else>
      <header class="attribution-header" :class="{ 'attribution-header-compact': !hasUserTurns }">
        <div>
          <span class="attribution-kicker">{{ t('kicker') }}</span>
          <h2>{{ t('title') }}</h2>
          <p>{{ t('headerNote') }}</p>
        </div>
        <span class="status-badge" :class="`status-${attribution.status}`">
          {{ attributionStatusLabel(attribution.status) }}
        </span>
      </header>

      <TimeAttributionBar
        v-if="hasUserTurns"
        :categories="attribution.categories"
        @select="openCategory"
      />

      <p v-if="hasUserTurns" class="detail-note coverage-note">
        {{ attributionCoverageLabel(attribution.coverage) }}
      </p>

      <p v-else class="attribution-compact-note">
        {{ t('emptyNote') }}
        <span class="attribution-compact-coverage">{{ attributionCoverageLabel(attribution.coverage) }}</span>
      </p>

      <div v-if="hasUserTurns" class="category-grid">
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

      <nav v-if="hasUserTurns" class="detail-tabs" :aria-label="t('dimensionsAria')">
        <button
          v-for="tab in localizedDetailTabs"
          :key="tab.id"
          type="button"
          :class="{ active: activeDetail === tab.id }"
          @click="activeDetail = tab.id"
        >
          {{ tab.label }}
        </button>
      </nav>

      <p v-if="hasUserTurns && activeDetail === 'commands'" class="detail-note">
        {{ t('commandsNote') }}
      </p>

      <p v-if="hasUserTurns && activeDetail === 'tools'" class="detail-note">
        {{ t('toolsNote') }}
      </p>

      <section v-if="hasUserTurns && activeDetail === 'rounds'" class="detail-list">
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
        <div v-if="!filteredRounds.length" class="attribution-empty">
          {{ t('noRequests') }}
        </div>
      </section>

      <section v-else-if="hasUserTurns" class="detail-list">
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
              {{ t('viaTool', { tools: row.agent_tools.join(', ') }) }}
            </small>
          </span>
          <span class="breakdown-duration">
            <strong>{{ formatAttributionDuration(row.duration_nanos) }}</strong>
            <small>{{ formatAttributionPercent(row.percentage_bps) }}</small>
          </span>
          <ExternalLink v-if="row.target" :size="14" aria-hidden="true" />
        </button>
        <div v-if="!filteredBreakdown.length" class="attribution-empty">
          {{ t('noMatch', { kind: activeDetail }) }}
        </div>
      </section>

      <section v-if="hasUserTurns && attribution.issues?.length" class="issues-panel">
        <h3>{{ t('collectionStatus') }}</h3>
        <article
          v-for="(issue, index) in groupedIssues"
          :key="`${issue.code}-${issue.action_id ?? index}`"
          :class="`issue-${issue.severity}`"
        >
          <strong>{{ issue.code }}</strong>
          <span>{{ issue.message }}</span>
          <small v-if="issue.count > 1" class="issue-count">
            {{ t('occurrences', { count: issue.count }) }}
          </small>
        </article>
      </section>

      <footer v-if="hasUserTurns" class="attribution-footnote">
        {{ t('standardNote') }}
      </footer>
    </template>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ExternalLink } from '@lucide/vue';

import TimeAttributionBar from '../../../components/time-attribution/TimeAttributionBar.vue';
import { useModuleLocale } from '../../../locale';
import strings from './locale';

const { t } = useModuleLocale(strings);

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
  { id: 'rounds', labelKey: 'userRequests' },
  { id: 'models', labelKey: 'models' },
  { id: 'tools', labelKey: 'agentTools' },
  { id: 'commands', labelKey: 'commands' },
]);
const localizedDetailTabs = computed(() =>
  detailTabs.map((tab) => ({ ...tab, label: t(tab.labelKey) })),
);
const activeDetail = ref('rounds');
const normalizedQuery = computed(() => props.query.trim().toLowerCase());
const hasUserTurns = computed(() => {
  const count = Number(props.attribution?.coverage?.user_turn_count ?? 0);
  try {
    return count > 0 && BigInt(props.attribution?.scope?.duration_nanos ?? 0) > 0n;
  } catch {
    return false;
  }
});
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
const groupedIssues = computed(() => {
  const groups = new Map();
  for (const issue of props.attribution?.issues ?? []) {
    const key = `${issue.code}\u0000${issue.message}`;
    const group = groups.get(key);
    if (group) {
      group.count += 1;
    } else {
      groups.set(key, { ...issue, count: 1 });
    }
  }
  return Array.from(groups.values());
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
    source: t('source'),
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
    source: t('source'),
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
    source: t('source'),
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
    source: t('source'),
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
    return t('noModelCalls');
  }
  return t('modelCalls', { count });
}

function attributionCoverageLabel(coverage) {
  const requests = Number(coverage?.llm_request_count ?? 0);
  const responses = Number(coverage?.llm_response_count ?? 0);
  const observedCalls = Number(coverage?.observed_llm_call_count ?? 0);
  const pairedCalls = Number(coverage?.paired_llm_call_count ?? 0);
  const unpairedCalls = Number(coverage?.unpaired_llm_call_count ?? 0);
  const orphanResponses = Number(coverage?.orphan_llm_response_count ?? 0);
  const attributedCalls = Number(coverage?.attributed_llm_call_count ?? 0);
  const excludedCalls = Number(coverage?.excluded_from_attribution_llm_call_count ?? 0);
  const userTurns = Number(coverage?.user_turn_count ?? 0);
  const inputBoundaries = Number(coverage?.strong_user_input_count ?? 0);
  return [
    t('coverage.userRequests', { count: userTurns }),
    t('coverage.inputBoundaries', { count: inputBoundaries }),
    t('coverage.llmRequests', { count: requests }),
    t('coverage.llmResponses', { count: responses }),
    t('coverage.structurallyPaired', { observed: observedCalls, paired: pairedCalls }),
    t('coverage.requestOnly', { count: unpairedCalls }),
    t('coverage.orphanResponses', { count: orphanResponses }),
    t('coverage.pairedAttributed', { count: attributedCalls }),
    t('coverage.pairedExcluded', { count: excludedCalls }),
  ].join(' · ');
}

function roundCategorySummary(round) {
  return (round?.categories ?? [])
    .filter((category) => BigInt(category.duration_nanos ?? 0) > 0n)
    .map((category) => {
      const label = {
        agent_side: t('category.agent'),
        model_side: t('category.model'),
        unattributed: t('category.unattributed'),
      }[category.key] ?? category.label;
      return `${label} ${formatAttributionDuration(category.duration_nanos)} (${formatAttributionPercent(category.percentage_bps)})`;
    })
    .join(' · ');
}

function breakdownCountLabel(row) {
  if (activeDetail.value === 'commands' && row.kind === 'tool_overhead') {
    return t('coverage.toolIntervals', { count: row.segment_count });
  }
  const noun = activeDetail.value === 'models'
    ? t('noun.calls')
    : activeDetail.value === 'commands'
      ? t('noun.commands')
      : t('noun.actions');
  return t('breakdownCount', { intervals: row.segment_count, actions: row.action_count, noun });
}

function dominantIntervalDescription(row) {
  const count = Number(row?.segment_count ?? 0);
  if (count <= 1) {
    return '';
  }
  return t('focus.aggregate', {
    duration: formatAttributionDuration(row.duration_nanos),
    count,
  });
}
</script>

<style scoped>
.time-attribution-tab {
  min-width: 0;
  min-height: 0;
  overflow: auto;
  display: grid;
  align-content: start;
  gap: var(--ui-space-xl, 20px);
  padding: var(--ui-viewport-padding, 24px);
  background: var(--ui-bg-base);
}

.attribution-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--ui-space-xl, 20px);
}

.attribution-header-compact > div {
  display: flex;
  align-items: baseline;
  gap: var(--ui-space-md);
}

.attribution-header-compact .attribution-kicker {
  color: var(--ui-muted);
  font-size: 10px;
}

.attribution-header-compact h2 {
  margin: 0;
  font-size: var(--ui-font-title);
}

.attribution-header-compact p {
  display: none;
}

.attribution-compact-note {
  max-width: 92ch;
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  line-height: 1.5;
}

.attribution-compact-coverage {
  display: block;
  margin-top: var(--ui-space-2xs);
  color: var(--ui-text-tertiary);
  font-size: var(--ui-font-xs);
}

.attribution-kicker {
  color: var(--ui-accent, var(--ui-accent));
  font-size: var(--ui-font-xs, 12px);
  font-weight: 600;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.attribution-header h2 {
  margin: var(--ui-space-xs, 4px) 0;
  color: var(--ui-text, var(--ui-text));
}

.attribution-header p,
.attribution-footnote,
.detail-note {
  max-width: 760px;
  margin: 0;
  color: var(--ui-muted, var(--ui-muted));
  font-size: var(--ui-font-sm, 13px);
  line-height: 1.55;
}

.coverage-note {
  margin-top: calc(var(--ui-space-lg, 16px) * -1);
}

.detail-note {
  max-width: none;
  padding: var(--ui-space-md, 10px) var(--ui-space-lg, 14px);
  border-left: 3px solid var(--ui-accent, var(--ui-accent));
  background: var(--ui-accent-muted, rgb(123 140 255 / 10%));
}

.status-badge {
  flex: 0 0 auto;
  padding: var(--ui-space-xs, 4px) var(--ui-space-md, 10px);
  border: 1px solid var(--ui-border, var(--ui-border));
  border-radius: 999px;
  font-size: var(--ui-font-xs, 12px);
  text-transform: uppercase;
}

.status-complete {
  color: var(--ui-success, #45b783);
}

.status-provisional {
  color: var(--ui-accent, #7b8cff);
}

.status-partial,
.status-invalid {
  color: var(--ui-danger, #dc6673);
}

.category-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: var(--ui-space-md, 10px);
}

.category-card,
.breakdown-row,
.row-heading {
  border: 1px solid var(--ui-border, var(--ui-border));
  background: var(--ui-surface, var(--ui-surface));
  color: var(--ui-text, var(--ui-text));
  cursor: pointer;
}

.category-card {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--ui-space-sm, 8px);
  padding: var(--ui-space-lg, 14px);
  border-radius: var(--ui-radius-md, 10px);
  text-align: left;
}

.category-card > span:nth-of-type(2) {
  grid-column: 2;
  color: var(--ui-muted, var(--ui-muted));
  font-size: var(--ui-font-sm, 13px);
}

.category-card > strong {
  grid-column: 2;
  font-size: var(--ui-font-display-sm, 20px);
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
  border-color: var(--ui-accent-soft, var(--ui-accent));
}

.category-card.focused,
.round-row.focused,
.breakdown-row.focused {
  border-color: var(--ui-accent, var(--ui-accent));
  box-shadow: 0 0 0 2px var(--ui-accent-muted, rgb(123 140 255 / 15%));
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
  font-size: var(--ui-font-sm, 13px);
}

.detail-tabs {
  display: inline-flex;
  width: fit-content;
  padding: var(--ui-space-2xs, 2px);
  border: 1px solid var(--ui-border, var(--ui-border));
  border-radius: var(--ui-radius-sm, 8px);
  background: var(--ui-surface, var(--ui-surface));
}

.detail-tabs button {
  min-height: 34px;
  padding: 0 var(--ui-space-lg, 14px);
  border: 0;
  border-radius: var(--ui-radius-sm, 8px);
  background: transparent;
  color: var(--ui-muted, var(--ui-muted));
  cursor: pointer;
}

.detail-tabs button.active {
  background: var(--ui-accent-muted, rgb(123 140 255 / 15%));
  color: var(--ui-text, var(--ui-text));
}

.detail-list {
  display: grid;
  gap: var(--ui-space-sm, 8px);
}

.round-row {
  display: grid;
  gap: var(--ui-space-md, 10px);
  padding: var(--ui-space-lg, 14px);
  border: 1px solid var(--ui-border, var(--ui-border));
  border-radius: var(--ui-radius-md, 10px);
  background: var(--ui-surface, var(--ui-surface));
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
  gap: var(--ui-space-md, 10px);
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
  gap: var(--ui-space-2xs, 2px);
}

.row-heading small,
.breakdown-row small {
  color: var(--ui-muted, var(--ui-muted));
}

.row-heading .round-boundary {
  color: var(--ui-text, var(--ui-text));
  font-weight: 500;
}

.row-heading .round-context {
  text-transform: none;
}

.breakdown-row {
  grid-template-columns: minmax(0, 1fr) auto auto;
  padding: var(--ui-space-lg, 14px);
  border-radius: var(--ui-radius-md, 10px);
}

.breakdown-duration {
  justify-items: end;
}

.issues-panel {
  display: grid;
  gap: var(--ui-space-sm, 8px);
  padding: var(--ui-space-lg, 14px);
  border: 1px solid var(--ui-border, var(--ui-border));
  border-radius: var(--ui-radius-md, 10px);
  background: var(--ui-surface, var(--ui-surface));
}

.issues-panel h3 {
  margin: 0 0 var(--ui-space-xs, 4px);
}

.issues-panel article {
  display: grid;
  grid-template-columns: minmax(170px, auto) minmax(0, 1fr);
  gap: var(--ui-space-md, 10px);
  color: var(--ui-muted, var(--ui-muted));
  font-size: var(--ui-font-sm, 13px);
}

.issues-panel article strong {
  color: var(--ui-text, var(--ui-text));
}

.issue-error strong {
  color: var(--ui-danger, #dc6673) !important;
}

.no-user-turn-state {
  display: grid;
  gap: var(--ui-space-sm, 8px);
  padding: var(--ui-space-xl, 20px);
  border: 1px dashed var(--ui-border, var(--ui-border));
  border-radius: var(--ui-radius-lg, 14px);
  background: var(--ui-surface, var(--ui-surface));
  color: var(--ui-muted, var(--ui-muted));
  line-height: 1.55;
}

.no-user-turn-state strong {
  color: var(--ui-text, var(--ui-text));
}

.attribution-empty {
  padding: var(--ui-space-2xl, 28px);
  color: var(--ui-muted, var(--ui-muted));
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
