<!-- Coordinate trace selection, timeline navigation and on-demand workspace data. -->
<template>
  <div class="workbench">
    <aside class="trace-rail">
      <div class="rail-title">{{ t('trace.railTitle') }}</div>
      <TraceListControls :pager="tracePages" />
      <div class="trace-list">
        <button
          v-for="trace in orderedTraces"
          :key="trace.id"
          :data-trace-id="trace.id"
          class="trace-row"
          :class="{ active: traceIdMatches(selectedTraceId, trace.id) }"
          type="button"
          @click="selectTrace(trace.id)"
        >
          <span class="trace-row-name">{{ trace.name }}</span>
          <small class="trace-row-meta">
            <span>{{ trace.display_id }}</span>
            <span class="trace-state" :class="`trace-state-${String(trace.state ?? '').toLowerCase()}`">
              {{ trace.state }}
            </span>
          </small>
        </button>
        <p v-if="!traces.length" class="rail-empty">{{ t('trace.railEmpty') }}</p>
      </div>

      <div v-if="selectedTrace" class="rail-summary">
        <div class="rail-summary-meta">
          <span class="trace-state" :class="`trace-state-${String(selectedTrace.state ?? '').toLowerCase()}`">
            {{ selectedTrace.state }}
          </span>
          <span v-if="traceDetail?.trace?.health" class="trace-health">{{ traceDetail.trace.health }}</span>
          <span v-if="traceDetail?.trace?.profile" class="trace-profile" :title="traceDetail.trace.profile">
            {{ traceDetail.trace.profile }}
          </span>
        </div>
        <dl class="rail-summary-facts">
          <div v-if="traceDetail?.trace?.local_id != null"><dt>{{ t('trace.localId') }}</dt><dd>{{ traceDetail.trace.local_id }}</dd></div>
          <div><dt>{{ t('trace.rootPid') }}</dt><dd>{{ traceDetail?.trace?.root_pid ?? '—' }}</dd></div>
          <div><dt>{{ t('trace.started') }}</dt><dd>{{ formatClock(traceDetail?.trace?.started_at) }}</dd></div>
        </dl>
      </div>
    </aside>

    <main class="workspace">
      <section v-if="selectedTrace" class="metrics-strip">
        <div v-for="metric in metrics" :key="metric.key" class="metric" :class="`metric-${metric.key}`">
          <span class="metric-icon" aria-hidden="true">
            <component :is="metric.icon" :size="17" />
          </span>
          <span class="metric-label">{{ metric.label }}</span>
          <strong><AnimatedNumber v-if="metric.value != null" :value="metric.value" /><span v-else>—</span></strong>
        </div>
      </section>

      <section v-if="!selectedTrace" class="trace-empty">
        <strong>{{ t('trace.selectTitle') }}</strong>
        <p>{{ t('trace.selectHint') }}</p>
      </section>

      <NavigationStrip
        v-if="selectedTrace"
        :model-value="activeGroupDefinition.id"
        :items="primaryNavigationItems"
        :aria-label="t('trace.groupsAria')"
        controls-id="trace-group-panel"
        id-prefix="trace-group"
        variant="primary"
        @update:model-value="selectTabGroup"
      />

      <section
        v-if="selectedTrace"
        id="trace-group-panel"
        class="trace-group-panel"
        :class="{ 'trace-group-panel-single': secondaryNavigationItems.length === 1 }"
        role="tabpanel"
        :aria-labelledby="`trace-group-${activeGroupDefinition.id}-tab`"
      >
        <NavigationStrip
          v-if="secondaryNavigationItems.length > 1"
          :model-value="activeTab"
          :items="secondaryNavigationItems"
          :aria-label="t('trace.viewsAria', { group: groupLabel(activeGroupDefinition) })"
          controls-id="trace-view-panel"
          id-prefix="trace-view"
          variant="secondary"
          @update:model-value="selectLeafTab"
        />

        <section
          id="trace-view-panel"
          class="trace-view-panel"
          role="tabpanel"
          :aria-labelledby="activeViewLabelId"
        >
          <TimelineControls
            v-if="timelineTab"
            :filters="timeline.filters" :loading="timelineLoading"
            :error="timelineError"
            @apply="applyTimelineFilters" @retry="timeline.retry()"
          />
          <div v-if="showLoadingPanel" class="loading-panel">
            <span class="loading-spinner" aria-hidden="true"></span>
            <p>{{ t('trace.loading') }}</p>
            <small>{{ t('trace.loadingHint') }}</small>
          </div>
          <component
            v-else
            :is="activeTabDefinition.component"
            v-bind="activeTabProps"
            @open-waterfall="openWaterfall"
            @load-attribution="ensureTimeAttributionForActiveTab"
            @update:viewport="timeline.setViewport($event)"
          />
        </section>
      </section>
    </main>

    <div v-if="error" class="error-bar">{{ error }}</div>
  </div>
