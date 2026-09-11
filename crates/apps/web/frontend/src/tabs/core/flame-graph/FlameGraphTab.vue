<template>
  <section class="tab-detail-layout flame-detail-layout" :class="{ 'detail-open': detailPanelVisible }">
    <section class="flame-panel tab-detail-main">
      <header class="flame-toolbar">
        <div class="flame-heading">
          <span class="flame-kicker">Execution profile</span>
          <div>
            <h2>Flame Graph</h2>
            <small>
              {{ model.totalActivities }} activities · {{ formatOffset(model.window.spanMs) }} observed
            </small>
          </div>
        </div>
        <div class="flame-toolbar-actions">
          <button
            v-if="waterfall.partial"
            type="button"
            class="tree-action"
            title="Load file, protocol, and runtime actions"
            @click="$emit('load-full-waterfall')"
          >
            Load full trace
          </button>
          <div class="flame-zoom-controls" aria-label="Timeline zoom controls">
            <button type="button" title="Zoom out" @click="zoomBy(1 / 1.4)">
              <Minus :size="15" aria-hidden="true" />
            </button>
            <button type="button" title="Reset timeline" @click="resetViewport">
              <RotateCcw :size="14" aria-hidden="true" />
            </button>
            <button type="button" title="Zoom in" @click="zoomBy(1.4)">
              <Plus :size="15" aria-hidden="true" />
            </button>
          </div>
        </div>
      </header>

      <div class="flame-legend" aria-label="Layer legend">
        <span><i class="agent"></i> Agent: main loop + associated subagent loops</span>
        <span><i class="harness"></i> Harness: background-tagged + unclaimed framework work</span>
        <small>Ctrl + wheel or W/S zoom · A/D pan · drag to move · 0 reset</small>
      </div>

      <div v-if="unavailableRequestCount" class="flame-layer-empty">
        {{ unavailableRequestCount }} request bodies unavailable. Agent grouping and background classification may be incomplete.
      </div>

      <div
        v-if="model.totalActivities"
        ref="navigationSurface"
        class="flame-navigation"
        :class="{ 'is-panning': panning }"
        aria-label="Flame Graph timeline. Scroll vertically to explore rows. Hold Control while scrolling or use W and S to zoom, A and D to move, and 0 to reset."
      >
        <TimelineOverview
          :bounds="bounds"
          :viewport="activeViewport"
          :lanes="overviewLanes"
          :end-label="formatOffset(bounds.spanMs)"
          @update:viewport="setOverviewViewport"
          @zoom="handleOverviewZoom"
        />
        <div
          class="flame-scroll"
          @wheel="handleWheel"
          @pointerdown="startPan"
        >
          <div class="flame-axis">
            <div class="fg-gutter">
              <span>Lane</span>
              <small>{{ viewportLabel }}</small>
            </div>
            <div ref="axisTrack" class="fg-axis-track fg-time-track">
              <span
                v-for="tick in ticks"
                :key="tick.pct"
                class="fg-tick"
                :style="{ left: `${tick.pct}%` }"
              >
                {{ tick.label }}
              </span>
            </div>
          </div>

          <section
            v-for="layer in filteredLayers"
            :key="layer.id"
            class="flame-layer"
            :class="`layer-${layer.id}`"
          >
          <button
            type="button"
            class="flame-layer-header"
            :aria-expanded="!collapsedLayers.has(layer.id)"
            @click="toggleLayer(layer.id)"
          >
            <span class="flame-layer-chevron">
              <ChevronRight v-if="collapsedLayers.has(layer.id)" :size="16" aria-hidden="true" />
              <ChevronDown v-else :size="16" aria-hidden="true" />
            </span>
            <span class="flame-layer-dot"></span>
            <span class="flame-layer-copy">
              <strong>{{ layer.label }}</strong>
              <small>{{ layer.description }}</small>
            </span>
            <span class="flame-layer-stats">
              <strong>{{ layer.activityCount }}</strong> activities
              <small>{{ formatOffset(layer.durationMs) }} covered</small>
            </span>
          </button>

          <div v-if="!collapsedLayers.has(layer.id)" class="flame-tracks">
            <template v-if="layer.agentScopes?.length">
              <section
                v-for="scope in layer.agentScopes"
                :key="scope.id"
                class="flame-agent-scope"
                :class="[`scope-${scope.role}`, { 'is-collapsed': collapsedAgentScopes.has(scope.id) }]"
                :style="{ '--agent-scope-depth': scope.depth }"
              >
                <button
                  type="button"
                  class="flame-agent-scope-header"
                  :aria-expanded="!collapsedAgentScopes.has(scope.id)"
                  @click="toggleAgentScope(scope.id)"
                >
                  <span class="flame-agent-scope-gutter">
                    <ChevronRight
                      v-if="collapsedAgentScopes.has(scope.id)"
                      :size="14"
                      aria-hidden="true"
                    />
                    <ChevronDown v-else :size="14" aria-hidden="true" />
                    <Bot v-if="scope.role === 'main'" :size="15" aria-hidden="true" />
                    <CornerDownRight v-else :size="15" aria-hidden="true" />
                    <strong>
                      {{ scope.role === 'main' ? 'MAIN' : `SUB ${scope.ordinal}` }}
                    </strong>
                  </span>
                  <span class="flame-agent-scope-summary">
                    <span class="flame-agent-scope-copy">
                      <strong>{{ scope.label }}</strong>
                      <small v-if="scope.role === 'main'">Primary conversation owner</small>
                      <small v-else>
                        Spawned by {{ scope.parentLabel }} · {{ scope.spawnLabel }}
                      </small>
                    </span>
                    <span class="flame-agent-scope-metrics">
                      <span>{{ scope.llmCallCount }} LLM</span>
                      <span>{{ scope.toolCallCount }} tools</span>
                      <span v-if="scope.childCount">{{ scope.childCount }} children</span>
                      <strong>{{ formatOffset(scope.durationMs) }}</strong>
                    </span>
                  </span>
                </button>

                <div v-if="!collapsedAgentScopes.has(scope.id)" class="flame-agent-scope-tracks">
                  <div
                    v-for="track in scope.tracks"
                    :key="track.id"
                    class="flame-track-row"
                    :style="{ '--track-height': `${10 + (track.depthCount ?? 1) * 28}px` }"
                  >
                    <div class="fg-gutter">
                      <span :title="track.description">{{ track.label }}</span>
                      <small v-if="!track.synthetic && track.activities.length" class="fg-lane-count">
                        ×{{ track.activityCount ?? track.activities.length }}
                      </small>
                    </div>
                    <div class="flame-track fg-time-track" :class="{ synthetic: track.synthetic }">
                      <FlameTrackCanvas
                        :track="track"
                        :viewport="canvasViewport"
                        :selected-id="selectedId"
                        @select="selectActivity"
                        @focus="focusActivity"
                      />
                    </div>
                  </div>
                </div>
              </section>
            </template>
            <template v-else>
              <div
                v-for="track in layer.tracks"
                :key="track.id"
                class="flame-track-row"
                :style="{ '--track-height': `${10 + (track.depthCount ?? 1) * 28}px` }"
              >
                <div class="fg-gutter">
                  <span :title="track.description">{{ track.label }}</span>
                  <small v-if="!track.synthetic && track.activities.length" class="fg-lane-count">
                    ×{{ track.activityCount ?? track.activities.length }}
                  </small>
                </div>
                <div class="flame-track fg-time-track" :class="{ synthetic: track.synthetic }">
                  <FlameTrackCanvas
                    :track="track"
                    :viewport="canvasViewport"
                    :selected-id="selectedId"
                    @select="selectActivity"
                    @focus="focusActivity"
                  />
                </div>
              </div>
            </template>
            <div v-if="!layer.tracks.length" class="flame-layer-empty">
              No matching {{ layer.label.toLowerCase() }} activities
            </div>
          </div>
          </section>
        </div>
      </div>

      <div v-else-if="modelBuilding" class="flame-empty">
        <Layers3 :size="24" aria-hidden="true" />
        <strong>Building execution profile…</strong>
        <span>Projecting the trace without blocking timeline controls.</span>
      </div>

      <div v-else class="flame-empty">
        <Layers3 :size="24" aria-hidden="true" />
        <strong>No semantic activities to chart</strong>
        <span>Run an observed agent request, then refresh this trace.</span>
      </div>
    </section>

    <DetailPanel
      :detail="selectedDetail"
      :trace-id="traceKey"
      :error="detailError"
      hide-when-empty
      @clear="clearSelection"
    />
  </section>
