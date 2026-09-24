<template>
  <section class="time-waterfall-page">
    <section ref="attributionSection" class="time-waterfall-attribution">
      <TimeAttributionTab
        :attribution="attribution"
        :query="query"
        :initial-detail="focusInterval?.dimension ?? ''"
        :initial-key="focusInterval?.key ?? ''"
        @open-waterfall="$emit('open-waterfall', $event)"
      />
    </section>

    <section
      ref="waterfallSection"
      class="time-waterfall-section"
      aria-labelledby="waterfall-section-title"
    >
      <header class="time-waterfall-heading">
        <span>{{ t('kicker') }}</span>
        <h2 id="waterfall-section-title">{{ t('title') }}</h2>
      </header>
  <section class="tab-detail-layout waterfall-detail-layout" :class="{ 'detail-open': selectedDetail }">
    <section class="waterfall-panel tab-detail-main">
    <div class="waterfall-toolbar">
      <span class="wf-count">
        {{ waterfall.partial
          ? t('countActionsPartial', { visible: totalActions, total: waterfall.totalActions })
          : t('countActions', { count: totalActions }) }}
        <template v-if="windowText"> · {{ windowText }}</template>
      </span>
      <div class="wf-actions">
        <button
          v-if="waterfall.partial"
          type="button"
          class="tree-action"
          :title="t('loadAllTitle')"
          @click="$emit('load-full-waterfall')"
        >
          {{ t('loadAllActions') }}
        </button>
        <button
          type="button"
          class="tree-action"
          :disabled="!hasTree || queryActive"
          @click="expandAll"
        >
          <ChevronsUpDown :size="15" aria-hidden="true" />
          {{ t('expandAll') }}
        </button>
        <button
          type="button"
          class="tree-action"
          :disabled="!hasTree || queryActive"
          @click="collapseAll"
        >
          <ChevronsDownUp :size="15" aria-hidden="true" />
          {{ t('collapseAll') }}
        </button>
      </div>
    </div>

    <div
      v-if="zoomLabel || focusWindow"
      class="waterfall-breadcrumb"
      :class="{ 'attribution-focus': focusWindow && !zoomLabel }"
    >
      <Search :size="14" aria-hidden="true" />
      <span class="wf-location-copy">
        <strong class="wf-zoom-label">
          {{ zoomLabel ? `Zoomed: ${zoomLabel}` : focusTitle }}
        </strong>
        <small v-if="focusWindow && !zoomLabel">{{ focusDescription }}</small>
      </span>
      <span
        v-if="focusWindow && !zoomLabel"
        class="wf-focus-status"
        :class="{ matched: focusMatchCount > 0 }"
      >
        {{ focusMatchLabel }}
      </span>
      <span
        v-if="focusOccurrences.length > 1 && !zoomLabel"
        class="wf-occurrence-nav"
      >
        <button
          type="button"
          :title="t('previousOccurrence')"
          :disabled="focusOccurrenceIndex <= 0"
          @click="navigateOccurrence(-1)"
        >
          <ChevronLeft :size="13" aria-hidden="true" />
        </button>
        <small>
          {{ focusOccurrenceIndex + 1 }} / {{ focusOccurrences.length }}
        </small>
        <button
          type="button"
          :title="t('nextOccurrence')"
          :disabled="focusOccurrenceIndex < 0 || focusOccurrenceIndex + 1 >= focusOccurrences.length"
          @click="navigateOccurrence(1)"
        >
          <ChevronRight :size="13" aria-hidden="true" />
        </button>
      </span>
      <button
        v-if="focusWindow && !zoomLabel"
        type="button"
        class="wf-zoom-reset"
        @click="showAttribution"
      >
        {{ t('backToAttribution') }}
      </button>
      <button type="button" class="wf-zoom-reset" @click="resetView">
        {{ focusWindow && !zoomLabel ? t('showFullTrace') : t('resetView') }}
      </button>
    </div>

    <div v-if="groups.length || idleRows.length" class="waterfall-legend">
      <button
        v-for="group in groups"
        :key="group.group"
        type="button"
        class="wf-chip"
        :class="[`wf-group-${group.group}`, { inactive: !isGroupActive(group.group) }]"
        @click="toggleGroup(group.group)"
      >
        <span class="wf-chip-dot"></span>
        {{ group.group }}
        <small>{{ group.count }}</small>
      </button>
      <span
        v-if="idleRows.length"
        class="wf-chip wf-chip-idle"
        :title="t('idleIntervals', { count: idleRows.length })"
      >
        <span class="wf-chip-dot"></span>
        {{ t('userIdle') }}
        <small>{{ idleRows.length }}</small>
      </span>
      <div v-if="isGroupActive('llm')" class="wf-phase-legend" aria-hidden="true">
        <span class="wf-phase-key wf-bar-request">{{ t('phaseRequest') }}</span>
        <span class="wf-phase-key wf-bar-ttft">{{ t('phaseTtft') }}</span>
        <span class="wf-phase-key wf-bar-response">{{ t('phaseResponse') }}</span>
      </div>
      <small class="wf-navigation-hint">{{ t('navigationHint') }}</small>
    </div>

    <section v-if="bottleneckGroups.length" class="waterfall-bottlenecks">
      <header class="wf-bottleneck-header">
        <div class="wf-bottleneck-heading">
          <Gauge :size="16" aria-hidden="true" />
          <span>
            <strong>{{ t('bottlenecks') }}</strong>
            <small>Top {{ attribution?.bottlenecks?.default_display_limit ?? 5 }} per type by default; concurrent intervals may overlap.</small>
          </span>
        </div>
        <button
          type="button"
          class="wf-bottleneck-toggle"
          :aria-expanded="bottlenecksExpanded"
          @click="bottlenecksExpanded = !bottlenecksExpanded"
        >
          {{ bottlenecksExpanded ? 'Collapse' : 'Expand' }}
          <ChevronDown v-if="bottlenecksExpanded" :size="14" aria-hidden="true" />
          <ChevronRight v-else :size="14" aria-hidden="true" />
        </button>
      </header>
      <div v-if="bottlenecksExpanded" class="wf-bottleneck-grid">
        <article
          v-for="group in bottleneckGroups"
          :key="group.id"
          class="wf-bottleneck-group"
          :class="`wf-bottleneck-${group.id}`"
        >
          <header>
            <span class="wf-bottleneck-dot"></span>
            <strong>{{ group.label }}</strong>
            <small>{{ group.countLabel }}</small>
          </header>
          <div
            v-if="group.items.length"
            class="wf-bottleneck-list"
            :class="{ expanded: group.isExpanded }"
          >
            <button
              v-for="(item, index) in group.items"
              :key="item.id"
              type="button"
              class="wf-bottleneck-item"
              :class="{ active: isFocusedBottleneck(item) }"
              :title="t('locateInWaterfall', { label: item.label })"
              @click="focusBottleneck(group, item, index)"
            >
              <span class="wf-bottleneck-rank">{{ index + 1 }}</span>
              <span class="wf-bottleneck-copy">
                <span class="wf-bottleneck-name">
                  <strong>{{ item.label }}</strong>
                  <em v-if="item.status && item.status !== 'complete'">{{ bottleneckStatusLabel(item.status) }}</em>
                </span>
                <small>{{ item.description }} · {{ item.offsetLabel }}</small>
                <span class="wf-bottleneck-meter" aria-hidden="true">
                  <span :style="{ width: `${item.relativeWidth}%` }"></span>
                </span>
              </span>
              <span class="wf-bottleneck-duration">
                <strong>{{ formatAttributionDuration(item.duration_nanos) }}</strong>
                <small>{{ item.traceShare }}</small>
              </span>
            </button>
          </div>
          <div v-else class="wf-bottleneck-empty">{{ group.emptyLabel }}</div>
          <button
            v-if="group.canExpand"
            type="button"
            class="wf-bottleneck-more"
            :aria-expanded="group.isExpanded"
            @click="toggleBottleneckGroup(group.id)"
          >
            {{ group.isExpanded ? `Show top ${group.displayLimit}` : `Show all ${group.observedCount}` }}
            <ChevronUp v-if="group.isExpanded" :size="13" aria-hidden="true" />
            <ChevronDown v-else :size="13" aria-hidden="true" />
          </button>
        </article>
      </div>
    </section>

    <div
      v-if="rows.length || idleLaneSegments.length"
      ref="waterfallScroll"
      class="waterfall-scroll"
      :class="{ 'is-panning': timelinePanning }"
      :aria-label="t('timelineAria')"
      @wheel="handleTimelineWheel"
      @pointerdown="startTimelinePan"
    >
      <div class="waterfall-axis">
        <div class="wf-gutter">{{ t('gutterAction') }}</div>
        <div ref="axisTrack" class="wf-axis-track wf-time-track">
          <span
            v-if="focusBandStyle"
            class="wf-axis-focus"
            :style="focusBandStyle"
            :title="focusDescription"
          ></span>
          <span v-for="tick in ticks" :key="tick.pct" class="wf-tick" :style="{ left: `${tick.pct}%` }">
            {{ tick.label }}
          </span>
        </div>
      </div>

      <div v-if="attributionLanes.length" class="waterfall-attribution-lanes">
        <div
          v-for="lane in attributionLanes"
          :key="lane.id"
          class="wf-attribution-lane"
        >
          <div class="wf-gutter">
            {{ lane.label }}
            <small>{{ lane.segments.length }}</small>
          </div>
          <div class="wf-attribution-track wf-time-track">
            <button
              v-for="segment in lane.segments"
              :key="segment.id"
              type="button"
              class="wf-attribution-segment"
              :class="segment.className"
              :style="segment.style"
              :title="segment.title"
              @click="focusLaneSegment(segment)"
            >
              <span v-if="segment.showLabel">{{ segment.label }}</span>
            </button>
          </div>
        </div>
      </div>

      <div v-if="idleLaneSegments.length" class="waterfall-idle-lanes">
        <div v-for="lane in agentIdleLanes" :key="lane.id" class="wf-idle-lane">
          <div class="wf-gutter" :title="`Session: ${lane.sessionId}`">
            {{ lane.label }}
            <small>{{ lane.segments.length }}</small>
          </div>
          <div class="wf-idle-track wf-time-track">
            <span
              v-for="segment in lane.segments"
              :key="segment.id"
              class="wf-idle-segment"
              :class="{ live: segment.live, waiting: segment.kind === 'hang' }"
              :style="segment.style"
              :title="segment.title"
            ></span>
          </div>
        </div>
      </div>

      <div class="waterfall-rows" :class="{ 'is-focusing': focusActive }">
        <div
          v-for="row in rows"
          :key="row.id"
          v-memo="[row.id, row.expanded, row.barTitle, selectedDetailId, axisWindowKey, focusActionKey]"
          class="wf-row"
          :class="[row.kindClass, {
            selected: row.id === selectedDetailId,
            'attribution-match': isFocusAction(row.id),
          }]"
          :data-action-id="row.id"
          @click="select(row)"
          @dblclick="zoomTo(row)"
        >
          <div class="wf-label" :style="{ paddingLeft: `${row.depth * 16 + 10}px` }">
            <button
              v-if="row.hasChildren"
              type="button"
              class="wf-toggle"
              @click.stop="toggleRow(row)"
            >
              <ChevronDown v-if="row.expanded" :size="14" aria-hidden="true" />
              <ChevronRight v-else :size="14" aria-hidden="true" />
            </button>
            <span v-else class="wf-toggle-spacer"></span>
            <span class="wf-kind" aria-hidden="true"></span>
            <div class="wf-label-main">
              <div class="wf-label-line">
                <span class="wf-label-text" :title="row.label">{{ row.label }}</span>
                <span v-if="row.llmScope" class="wf-llm-scope" :title="row.llmScope">{{ row.llmScope }}</span>
                <span v-if="row.target" class="wf-label-target" :title="row.target">{{ row.target }}</span>
              </div>
              <div v-if="contextChip(row)" class="wf-agent-context" :title="row.agentContext">
                <span aria-hidden="true">↳</span>{{ contextChip(row) }}
              </div>
              <div class="wf-label-meta">
                <span class="wf-meta-start" :title="`start +${formatOffset(row.startOffsetMs)}`">
                  {{ row.startClock || row.startOffsetLabel }}
                </span>
                <DurationBadge :live="row.live">{{ row.durationText }}</DurationBadge>
              </div>
              <div
                v-if="row.llmRequestPreview || row.llmResponsePreview"
                class="wf-llm-messages"
              >
                <div
                  v-if="row.llmRequestPreview"
                  class="wf-llm-message wf-llm-message-request"
                  :title="row.llmMessages?.requestFull || row.llmRequestPreview"
                >
                  <span class="wf-llm-message-label">{{ t('roleUser') }}</span>
                  <span class="wf-llm-message-text">{{ row.llmRequestPreview }}</span>
                </div>
                <div
                  v-if="row.llmResponsePreview"
                  class="wf-llm-message wf-llm-message-response"
                  :title="row.llmMessages?.responseFull || row.llmResponsePreview"
                >
                  <span class="wf-llm-message-label">{{ t('roleAssistant') }}</span>
                  <span class="wf-llm-message-text">{{ row.llmResponsePreview }}</span>
                </div>
              </div>
            </div>
            <button
              v-if="row.hasChildren"
              type="button"
              class="wf-zoom"
              :title="t('zoomToSubtree')"
              @click.stop="zoomTo(row)"
            >
              <ZoomIn :size="13" aria-hidden="true" />
            </button>
          </div>
          <div class="wf-track wf-time-track">
            <span
              v-if="focusBandStyle"
              class="wf-focus-band"
              :class="{ linked: isFocusAction(row.id) }"
              :style="focusBandStyle"
              :title="isFocusAction(row.id) ? focusDescription : undefined"
            ></span>
            <template v-if="row.barSegments?.length">
              <div
                v-for="(segment, index) in row.barSegments"
                :key="`${row.id}-${segment.kind}-${index}`"
                class="wf-bar wf-bar-phase"
                :class="[
                  `wf-bar-${segment.kind}`,
                  `wf-status-${row.status}`,
                  { live: row.live && segment.kind !== 'ttft' },
                  { instant: segment.instant },
                ]"
                :style="segment.style"
                :title="row.barTitle"
              />
            </template>
            <div
              v-else-if="row.barStyle"
              class="wf-bar"
              :class="[
                row.barClass,
                `wf-status-${row.status}`,
                { live: row.live, instant: row.barInstant },
              ]"
              :style="row.barStyle"
              :title="row.barTitle"
            />
          </div>
        </div>

        <div v-if="hasMoreRows" class="wf-load-more-row">
          <button type="button" class="wf-load-more" @click="loadMore">
            Load {{ nextBatchSize }} more ({{ remainingRows }} hidden)
          </button>
          <button type="button" class="wf-load-all" @click="loadAll">{{ t('loadAll') }}</button>
        </div>
      </div>
    </div>

    <div v-else-if="modelBuilding && hasWaterfallData" class="waterfall-empty">{{ t('building') }}</div>
    <div v-else class="waterfall-empty">{{ t('noActions') }}</div>
    </section>
    <DetailPanel
      :detail="selectedDetail"
      :trace-id="traceKey"
      hide-when-empty
      @clear="clearDetail"
    />
  </section>
    </section>
  </section>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, watch } from 'vue';
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  ChevronsDownUp,
  ChevronsUpDown,
  Gauge,
  Search,
  ZoomIn,
} from '@lucide/vue';