</template>

<script setup>
import { computed, markRaw, onBeforeUnmount, ref, shallowRef, watch } from 'vue';
import { Activity, Boxes, Cpu, GitBranch } from '@lucide/vue';
import AnimatedNumber from '../components/AnimatedNumber.vue';

import {
  readActionTreeRoot,
  readTraceAlerts,
  readLlmTrajectoryGraph,
  readCommands,
  readTraceDiagnostics,
  readTraceEvents,
  readTraceResources,
  readTracePayloads,
  readTraceProcesses,
  readTraceMetadata,
  readTraceCounts,
  readTimelinePage,
  readTraceTimeAttribution,
  readTraceTimeline,
} from '../api';
import { emptyActionTree, emptyTrajectoryGraph, emptyCommands, withCommandTrace, freezeTraceList, freezeTracePayload, eventBackedTab, traceIdMatches, compareTraceRecency } from './timeline/utils.js';
import TraceListControls from './traces/TraceListControls.vue';
import TimelineControls from './timeline/TimelineControls.vue';
import { TimelineWindow } from './timeline/window.js';
import NavigationStrip from '../components/navigation/NavigationStrip.vue';
import { useModuleLocale } from '../locale';
import { TAB_DEFINITIONS, TAB_GROUP_DEFINITIONS, TAB_IDS } from '../tabs/registry';
import strings from './locale';