</template>

<script setup>
import {
  computed,
  markRaw,
  onBeforeUnmount,
  ref,
  shallowRef,
  watch,
} from 'vue';
import {
  Bot,
  ChevronDown,
  ChevronRight,
  CornerDownRight,
  Layers3,
  Minus,
  Plus,
  RotateCcw,
} from '@lucide/vue';

import { readActionDetail, readActionLlmRequestContent } from '../../../api.js';
import { createRequestContextLoader } from './request-context.js';
import DetailPanel from '../../../components/DetailPanel.vue';
import TimelineOverview from '../../../components/timeline/TimelineOverview.vue';
import { constrainTimeViewport } from '../time-navigation/model.js';
import { useTimelineNavigation } from '../time-navigation/useTimelineNavigation.js';
import { formatOffset } from '../waterfall/model';
import FlameTrackCanvas from './FlameTrackCanvas.vue';
import {
  flameAnimationDriver,
  scheduleFlameRedraw,
} from './frame-scheduler.js';
import {
  buildFlameGraph,
  emptyFlameGraphModel,
  filterFlameGraph,
  flameActivityDetail,
  flameOverviewLanes,
} from './model';

const props = defineProps({
  traceKey: {
    type: [String, Number],
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

defineEmits(['load-full-waterfall']);

const model = shallowRef(emptyFlameGraphModel());
const modelBuilding = ref(false);
const unavailableRequestCount = ref(0);
const loadRequestContexts = createRequestContextLoader(readActionLlmRequestContent);
const viewport = ref(null);
const canvasViewport = markRaw({ startMs: 0, spanMs: 1 });
const collapsedLayers = ref(new Set());
const collapsedAgentScopes = ref(new Set());
const selectedId = ref(null);
const selectedDetail = ref(null);
const detailError = ref('');
const navigationSurface = ref(null);
const axisTrack = ref(null);
let activeDetailLoad = null;
let modelBuildToken = 0;
let modelIdleHandle = null;

const bounds = computed(() => ({ startMs: 0, spanMs: model.value.window.spanMs }));
const activeViewport = computed(() => viewport.value ?? bounds.value);
const filteredLayers = computed(() => filterFlameGraph(model.value, props.query));
const overviewLanes = computed(() => flameOverviewLanes(model.value));
const viewportLabel = computed(() => {
  const current = activeViewport.value;
  return current.spanMs >= bounds.value.spanMs - 0.001
    ? 'full trace'
    : compactTimeRange(current.startMs, current.startMs + current.spanMs);
});
const detailPanelVisible = computed(() => Boolean(selectedDetail.value || detailError.value));
const ticks = computed(() => Array.from({ length: 7 }, (_, index) => {
  const fraction = index / 6;
  return {
    pct: fraction * 100,
    label: formatOffset(activeViewport.value.startMs + activeViewport.value.spanMs * fraction),
  };
}));

watch(
  () => [
    props.waterfall?.actions,
    props.waterfall?.links,
    props.waterfall?.associations,
    props.traceKey,
  ],
  ([actions, links, associations, traceId]) => {
    scheduleModelBuild(actions, links, associations, traceId);
    collapsedAgentScopes.value = new Set();
    resetTimeViewport();
    clearSelection();
  },
  { immediate: true },
);

const {
  panning,
  zoomBy,
  resetViewport,
  handleWheel,
  startPan,
} = useTimelineNavigation({
  viewport,
  activeViewport,
  bounds,
  surfaceRef: navigationSurface,
  trackRef: axisTrack,
  trackSelector: '.fg-time-track',
  reset: resetTimeViewport,
  setViewport: setTimeViewport,
  animationDriver: flameAnimationDriver,
});

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

function scheduleModelBuild(actions, links, associations, traceId) {
  modelBuildToken += 1;
  const token = modelBuildToken;
  unavailableRequestCount.value = 0;
  model.value = emptyFlameGraphModel();
  if (modelIdleHandle !== null) {
    if (typeof cancelIdleCallback === 'function') {
      cancelIdleCallback(modelIdleHandle);
    } else {
      clearTimeout(modelIdleHandle);
    }
    modelIdleHandle = null;
  }
  if (!actions?.length && !links?.length) {
    model.value = emptyFlameGraphModel();
    resetTimeViewport();
    modelBuilding.value = false;
    return;
  }
  modelBuilding.value = true;
  const build = async () => {
    modelIdleHandle = null;
    const isCurrent = () => token === modelBuildToken;
    if (!isCurrent()) return;
    const { contexts, unavailable } = await loadRequestContexts(traceId, actions ?? [], isCurrent);
    if (!isCurrent()) return;
    unavailableRequestCount.value = unavailable;
    // Flame-graph ownership must not depend on backend user-turn accounting.
    model.value = buildFlameGraph(actions, links, null, associations, contexts);
    resetTimeViewport();
    modelBuilding.value = false;
  };
  if (typeof requestIdleCallback === 'function') {
    modelIdleHandle = requestIdleCallback(build, { timeout: 120 });
  } else {
    modelIdleHandle = setTimeout(build, 0);
  }
}

function compactTimeRange(startMs, endMs) {
  const start = formatOffset(startMs);
  const end = formatOffset(endMs);
  const unit = end.match(/[a-zµ]+$/i)?.[0];
  return unit && start.endsWith(unit)
    ? `${start.slice(0, -unit.length)}–${end}`
    : `${start}–${end}`;
}

function toggleLayer(layerId) {
  const next = new Set(collapsedLayers.value);
  if (next.has(layerId)) {
    next.delete(layerId);
  } else {
    next.add(layerId);
  }
  collapsedLayers.value = next;
}

function toggleAgentScope(scopeId) {
  const next = new Set(collapsedAgentScopes.value);
  if (next.has(scopeId)) {
    next.delete(scopeId);
  } else {
    next.add(scopeId);
  }
  collapsedAgentScopes.value = next;
}

async function selectActivity(activity) {
  if (activity.synthetic) {
    return;
  }
  const token = Symbol();
  activeDetailLoad = token;
  detailError.value = '';
  selectedId.value = activity.id;
  selectedDetail.value = flameActivityDetail(activity);
  if (activity.density) {
    return;
  }
  try {
    const action = await readActionDetail(props.traceKey, activity.id);
    if (activeDetailLoad === token && selectedId.value === activity.id) {
      selectedDetail.value = flameActivityDetail({ ...activity, action });
    }
  } catch (error) {
    if (activeDetailLoad === token && selectedId.value === activity.id) {
      detailError.value = String(error?.message ?? error);
    }
  }
}

function clearSelection() {
  activeDetailLoad = null;
  selectedId.value = null;
  selectedDetail.value = null;
  detailError.value = '';
}

function focusActivity(activity) {
  const duration = Math.max(activity.durMs ?? 0.001, bounds.value.spanMs * 0.002);
  const padding = Math.max(duration * 0.12, bounds.value.spanMs * 0.005);
  setTimeViewport({
    startMs: Math.max(activity.startOffsetMs - padding, 0),
    spanMs: Math.min(duration + padding * 2, bounds.value.spanMs),
  });
  selectActivity(activity);
}

function setOverviewViewport(nextViewport) {
  setTimeViewport(nextViewport);
}

function handleOverviewZoom({ factor, anchor }) {
  zoomBy(factor, anchor);
}

function setTimeViewport(nextViewport) {
  const next = constrainTimeViewport(nextViewport, bounds.value);
  viewport.value = next;
  syncCanvasViewport(next);
}

function resetTimeViewport() {
  viewport.value = null;
  syncCanvasViewport(bounds.value);
}

function syncCanvasViewport(nextViewport) {
  canvasViewport.startMs = Number(nextViewport?.startMs) || 0;
  canvasViewport.spanMs = Math.max(Number(nextViewport?.spanMs) || 1, 0.001);
  scheduleFlameRedraw();
}

</script>

<style src="./flame-graph.css" scoped></style>