import DetailPanel from '../../../components/DetailPanel.vue';
import DurationBadge from '../../../components/DurationBadge.vue';
import { formatAttributionDuration } from '../../../components/time-attribution/model';
import { useModuleLocale } from '../../../locale';
import TimeAttributionTab from '../time-attribution/TimeAttributionTab.vue';
import strings from './locale';

const { t } = useModuleLocale(strings);

import { TABLE_RENDER_LIMITS } from '../../tableConfig';
import { normalizeTableQuery } from '../../tableModel';
import { useTimelineNavigation } from '../time-navigation/useTimelineNavigation.js';
import {
  actionDetail,
  buildWaterfall,
  collectDefaultExpandedIds,
  collectParentIds,
  decorateWaterfallRows,
  defaultActiveGroups,
  emptyWaterfallModel,
  findWaterfallNode,
  findWaterfallPath,
  flattenMatchingWaterfall,
  flattenVisibleWaterfall,
  formatOffset,
  idleIntervalRows,
  panTimeViewport,
  projectTimeInterval,
  subtreeWindow,
  windowLabel,
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
    default: null,
  },
  waterfall: {
    type: Object,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
  focusInterval: {
    type: Object,
    default: null,
  },
  attribution: {
    type: Object,
    default: null,
  },
});

const emit = defineEmits(['open-waterfall', 'load-full-waterfall']);