const props = defineProps({
  tracePages: { type: Object, required: true },
  traces: {
    type: Array,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
  refreshNonce: {
    type: Number,
    default: 0,
  },
  pendingTraceSelection: {
    type: Object,
    default: null,
  },
});

const emit = defineEmits(['active-title', 'loading', 'selection-consumed']);

const tabs = TAB_DEFINITIONS;
const tabGroups = TAB_GROUP_DEFINITIONS;
const tabDefinitionsById = new Map(tabs.map((tab) => [tab.id, tab]));
const tabGroupsById = new Map(tabGroups.map((group) => [group.id, group]));
const tabGroupsByTabId = new Map();
for (const group of tabGroups) {
  for (const tabId of group.tabIds) {
    if (!tabDefinitionsById.has(tabId)) {
      throw new Error(`Trace navigation group ${group.id} references unknown view ${tabId}`);
    }
    if (tabGroupsByTabId.has(tabId)) {
      throw new Error(`Trace view ${tabId} belongs to more than one navigation group`);
    }
    tabGroupsByTabId.set(tabId, group);
  }
}
if (tabGroupsByTabId.size !== tabs.length) {
  throw new Error('Every Trace view must belong to exactly one navigation group');
}

const { t } = useModuleLocale(strings);

function groupLabel(group) {
  return t(group?.labelKey ?? '');
}

const primaryNavigationItems = tabGroups.map(({ id, labelKey }) => ({ id, label: t(labelKey) }));
const METRIC_ICONS = Object.freeze({
  events: markRaw(Activity),
  payloads: markRaw(Boxes),
  actions: markRaw(GitBranch),
  processes: markRaw(Cpu),
});
const activeTab = ref(tabGroups[0]?.defaultTabId ?? TAB_IDS.actionTree);
const lastTabByGroup = ref(
  Object.fromEntries(tabGroups.map((group) => [group.id, group.defaultTabId])),
);
const selectedTraceId = ref(null);
const traceDetail = shallowRef(null);
const traceCounts = shallowRef(null);
const actionTree = ref(emptyActionTree());
const trajectoryGraph = shallowRef(emptyTrajectoryGraph());
const commands = shallowRef(emptyCommands());
const timeline = new TimelineWindow(readTimelinePage);
const waterfall = timeline.data;
const timelineLoading = timeline.loading;
const timelineError = timeline.error;
const timelineTab = computed(() => [TAB_IDS.flameGraph, TAB_IDS.waterfall].includes(activeTab.value));
const timeAttribution = shallowRef(null);
const attributionLoading = ref(false);
const waterfallFocus = shallowRef(null);
const error = ref('');
const loading = ref(false);
let activeTraceLoad = null;
let countsController = null;
let activeTrajectoryLoad = null;
let activeCommandsLoad = null;
let activeTreeLoad = null;
let activeAttributionLoad = null;
let activeTracePartLoad = null;

const selectedTrace = computed(() => {
  if (selectedTraceId.value == null) return null;
  if (traceIdMatches(traceDetail.value?.trace?.id, selectedTraceId.value)) return traceDetail.value.trace;
  return props.traces.find((trace) => traceIdMatches(trace.id, selectedTraceId.value))
    ?? { id: selectedTraceId.value, name: String(selectedTraceId.value) };
});
// Newest traces sit at the top of the rail and are the default selection.
const orderedTraces = computed(() => [...props.traces].sort(compareTraceRecency));
const activeTraceName = computed(
  () => traceDetail.value?.trace?.name ?? selectedTrace.value?.name ?? t('app.titles.noTraceSelected'),
);

const activeTabDefinition = computed(() => tabDefinitionsById.get(activeTab.value));
const activeGroupDefinition = computed(() => tabGroupsByTabId.get(activeTab.value));
const secondaryNavigationItems = computed(() =>
  activeGroupDefinition.value.tabIds.map((tabId) => {
    const definition = tabDefinitionsById.get(tabId);
    return { ...definition, label: t(definition.labelKey) };
  }),
);
const activeViewLabelId = computed(() => {
  if (secondaryNavigationItems.value.length === 1) {
    return `trace-group-${activeGroupDefinition.value.id}-tab`;
  }
  return `trace-view-${activeTab.value}-tab`;
});
const activeTabProps = computed(() => {
  const tabProps = {
    traceKey: selectedTraceId.value,
    traceDetail: traceDetail.value,
    actionTree: actionTree.value,
    query: props.query,
  };
  if (activeTab.value === TAB_IDS.commands) {
    tabProps.commands = commands.value;
  }
  if (activeTab.value === TAB_IDS.llmTrajectory) {
    tabProps.trajectoryGraph = trajectoryGraph.value;
  }
  if (activeTab.value === TAB_IDS.waterfall || activeTab.value === TAB_IDS.flameGraph) {
    Object.assign(tabProps, timeline.viewProps);
  }
  if (activeTab.value === TAB_IDS.waterfall) {
    tabProps.idleRange = timeline.filters;
    tabProps.focusInterval = waterfallFocus.value;
    tabProps.attribution = timeAttribution.value;
    tabProps.attributionLoading = attributionLoading.value;
  }
  return tabProps;
});

const metrics = computed(() => {
  const counts = traceCounts.value ?? {};
  return [
    { key: 'events', label: t('trace.metricEvents'), value: counts.events ?? null, icon: METRIC_ICONS.events },
    { key: 'payloads', label: t('trace.metricPayloads'), value: counts.payloads ?? null, icon: METRIC_ICONS.payloads },
    { key: 'actions', label: t('trace.metricActions'), value: counts.actions ?? null, icon: METRIC_ICONS.actions },
    { key: 'processes', label: t('trace.metricProcesses'), value: counts.processes ?? null, icon: METRIC_ICONS.processes },
  ];
});

const clockFormatter = new Intl.DateTimeFormat(undefined, {
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
});

function formatClock(value) {
  const millis = Number(value ?? 0);
  return Number.isFinite(millis) && millis > 0 ? clockFormatter.format(new Date(millis)) : '—';
}

const showLoadingPanel = computed(() => {
  if (!loading.value) {
    return false;
  }
  if (activeTab.value === TAB_IDS.actionTree) {
    return !actionTree.value?.rootData;
  }
  if (activeTab.value === TAB_IDS.commands) {
    return commands.value?.loadedTraceId !== selectedTraceId.value;
  }
  if (timelineTab.value) return false;
  if (activeTab.value === TAB_IDS.llmTrajectory) {
    return trajectoryGraph.value?.loadedTraceId !== selectedTraceId.value;
  }
  return !traceDetail.value;
});

watch(selectedTraceId, async (traceId) => {
  if (!traceId) {
    traceDetail.value = null;
    actionTree.value = emptyActionTree();
    trajectoryGraph.value = emptyTrajectoryGraph();
    commands.value = emptyCommands();
    timeline.reset();
    timeAttribution.value = null;
    waterfallFocus.value = null;
    return;
  }
  await loadTrace(traceId);
});

watch(
  () => props.traces,
  () => {
    reconcileSelectedTrace();
  },
  { immediate: true },
);

watch(
  () => props.refreshNonce,
  async () => {
    if (selectedTraceId.value) {
      await loadTrace(selectedTraceId.value);
    }
  },
);

watch(activeTab, async (tabId) => {
  if (![TAB_IDS.flameGraph, TAB_IDS.waterfall].includes(tabId)) timeline.cancel();
  const group = tabGroupsByTabId.get(tabId);
  if (group) {
    lastTabByGroup.value = {
      ...lastTabByGroup.value,
      [group.id]: tabId,
    };
  }
  await ensureDataForActiveTab();
});

watch(
  () => props.pendingTraceSelection,
  async (target) => {
    if (!target?.traceId) {
      return;
    }
    if (target.tabId && tabs.some((tab) => tab.id === target.tabId)) {
      activeTab.value = target.tabId;
    }
    if (target.focus) {
      waterfallFocus.value = {
        ...target.focus,
        traceId: target.traceId,
        nonce: target.nonce ?? Date.now(),
      };
    }
    if (!traceIdMatches(selectedTraceId.value, target.traceId)) {
      selectedTraceId.value = target.traceId;
    }
    emit('selection-consumed');
  },
  { immediate: true },
);

watch(
  activeTraceName,
  (title) => {
    emit('active-title', title);
  },
  { immediate: true },
);

watch(
  loading,
  (value) => {
    emit('loading', value);
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  activeTraceLoad = null;
  countsController?.abort();
  activeTreeLoad = null;
  activeTrajectoryLoad = null;
  activeCommandsLoad = null;
  activeAttributionLoad = null;
  activeTracePartLoad = null;
  timeline.cancel();
  emit('loading', false);
});

function reconcileSelectedTrace() {
  if (selectedTraceId.value == null && orderedTraces.value.length) selectedTraceId.value = orderedTraces.value[0].id;
}

function selectTrace(traceId) {
  selectedTraceId.value = traceId;
}

function selectTabGroup(groupId) {
  const group = tabGroupsById.get(groupId);
  if (!group) {
    error.value = t('trace.unknownViewGroup', { group: groupId });
    return;
  }
  const rememberedTabId = lastTabByGroup.value[groupId];
  activeTab.value = group.tabIds.includes(rememberedTabId)
    ? rememberedTabId
    : group.defaultTabId;
}

function selectLeafTab(tabId) {
  if (!activeGroupDefinition.value.tabIds.includes(tabId)) {
    error.value = t('trace.viewOutsideGroup', {
      view: tabId,
      group: groupLabel(activeGroupDefinition.value),
    });
    return;
  }
  activeTab.value = tabId;
}

async function loadTrace(traceId) {
  const token = Symbol();
  activeTraceLoad = token;
  countsController?.abort();
  countsController = new AbortController();
  traceCounts.value = null;
  traceDetail.value = traceIdMatches(traceDetail.value?.trace?.id, traceId) ? { trace: traceDetail.value.trace } : null;
  actionTree.value = emptyActionTree();
  commands.value = emptyCommands();
  timeline.reset();
  timeAttribution.value = null;
  activeAttributionLoad = null;
  attributionLoading.value = false;
  trajectoryGraph.value = emptyTrajectoryGraph();
  if (!traceIdMatches(waterfallFocus.value?.traceId, traceId)) {
    waterfallFocus.value = null;
  }
  loading.value = true;
  error.value = '';

  const isCurrent = () => activeTraceLoad === token && traceIdMatches(selectedTraceId.value, traceId);
  const fail = (err) => {
    if (isCurrent()) {
      error.value = String(err.message ?? err);
    }
  };

  const summaryLoad = readTraceMetadata(traceId)
    .then((summaryData) => {
      if (isCurrent()) {
        traceDetail.value = summaryData;
      }
    })
    .catch(fail);

  readTraceCounts(traceId, { signal: countsController.signal })
    .then(({ counts }) => { if (isCurrent()) traceCounts.value = counts; })
    .catch(err => { if (err.name !== 'AbortError') fail(err); });

  try {
    await summaryLoad;
    if (isCurrent()) {
      await ensureDataForActiveTab();
    }
  } finally {
    if (isCurrent()) {
      loading.value = false;
    }
  }
}

async function ensureDataForActiveTab() {
  await Promise.all([
    ensureTracePartForActiveTab(),
    ensureTrajectoryForActiveTab(),
    ensureCommandsForActiveTab(),
    ensureWaterfallForActiveTab(),
    ensureTreeForActiveTab(),
  ]);
}

async function ensureTracePartForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId) {
    return;
  }
  if (activeTab.value === TAB_IDS.timeline) {
    await ensureTracePart(traceId, 'timeline', readTraceTimeline);
  } else if (activeTab.value === TAB_IDS.resources) {
    await ensureTracePart(traceId, 'resources', readTraceResources);
  } else if (eventBackedTab(activeTab.value)) {
    await ensureTracePart(traceId, 'events', readTraceEvents);
  } else if (activeTab.value === TAB_IDS.payloads) {
    await ensureTracePart(traceId, 'payloads', readTracePayloads);
  } else if (activeTab.value === TAB_IDS.processes || activeTab.value === TAB_IDS.processTree) {
    await ensureTracePart(traceId, 'processes', readTraceProcesses);
  } else if (activeTab.value === TAB_IDS.diagnostics) {
    await ensureTracePart(traceId, 'diagnostics', readTraceDiagnostics);
  } else if (activeTab.value === TAB_IDS.alerts) {
    await ensureTracePart(traceId, 'alerts', readTraceAlerts);
  }
}

