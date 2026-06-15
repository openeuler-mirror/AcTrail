<template>
  <section class="tab-detail-layout">
    <section class="waterfall-panel tab-detail-main">
    <div class="waterfall-toolbar">
      <span class="wf-count">
        {{ totalActions }} actions
        <template v-if="windowText"> · {{ windowText }}</template>
      </span>
      <div class="wf-actions">
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

    <div v-if="zoomLabel" class="waterfall-breadcrumb">
      <Search :size="14" aria-hidden="true" />
      <span class="wf-zoom-label">Zoomed: {{ zoomLabel }}</span>
      <button type="button" class="wf-zoom-reset" @click="resetZoom">Reset zoom</button>
    </div>

    <div v-if="groups.length" class="waterfall-legend">
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
      <div v-if="isGroupActive('llm')" class="wf-phase-legend" aria-hidden="true">
        <span class="wf-phase-key wf-bar-request">req</span>
        <span class="wf-phase-key wf-bar-ttft">ttft</span>
        <span class="wf-phase-key wf-bar-response">res</span>
      </div>
    </div>

    <div v-if="rows.length" class="waterfall-scroll">
      <div class="waterfall-axis">
        <div class="wf-gutter">Action</div>
        <div class="wf-axis-track">
          <span v-for="tick in ticks" :key="tick.pct" class="wf-tick" :style="{ left: `${tick.pct}%` }">
            {{ tick.label }}
          </span>
        </div>
      </div>

      <div class="waterfall-rows">
        <div
          v-for="row in rows"
          :key="row.id"
          v-memo="[row.id, row.expanded, row.barTitle, selectedDetailId, axisWindowKey]"
          class="wf-row"
          :class="{ selected: row.id === selectedDetailId }"
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
            <div class="wf-label-main">
              <div class="wf-label-line">
                <span class="wf-label-text" :title="row.label">{{ row.label }}</span>
                <span v-if="row.llmScope" class="wf-llm-scope" :title="row.llmScope">{{ row.llmScope }}</span>
                <span v-if="row.target" class="wf-label-target" :title="row.target">{{ row.target }}</span>
              </div>
              <div v-if="row.agentContext" class="wf-agent-context" :title="row.agentContext">
                under {{ row.agentContext }}
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
                  <span class="wf-llm-message-label">user</span>
                  <span class="wf-llm-message-text">{{ row.llmRequestPreview }}</span>
                </div>
                <div
                  v-if="row.llmResponsePreview"
                  class="wf-llm-message wf-llm-message-response"
                  :title="row.llmMessages?.responseFull || row.llmResponsePreview"
                >
                  <span class="wf-llm-message-label">assistant</span>
                  <span class="wf-llm-message-text">{{ row.llmResponsePreview }}</span>
                </div>
              </div>
            </div>
            <button
              v-if="row.hasChildren"
              type="button"
              class="wf-zoom"
              title="Zoom to this subtree"
              @click.stop="zoomTo(row)"
            >
              <ZoomIn :size="13" aria-hidden="true" />
            </button>
          </div>
          <div class="wf-track">
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
              v-else
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
          <button type="button" class="wf-load-all" @click="loadAll">Load all</button>
        </div>
      </div>
    </div>

    <div v-else-if="modelBuilding && hasWaterfallData" class="waterfall-empty">Building chart…</div>
    <div v-else class="waterfall-empty">No actions to chart</div>
    </section>
    <DetailPanel :detail="selectedDetail" :trace-id="traceKey" @clear="clearDetail" />
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { ChevronDown, ChevronRight, ChevronsDownUp, ChevronsUpDown, Search, ZoomIn } from '@lucide/vue';

import DetailPanel from '../../../components/DetailPanel.vue';
import DurationBadge from '../../../components/DurationBadge.vue';
import { TABLE_RENDER_LIMITS } from '../../tableConfig';
import { normalizeTableQuery } from '../../tableModel';
import {
  actionDetail,
  buildWaterfall,
  collectDefaultExpandedIds,
  collectParentIds,
  decorateWaterfallRows,
  defaultActiveGroups,
  emptyWaterfallModel,
  findWaterfallNode,
  flattenMatchingWaterfall,
  flattenVisibleWaterfall,
  formatOffset,
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
});

const expandedIds = ref(new Set());
const activeGroups = ref(new Set());
const zoomId = ref(null);
const visibleLimit = ref(TABLE_RENDER_LIMITS.initialRows);
const selectedDetailId = ref(null);
const selectedDetail = ref(null);
const model = ref(emptyWaterfallModel());
const modelBuilding = ref(false);
let modelBuildToken = 0;
let modelIdleHandle = null;

const hasWaterfallData = computed(
  () => (props.waterfall?.actions?.length ?? 0) > 0 || (props.waterfall?.links?.length ?? 0) > 0,
);

function scheduleWaterfallBuild(actions, links) {
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
  if (!actions?.length && !links?.length) {
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
    model.value = buildWaterfall(actions, links);
    modelBuilding.value = false;
  };
  if (typeof requestIdleCallback === 'function') {
    modelIdleHandle = requestIdleCallback(runBuild, { timeout: 120 });
  } else {
    modelIdleHandle = setTimeout(runBuild, 0);
  }
}

watch(
  () => [props.waterfall?.actions, props.waterfall?.links],
  ([actions, links]) => {
    scheduleWaterfallBuild(actions, links);
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
const axisWindow = computed(() =>
  zoomNode.value
    ? subtreeWindow(zoomNode.value, window.value.spanMs)
    : { startMs: 0, spanMs: window.value.spanMs },
);
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
  const flat = queryActive.value
    ? flattenMatchingWaterfall(displayRoots.value, normalizedQuery.value, activeGroups.value)
    : flattenVisibleWaterfall(displayRoots.value, expandedIds.value, activeGroups.value);
  return decorateWaterfallRows(flat, axisWindow.value);
});

const totalRows = computed(() => allRows.value.length);
const rows = computed(() => allRows.value.slice(0, visibleLimit.value));
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
  },
);

watch([displayRoots, normalizedQuery, activeGroups], () => {
  visibleLimit.value = TABLE_RENDER_LIMITS.initialRows;
});

function select(row) {
  selectedDetailId.value = row.id;
  selectedDetail.value = actionDetail(row.action, {
    ...row.llmMessages,
    scope: row.llmScope,
    parent: row.agentContext,
    ttft: row.llmPhases?.gap?.durMs ? formatOffset(row.llmPhases.gap.durMs) : null,
  });
}

function clearDetail() {
  selectedDetailId.value = null;
  selectedDetail.value = null;
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

function resetZoom() {
  zoomId.value = null;
}
</script>
<style src="./waterfall.css" scoped></style>