const expandedIds = ref(new Set());
const activeGroups = ref(new Set());
const zoomId = ref(null);
const focusEnabled = ref(true);
const bottlenecksExpanded = ref(false);
const expandedBottleneckGroups = ref(new Set());
const visibleLimit = ref(TABLE_RENDER_LIMITS.initialRows);
const selectedDetailId = ref(null);
const selectedDetail = ref(null);
const attributionSection = ref(null);
const waterfallSection = ref(null);
const waterfallScroll = ref(null);
const axisTrack = ref(null);
const manualTimeViewport = ref(null);
const model = shallowRef(emptyWaterfallModel());
const modelBuilding = ref(false);
let modelBuildToken = 0;
let modelIdleHandle = null;

function showAttribution() {
  attributionSection.value?.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

const hasWaterfallData = computed(
  () =>
    (props.waterfall?.actions?.length ?? 0) > 0
    || (props.waterfall?.links?.length ?? 0) > 0
    || (props.waterfall?.idleIntervals?.length ?? 0) > 0,
);

function scheduleWaterfallBuild(actions, links, intervals, axisEnd) {
  modelBuildToken += 1;
  const token = modelBuildToken;
  if (modelIdleHandle !== null) {
    if (typeof cancelIdleCallback === 'function') {
      cancelIdleCallback(modelIdleHandle);
    } else {
      clearTimeout(modelIdleHandle);
    }
    modelIdleHandle = null;
  }
  if (!actions?.length && !links?.length && !intervals?.length) {
    model.value = emptyWaterfallModel();
    modelBuilding.value = false;
    return;
  }
  modelBuilding.value = true;
  const runBuild = () => {
    modelIdleHandle = null;
    if (token !== modelBuildToken) {
      return;
    }
    model.value = buildWaterfall(actions, links, intervals, axisEnd);
    modelBuilding.value = false;
  };
  if (typeof requestIdleCallback === 'function') {
    modelIdleHandle = requestIdleCallback(runBuild, { timeout: 120 });
  } else {
    modelIdleHandle = setTimeout(runBuild, 0);
  }
}

watch(
  () => [
    props.waterfall?.actions,
    props.waterfall?.links,
    props.waterfall?.idleIntervals,
    props.waterfall?.axisEnd,
  ],
  ([actions, links, intervals, axisEnd]) => {
    scheduleWaterfallBuild(actions, links, intervals, axisEnd);
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  modelBuildToken += 1;
  if (modelIdleHandle !== null) {
    if (typeof cancelIdleCallback === 'function') {
      cancelIdleCallback(modelIdleHandle);
    } else {
      clearTimeout(modelIdleHandle);
    }
  }
});

const roots = computed(() => model.value.roots);
const groups = computed(() => model.value.groups);
const window = computed(() => model.value.window);
const idleRows = computed(() =>
  idleIntervalRows(model.value.idleIntervals ?? [], window.value),
);
const totalActions = computed(() => model.value.totalActions);
const windowText = computed(() => windowLabel(window.value));
const parentIds = computed(() => collectParentIds(roots.value));
const hasTree = computed(() => parentIds.value.length > 0);
const normalizedQuery = computed(() => normalizeTableQuery(props.query));
const queryActive = computed(() => normalizedQuery.value.length > 0);

const zoomNode = computed(() =>
  zoomId.value ? findWaterfallNode(roots.value, zoomId.value) : null,
);
const zoomLabel = computed(() => {
  const node = zoomNode.value;
  if (!node) {
    return '';
  }
  return [node.label, node.target].filter(Boolean).join(' ');
});
const displayRoots = computed(() => (zoomNode.value ? [zoomNode.value] : roots.value));
const focusActionIds = computed(() =>
  Array.from(
    new Set(
      (props.focusInterval?.actionIds ?? [])
        .map((actionId) => String(actionId))
        .filter(Boolean),
    ),
  ),
);
const focusActionIdSet = computed(() => new Set(focusActionIds.value));
const focusActionKey = computed(() => focusActionIds.value.join('|'));
const focusPaths = computed(() =>
  focusActionIds.value
    .map((actionId) => findWaterfallPath(roots.value, actionId))
    .filter((path) => path.length),
);
const primaryFocusPath = computed(() => focusPaths.value[0] ?? []);
const focusMatchCount = computed(() => focusPaths.value.length);
const focusWindow = computed(() => {
  if (!focusEnabled.value || !props.focusInterval || !window.value?.startNanos) {
    return null;
  }
  try {
    const startNanos = BigInt(props.focusInterval.startNanos);
    const endNanos = BigInt(props.focusInterval.endNanos);
    if (endNanos <= startNanos) {
      return null;
    }
    return {
      startMs: Number(startNanos - window.value.startNanos) / 1_000_000,
      spanMs: Math.max(Number(endNanos - startNanos) / 1_000_000, 0.001),
    };
  } catch {
    return null;
  }
});
const focusTitle = computed(() => {
  const source = props.focusInterval?.source ?? t('source.attribution');
  const dimension = {
    category: t('dimensions.category'),
    round: t('dimensions.round'),
    model: t('dimensions.model'),
    model_request: t('dimensions.modelRequest'),
    tool: t('dimensions.tool'),
    command: t('dimensions.command'),
    command_occurrence: t('dimensions.commandOccurrence'),
    unattributed_gap: t('dimensions.unattributedGap'),
  }[props.focusInterval?.dimension];
  const context = [dimension, props.focusInterval?.label].filter(Boolean).join(' · ');
  return context ? `${source} · ${context}` : `${source} interval`;
});
const focusDescription = computed(() => {
  if (!focusWindow.value) {
    return '';
  }
  const start = focusWindow.value.startMs;
  const end = start + focusWindow.value.spanMs;
  const range = `+${formatOffset(start)} → +${formatOffset(end)}`;
  const duration = formatOffset(focusWindow.value.spanMs);
  return [props.focusInterval?.description, `${duration} · ${range}`]
    .filter(Boolean)
    .join(' · ');
});
const focusMatchLabel = computed(() => {
  if (!focusActionIds.value.length) {
    return t('timeRange');
  }
  if (!focusMatchCount.value) {
    return t('linkedUnavailable');
  }
  return t('linkedActions', {
    matched: focusMatchCount.value,
    total: focusActionIds.value.length,
    noun: focusActionIds.value.length === 1 ? t('linkedAction') : t('linkedActionPlural'),
  });
});
const focusOccurrences = computed(() =>
  attributionOccurrences(
    props.attribution,
    props.focusInterval?.dimension,
    props.focusInterval?.key,
  ),
);
const focusOccurrenceIndex = computed(() => {
  const start = String(props.focusInterval?.startNanos ?? '');
  const end = String(props.focusInterval?.endNanos ?? '');
  return focusOccurrences.value.findIndex(
    (occurrence) => occurrence.startNanos === start && occurrence.endNanos === end,
  );
});
const focusAxisWindow = computed(() =>
  focusWindow.value
    ? contextualFocusWindow(focusWindow.value, window.value.spanMs)
    : null,
);
const baseAxisWindow = computed(() =>
  zoomNode.value
    ? subtreeWindow(zoomNode.value, window.value.spanMs)
    : focusAxisWindow.value ?? { startMs: 0, spanMs: window.value.spanMs },
);
const axisWindow = computed(() => manualTimeViewport.value ?? baseAxisWindow.value);
const {
  panning: timelinePanning,
  zoomBy: zoomTimeline,
  resetViewport: resetTimeline,
  handleWheel: handleTimelineWheel,
  startPan: startTimelinePan,
} = useTimelineNavigation({
  viewport: manualTimeViewport,
  activeViewport: axisWindow,
  bounds: baseAxisWindow,
  surfaceRef: waterfallScroll,
  trackRef: axisTrack,
  trackSelector: '.wf-time-track',
  reset: () => {
    manualTimeViewport.value = null;
  },
});
watch(
  () => `${baseAxisWindow.value.startMs}:${baseAxisWindow.value.spanMs}`,
  () => {
    manualTimeViewport.value = null;
  },
);
const focusBandStyle = computed(() => {
  if (!focusWindow.value || zoomNode.value) {
    return null;
  }
  const projected = projectTimeInterval(
    focusWindow.value.startMs,
    focusWindow.value.startMs + focusWindow.value.spanMs,
    axisWindow.value,
  );
  if (!projected) {
    return null;
  }
  return {
    left: `${projected.leftPct}%`,
    width: `${projected.widthPct}%`,
  };
});
const attributionLanes = computed(() => {
  if (!props.attribution || !window.value?.startNanos) {
    return [];
  }
  return [
    {
      id: 'categories',
      label: t('lanes.agentModel'),
      segments: projectAttributionLane(
        props.attribution.segments,
        'category',
        axisWindow.value,
        window.value.startNanos,
      ),
    },
    {
      id: 'commands',
      label: t('lanes.commands'),
      segments: projectAttributionLane(
        props.attribution.command_segments,
        'command',
        axisWindow.value,
        window.value.startNanos,
      ),
    },
  ].filter((lane) => lane.segments.length);
});
const bottleneckGroups = computed(() => {
  const bottlenecks = props.attribution?.bottlenecks;
  if (!bottlenecks) {
    return [];
  }
  const displayLimit = Math.max(Number(bottlenecks.default_display_limit ?? 5), 1);
  const definitions = [
    {
      id: 'models',
      label: t('bottleneckGroup.models'),
      countNoun: t('countNoun.requests'),
      dimension: 'model_request',
      collection: bottlenecks.model_requests,
      emptyLabel: t('empty.models'),
    },
    {
      id: 'commands',
      label: t('bottleneckGroup.commands'),
      countNoun: t('countNoun.occurrences'),
      dimension: 'command_occurrence',
      collection: bottlenecks.commands,
      emptyLabel: t('empty.commands'),
    },
    {
      id: 'unattributed',
      label: t('bottleneckGroup.unattributed'),
      countNoun: t('countNoun.gaps'),
      dimension: 'unattributed_gap',
      collection: bottlenecks.unattributed_gaps,
      emptyLabel: t('empty.unattributed'),
    },
  ];
  return definitions.map((definition) => {
    const observedCount = Number(definition.collection?.observed_count ?? 0);
    const allItems = decorateBottleneckItems(
      definition.collection?.items,
      props.attribution?.scope?.duration_nanos,
      window.value?.startNanos,
    );
    const isExpanded = expandedBottleneckGroups.value.has(definition.id);
    const items = isExpanded ? allItems : allItems.slice(0, displayLimit);
    return {
      ...definition,
      observedCount,
      displayLimit,
      isExpanded,
      canExpand: allItems.length > displayLimit,
      countLabel: observedCount
        ? t('showing', { shown: items.length, total: observedCount, noun: definition.countNoun })
        : t('zeroCount', { noun: definition.countNoun }),
      items,
    };
  });
});
const axisWindowKey = computed(() => {
  const { startMs, spanMs } = axisWindow.value;
  return `${startMs}:${spanMs}`;
});
const ticks = computed(() => {
  const { startMs, spanMs } = axisWindow.value;
  return Array.from({ length: 5 }, (_, index) => {
    const fraction = index / 4;
    return { pct: fraction * 100, label: formatOffset(startMs + spanMs * fraction) };
  });
});

const allRows = computed(() => {
  const flat = queryActive.value && !focusWindow.value
    ? flattenMatchingWaterfall(displayRoots.value, normalizedQuery.value, activeGroups.value)
    : flattenVisibleWaterfall(displayRoots.value, expandedIds.value, activeGroups.value);
  return focusWindow.value && !zoomNode.value
    ? flat.filter((row) => rowOverlapsWindow(row, focusWindow.value))
    : flat;
});

const totalRows = computed(() => allRows.value.length);
const rows = computed(() => decorateWaterfallRows(
  allRows.value.slice(0, visibleLimit.value),
  axisWindow.value,
));
const idleLaneSegments = computed(() => {
  const segments = [];
  for (const interval of idleRows.value) {
    const style = idleSegmentStyle(interval);
    if (!style) {
      continue;
    }
    segments.push({ id: interval.id, style, title: idleTitle(interval), live: interval.live,
      kind: interval.kind, sessionId: interval.sessionId });
  }
  return segments;
});
const agentIdleLanes = computed(() => {
  const lanes = new Map();
  for (const segment of idleLaneSegments.value) {
    const id = JSON.stringify([segment.sessionId, segment.kind]);
    if (!lanes.has(id)) {
      lanes.set(id, { id, sessionId: segment.sessionId,
        label: t('agentStalled'),
        segments: [] });
    }
    lanes.get(id).segments.push(segment);
  }
  return [...lanes.values()];
});
const remainingRows = computed(() => Math.max(totalRows.value - rows.value.length, 0));
const nextBatchSize = computed(() => Math.min(TABLE_RENDER_LIMITS.rowBatchSize, remainingRows.value));
const hasMoreRows = computed(() => remainingRows.value > 0 && nextBatchSize.value > 0);

watch(
  model,
  (nextModel) => {
    clearDetail();
    expandedIds.value = new Set(collectDefaultExpandedIds(nextModel.roots));
    activeGroups.value = defaultActiveGroups(nextModel.groups);
    zoomId.value = null;
    queueFocusApplication();
  },
);

watch(
  () => props.focusInterval?.nonce,
  (nonce) => {
    focusEnabled.value = true;
    zoomId.value = null;
    queueFocusApplication();
    if (nonce) {
      nextTick(() => {
        waterfallSection.value?.scrollIntoView({ behavior: 'smooth', block: 'start' });
      });
    }
  },
  { immediate: true },
);

watch(
  () => props.attribution?.trace?.id,
  () => {
    bottlenecksExpanded.value = false;
    expandedBottleneckGroups.value = new Set();
  },
);

watch([displayRoots, normalizedQuery, activeGroups], () => {
  visibleLimit.value = TABLE_RENDER_LIMITS.initialRows;
});

function select(row) {
  selectedDetailId.value = row.id;
  const detail = actionDetail(row.action, {
    ...row.llmMessages,
    scope: row.llmScope,
    parent: row.agentContext,
    ttft: row.llmPhases?.gap?.durMs ? formatOffset(row.llmPhases.gap.durMs) : null,
  }, row.target);
  if (focusWindow.value && isFocusAction(row.id)) {
    detail.rows = {
      attribution: focusTitle.value,
      attributed_duration: formatOffset(focusWindow.value.spanMs),
      attributed_interval: focusDescription.value,
      ...detail.rows,
    };
  }
  selectedDetail.value = detail;
}

function clearDetail() {
  selectedDetailId.value = null;
  selectedDetail.value = null;
}

function isFocusAction(actionId) {
  return focusEnabled.value && focusActionIdSet.value.has(String(actionId));
}

const focusActive = computed(() => focusEnabled.value && focusActionIdSet.value.size > 0);

/**
 * Nested rows repeat the enclosing command line on every row, which reads as
 * noise. Only surface the context when it differs from the row's own target.
 */
function contextChip(row) {
  const context = String(row.agentContext ?? '').trim();
  if (!context || context === String(row.target ?? '').trim()) {
    return '';
  }
  return context.length > 32 ? `${context.slice(0, 31)}…` : context;
}

function idleSegmentStyle(interval) {
  const axis = axisWindow.value;
  // Open intervals end at the server-provided axis end. The surrounding
  // workspace refreshes the action-tree response to advance this boundary.
  const endMs = interval.endOffsetMs ?? axis.startMs + axis.spanMs;
  const projected = projectTimeInterval(interval.startOffsetMs, endMs, axis);
  if (!projected) {
    return null;
  }
  return {
    left: `${projected.leftPct}%`,
    width: `${projected.widthPct}%`,
  };
}

function idleTitle(interval) {
  const lines = [t('agentStalled')];
  lines.push(t('idle.session', { id: interval.sessionId }));
  if (interval.taskId) {
    lines.push(t('idle.task', { id: interval.taskId }));
  }
  lines.push(t('idle.start', { clock: nanosClock(interval.startNanos) }));
  lines.push(t('idle.end', { clock: interval.live ? t('idle.running') : nanosClock(interval.endNanos) }));
  lines.push(t('idle.duration', {
    value: interval.live ? t('idle.running') : formatOffset(interval.durMs),
  }));
  return lines.join('\n');
}

function nanosClock(nanos) {
  const millis = Number(BigInt(nanos) / 1000000n);
  return new Date(millis).toLocaleString();
}

function queueFocusApplication() {
  nextTick(applyAttributionFocus);
}

async function applyAttributionFocus() {
  if (!focusEnabled.value || !focusWindow.value || !primaryFocusPath.value.length) {
    return;
  }
  const nextExpanded = new Set(expandedIds.value);
  const nextGroups = new Set(activeGroups.value);
  for (const path of focusPaths.value) {
    for (const ancestor of path.slice(0, -1)) {
      if (ancestor.hasChildren) {
        nextExpanded.add(ancestor.id);
      }
    }
    for (const node of path) {
      nextGroups.add(node.kindGroup);
    }
  }
  expandedIds.value = nextExpanded;
  activeGroups.value = nextGroups;
  await nextTick();

  const targetRow = focusActionIds.value
    .map((actionId) => allRows.value.find((row) => row.id === actionId))
    .find(Boolean);
  if (!targetRow) {
    return;
  }
  const rowIndex = allRows.value.findIndex((row) => row.id === targetRow.id);
  if (rowIndex >= visibleLimit.value) {
    visibleLimit.value = rowIndex + 1;
    await nextTick();
  }
  select(targetRow);
  await nextTick();
  const element = Array.from(
    waterfallScroll.value?.querySelectorAll('.wf-row') ?? [],
  ).find((row) => row.dataset.actionId === targetRow.id);
  element?.scrollIntoView({ block: 'center', inline: 'nearest' });
}

function navigateOccurrence(delta) {
  const index = focusOccurrenceIndex.value + delta;
  const occurrence = focusOccurrences.value[index];
  if (!occurrence) {
    return;
  }
  emit('open-waterfall', {
    ...props.focusInterval,
    ...occurrence,
  });
}

function focusLaneSegment(segment) {
  emit('open-waterfall', segment.target);
}

function focusBottleneck(group, item, index) {
  emit('open-waterfall', {
    startNanos: item.start_unix_nanos,
    endNanos: item.end_unix_nanos,
    actionIds: Array.isArray(item.action_ids) ? item.action_ids : [],
    source: t('source.bottlenecks'),
    dimension: group.dimension,
    key: item.key,
    label: item.label,
    description: [
      `#${index + 1} longest ${group.label.toLowerCase()}`,
      item.description,
      bottleneckStatusDescription(item.status),
    ].filter(Boolean).join(' · '),
  });
}

function toggleBottleneckGroup(groupId) {
  const next = new Set(expandedBottleneckGroups.value);
  if (next.has(groupId)) {
    next.delete(groupId);
  } else {
    next.add(groupId);
  }
  expandedBottleneckGroups.value = next;
}

function isFocusedBottleneck(item) {
  return focusEnabled.value
    && String(item.start_unix_nanos) === String(props.focusInterval?.startNanos ?? '')
    && String(item.end_unix_nanos) === String(props.focusInterval?.endNanos ?? '');
}

function isGroupActive(group) {
  return activeGroups.value.has(group);
}

function toggleGroup(group) {
  const next = new Set(activeGroups.value);
  if (next.has(group)) {
    next.delete(group);
  } else {
    next.add(group);
  }
  activeGroups.value = next;
}

function toggleRow(row) {
  const next = new Set(expandedIds.value);
  if (next.has(row.id)) {
    next.delete(row.id);
  } else {
    next.add(row.id);
  }
  expandedIds.value = next;
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

function zoomTo(row) {
  if (!row.hasChildren) {
    return;
  }
  zoomId.value = row.id;
  const next = new Set(expandedIds.value);
  next.add(row.id);
  expandedIds.value = next;
}

function resetView() {
  resetTimeline();
  if (zoomId.value) {
    zoomId.value = null;
    queueFocusApplication();
    return;
  }
  focusEnabled.value = false;
  clearDetail();
  expandedIds.value = new Set(collectDefaultExpandedIds(roots.value));
  activeGroups.value = defaultActiveGroups(groups.value);
  visibleLimit.value = TABLE_RENDER_LIMITS.initialRows;
}

function rowOverlapsWindow(row, targetWindow) {
  const targetEnd = targetWindow.startMs + targetWindow.spanMs;
  const rowEnd = row.live
    ? Number.POSITIVE_INFINITY
    : row.startOffsetMs + Math.max(row.durMs ?? 0, 0);
  return row.startOffsetMs <= targetEnd && rowEnd >= targetWindow.startMs;
}

function contextualFocusWindow(targetWindow, globalSpanMs) {
  const minimumPadding = Math.min(Math.max(globalSpanMs * 0.01, 2), 25);
  const padding = Math.max(targetWindow.spanMs * 0.06, minimumPadding);
  const startMs = Math.max(targetWindow.startMs - padding, 0);
  const endMs = Math.min(
    targetWindow.startMs + targetWindow.spanMs + padding,
    globalSpanMs,
  );
  return {
    startMs,
    spanMs: Math.max(endMs - startMs, targetWindow.spanMs, 0.001),
  };
}

function attributionOccurrences(attribution, dimension, key) {
  if (!attribution || !dimension || !key) {
    return [];
  }
  let segments = [];
  if (dimension === 'command') {
    segments = (attribution.command_segments ?? []).filter(
      (segment) => segment.key === key,
    );
  } else if (dimension === 'model') {
    segments = (attribution.segments ?? []).filter(
      (segment) => segment.category === 'model_side' && segment.key === key,
    );
  } else if (dimension === 'tool') {
    segments = (attribution.segments ?? []).filter(
      (segment) => segment.category === 'agent_side' && segment.key === key,
    );
  } else if (dimension === 'category') {
    segments = (attribution.segments ?? []).filter(
      (segment) => segment.category === key,
    );
  } else if (dimension === 'model_request') {
    segments = (attribution.bottlenecks?.model_requests?.items ?? []).filter(
      (segment) => segment.key === key,
    );
  } else if (dimension === 'command_occurrence') {
    segments = (attribution.bottlenecks?.commands?.items ?? []).filter(
      (segment) => segment.key === key,
    );
  } else if (dimension === 'unattributed_gap') {
    segments = attribution.bottlenecks?.unattributed_gaps?.items ?? [];
  }
  return segments
    .map((segment) => ({
      startNanos: segment.start_unix_nanos,
      endNanos: segment.end_unix_nanos,
      actionIds: Array.isArray(segment.action_ids) ? segment.action_ids : [],
    }))
    .sort((left, right) => compareNanos(left.startNanos, right.startNanos));
}

function decorateBottleneckItems(items, traceDurationNanos, windowStartNanos) {
  const rows = items ?? [];
  const durations = rows.map((item) => parseNanos(item.duration_nanos));
  const longest = durations.reduce((current, duration) => (
    duration > current ? duration : current
  ), 0n);
  const traceDuration = parseNanos(traceDurationNanos);
  return rows.map((item, index) => {
    const duration = durations[index];
    return {
      ...item,
      offsetLabel: `+${formatOffset(nanosOffsetMs(item.start_unix_nanos, windowStartNanos))}`,
      relativeWidth: ratioPercent(duration, longest, 1),
      traceShare: `${ratioPercent(duration, traceDuration, 2).toFixed(2)}% Trace`,
    };
  });
}

function bottleneckStatusLabel(status) {
  return {
    in_progress: 'Live',
    partial: 'Partial',
    provisional: 'Provisional',
    error: 'Error',
  }[status] ?? status;
}

function bottleneckStatusDescription(status) {
  return status && status !== 'complete'
    ? `${bottleneckStatusLabel(status)} interval`
    : '';
}

function parseNanos(value) {
  try {
    return BigInt(value ?? 0);
  } catch {
    return 0n;
  }
}

function ratioPercent(value, total, decimals) {
  if (value <= 0n || total <= 0n) {
    return 0;
  }
  const scale = 10n ** BigInt(decimals);
  const scaled = (value * 100n * scale + total / 2n) / total;
  return Number(scaled) / Number(scale);
}

function projectAttributionLane(rows, laneKind, targetWindow, windowStartNanos) {
  const axisEnd = targetWindow.startMs + targetWindow.spanMs;
  return (rows ?? [])
    .map((row, index) => {
      const startMs = nanosOffsetMs(row.start_unix_nanos, windowStartNanos);
      const endMs = nanosOffsetMs(row.end_unix_nanos, windowStartNanos);
      const clippedStart = Math.max(startMs, targetWindow.startMs);
      const clippedEnd = Math.min(endMs, axisEnd);
      if (clippedEnd <= clippedStart) {
        return null;
      }
      const left = ((clippedStart - targetWindow.startMs) / targetWindow.spanMs) * 100;
      const width = ((clippedEnd - clippedStart) / targetWindow.spanMs) * 100;
      const context = laneTargetContext(row, laneKind);
      return {
        id: `${laneKind}-${row.id ?? row.key ?? index}-${index}`,
        label: row.label,
        className: laneKind === 'command'
          ? `lane-command-${row.kind}`
          : `lane-category-${row.category}`,
        style: {
          left: `${left}%`,
          width: `${width}%`,
        },
        showLabel: width >= 7,
        title: `${row.label} · ${formatAttributionDuration(row.duration_nanos)}`,
        target: {
          startNanos: row.start_unix_nanos,
          endNanos: row.end_unix_nanos,
          actionIds: Array.isArray(row.action_ids) ? row.action_ids : [],
          source: t('source.lane'),
          ...context,
        },
      };
    })
    .filter(Boolean);
}

function laneTargetContext(row, laneKind) {
  if (laneKind === 'command') {
    return {
      dimension: 'command',
      key: row.key,
      label: row.label,
    };
  }
  if (row.category === 'model_side') {
    return {
      dimension: 'model',
      key: row.key,
      label: row.label,
    };
  }
  if (row.category === 'agent_side' && row.subcategory !== 'orchestration') {
    return {
      dimension: 'tool',
      key: row.key,
      label: row.label,
    };
  }
  return {
    dimension: 'category',
    key: row.category,
    label: row.label,
  };
}

function nanosOffsetMs(value, origin) {
  try {
    return Number(BigInt(value) - BigInt(origin)) / 1_000_000;
  } catch {
    return 0;
  }
}

function compareNanos(left, right) {
  try {
    const leftNanos = BigInt(left);
    const rightNanos = BigInt(right);
    return leftNanos < rightNanos ? -1 : leftNanos > rightNanos ? 1 : 0;
  } catch {
    return String(left).localeCompare(String(right));
  }
}
</script>
<style src="./waterfall.css" scoped></style>