async function ensureTracePart(traceId, key, loader) {
  if (traceDetail.value?.[key] !== undefined) {
    return;
  }
  const token = Symbol();
  activeTracePartLoad = token;
  try {
    const data = await loader(traceId);
    if (activeTracePartLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      traceDetail.value = {
        ...(traceDetail.value ?? {}),
        ...freezeTracePayload(data),
      };
    }
  } catch (err) {
    if (activeTracePartLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      error.value = String(err.message ?? err);
    }
  }
}


async function ensureTrajectoryForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId || activeTab.value !== TAB_IDS.llmTrajectory) {
    return;
  }
  if (trajectoryGraph.value?.loadedTraceId === traceId) {
    return;
  }
  const token = Symbol();
  activeTrajectoryLoad = token;
  try {
    const data = await readLlmTrajectoryGraph(traceId);
    if (activeTrajectoryLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      trajectoryGraph.value = {
        ...data,
        nodes: freezeTraceList(data.nodes),
        edges: freezeTraceList(data.edges),
        loadedTraceId: traceId,
      };
    }
  } catch (err) {
    if (activeTrajectoryLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      error.value = String(err.message ?? err);
    }
  }
}
async function ensureCommandsForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId || activeTab.value !== TAB_IDS.commands) {
    return;
  }
  if (commands.value?.loadedTraceId === traceId) {
    return;
  }
  const token = Symbol();
  activeCommandsLoad = token;
  try {
    const data = await readCommands(traceId);
    if (activeCommandsLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      commands.value = withCommandTrace(data, traceId);
    }
  } catch (err) {
    if (activeCommandsLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      error.value = String(err.message ?? err);
    }
  }
}

async function ensureTreeForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId || activeTab.value !== TAB_IDS.actionTree || actionTree.value.loadedTraceId === traceId) return;
  const token = Symbol();
  activeTreeLoad = token;
  try {
    const data = await readActionTreeRoot(traceId);
    if (activeTreeLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      actionTree.value = { ...emptyActionTree(data.summary, data), loadedTraceId: traceId };
    }
  } catch (err) {
    if (activeTreeLoad === token && traceIdMatches(selectedTraceId.value, traceId)) error.value = String(err.message ?? err);
  }
}

async function ensureWaterfallForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId || !timelineTab.value || waterfall.value.loadedTraceId === traceId || timelineLoading.value) return;
  await timeline.load(traceId, { metadata: traceDetail.value });
}

function applyTimelineFilters(filters) {
  if (selectedTraceId.value) timeline.load(selectedTraceId.value, { filters, metadata: traceDetail.value });
}

async function ensureTimeAttributionForActiveTab() {
  const traceId = selectedTraceId.value;
  if (
    !traceId
    || activeTab.value !== TAB_IDS.waterfall
  ) {
    return;
  }
  if (timeAttribution.value?.trace && traceIdMatches(timeAttribution.value.trace.id, traceId)) {
    return;
  }
  const token = Symbol();
  if (attributionLoading.value) return;
  activeAttributionLoad = token;
  attributionLoading.value = true;
  try {
    const data = await readTraceTimeAttribution(traceId);
    if (activeAttributionLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      timeAttribution.value = data;
    }
  } catch (err) {
    if (activeAttributionLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      error.value = String(err.message ?? err);
    }
  } finally {
    if (activeAttributionLoad === token) attributionLoading.value = false;
  }
}

async function openWaterfall(target) {
  if (!target?.startNanos || !target?.endNanos) {
    return;
  }
  waterfallFocus.value = {
    ...target,
    traceId: selectedTraceId.value,
    nonce: Date.now(),
  };
  activeTab.value = TAB_IDS.waterfall;
  await timeline.load(selectedTraceId.value, { filters: { ...timeline.filters, from: String(target.startNanos), to: String(target.endNanos) } });
}

</script>

<style scoped>
.trace-group-panel,
.trace-view-panel {
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: hidden;
}

.trace-group-panel {
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
}

.trace-view-panel { overflow: auto; }

.trace-group-panel-single {
  grid-template-rows: minmax(0, 1fr);
}
</style>
