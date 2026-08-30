<template>
  <div class="workbench">
    <aside class="trace-rail">
      <div class="rail-title">Traces</div>
      <button
        v-for="trace in traces"
        :key="trace.id"
        class="trace-row"
        :class="{ active: traceIdMatches(selectedTraceId, trace.id) }"
        type="button"
        @click="selectTrace(trace.id)"
      >
        <span>{{ trace.name }}</span>
        <small>{{ trace.display_id }}</small>
      </button>
    </aside>

    <main class="workspace">
      <section class="metrics-strip">
        <div v-for="metric in metrics" :key="metric.key" class="metric" :class="`metric-${metric.key}`">
          <span class="metric-icon" aria-hidden="true">
            <component :is="metric.icon" :size="17" />
          </span>
          <span class="metric-label">{{ metric.label }}</span>
          <strong>{{ metric.value }}</strong>
        </div>
      </section>

      <NavigationStrip
        :model-value="activeGroupDefinition.id"
        :items="primaryNavigationItems"
        aria-label="Trace view groups"
        controls-id="trace-group-panel"
        id-prefix="trace-group"
        variant="primary"
        @update:model-value="selectTabGroup"
      />

      <section
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
          :aria-label="`${activeGroupDefinition.label} views`"
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
          <div v-if="showLoadingPanel" class="loading-panel">
            <span class="loading-spinner" aria-hidden="true"></span>
            <p>Loading and analyzing captured data...</p>
            <small>This may take a moment for larger traces.</small>
          </div>
          <component
            v-else
            :is="activeTabDefinition.component"
            v-bind="activeTabProps"
            @open-waterfall="openWaterfall"
            @load-full-waterfall="loadFullWaterfall"
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

import {
  readActionTree,
  readActionTreeRoot,
  readTraceAlerts,
  readCommands,
  readTraceDiagnostics,
  readTraceEvents,
  readTracePayloads,
  readTraceProcesses,
  readTraceSummary,
  readTraceTimeAttribution,
  readTraceTimeline,
  readWaterfall,
} from '../api';
import NavigationStrip from '../components/navigation/NavigationStrip.vue';
import { TAB_DEFINITIONS, TAB_GROUP_DEFINITIONS, TAB_IDS } from '../tabs/registry';

const props = defineProps({
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

const primaryNavigationItems = tabGroups.map(({ id, label }) => ({ id, label }));
const METRIC_ICONS = Object.freeze({
  events: markRaw(Activity),
  payloads: markRaw(Boxes),
  actions: markRaw(GitBranch),
  processes: markRaw(Cpu),
});
const activeTab = ref(TAB_IDS.overview);
const lastTabByGroup = ref(
  Object.fromEntries(tabGroups.map((group) => [group.id, group.defaultTabId])),
);
const selectedTraceId = ref(null);
const traceDetail = shallowRef(null);
const actionTree = ref(emptyActionTree());
const commands = shallowRef(emptyCommands());
const waterfall = shallowRef(emptyWaterfall());
const timeAttribution = shallowRef(null);
const waterfallFocus = shallowRef(null);
const error = ref('');
const loading = ref(false);
let activeTraceLoad = null;
let activeCommandsLoad = null;
let activeWaterfallLoad = null;
let activeAttributionLoad = null;
let activeTracePartLoad = null;

const selectedTrace = computed(() =>
  props.traces.find((trace) => traceIdMatches(trace.id, selectedTraceId.value)),
);
const activeTraceName = computed(
  () => traceDetail.value?.trace?.name ?? selectedTrace.value?.name ?? 'No trace selected',
);

const activeTabDefinition = computed(() => tabDefinitionsById.get(activeTab.value));
const activeGroupDefinition = computed(() => tabGroupsByTabId.get(activeTab.value));
const secondaryNavigationItems = computed(() =>
  activeGroupDefinition.value.tabIds.map((tabId) => tabDefinitionsById.get(tabId)),
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
  if (activeTab.value === TAB_IDS.waterfall) {
    tabProps.waterfall = waterfall.value;
    tabProps.focusInterval = waterfallFocus.value;
    tabProps.attribution = timeAttribution.value;
  }
  return tabProps;
});

const metrics = computed(() => {
  const counts = traceDetail.value?.counts ?? {};
  const semantic = actionTree.value?.summary ?? {};
  return [
    { key: 'events', label: 'Events', value: counts.events ?? 0, icon: METRIC_ICONS.events },
    {
      key: 'payloads',
      label: 'Payloads',
      value: counts.payloads ?? traceDetail.value?.payloads?.length ?? 0,
      icon: METRIC_ICONS.payloads,
    },
    {
      key: 'actions',
      label: 'Actions',
      value: semantic.actions ?? actionTree.value?.actions?.length ?? 0,
      icon: METRIC_ICONS.actions,
    },
    {
      key: 'processes',
      label: 'Processes',
      value: traceDetail.value?.processes?.length ?? counts.process ?? 0,
      icon: METRIC_ICONS.processes,
    },
  ];
});

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
  if (activeTab.value === TAB_IDS.waterfall) {
    return waterfall.value?.loadedTraceId !== selectedTraceId.value;
  }
  return !traceDetail.value;
});

watch(selectedTraceId, async (traceId) => {
  if (!traceId) {
    traceDetail.value = null;
    actionTree.value = emptyActionTree();
    commands.value = emptyCommands();
    waterfall.value = emptyWaterfall();
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
  emit('loading', false);
});

function reconcileSelectedTrace() {
  if (!props.traces.length) {
    selectedTraceId.value = null;
    return;
  }
  if (!selectedTrace.value) {
    selectedTraceId.value = props.traces[0].id;
  }
}

function selectTrace(traceId) {
  selectedTraceId.value = traceId;
}

function selectTabGroup(groupId) {
  const group = tabGroupsById.get(groupId);
  if (!group) {
    error.value = `Unknown Trace view group: ${groupId}`;
    return;
  }
  const rememberedTabId = lastTabByGroup.value[groupId];
  activeTab.value = group.tabIds.includes(rememberedTabId)
    ? rememberedTabId
    : group.defaultTabId;
}

function selectLeafTab(tabId) {
  if (!activeGroupDefinition.value.tabIds.includes(tabId)) {
    error.value = `Trace view ${tabId} does not belong to ${activeGroupDefinition.value.label}`;
    return;
  }
  activeTab.value = tabId;
}

async function loadTrace(traceId) {
  const token = Symbol();
  activeTraceLoad = token;
  traceDetail.value = null;
  actionTree.value = emptyActionTree();
  commands.value = emptyCommands();
  waterfall.value = emptyWaterfall();
  timeAttribution.value = null;
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

  const summaryLoad = readTraceSummary(traceId)
    .then((summaryData) => {
      if (isCurrent()) {
        traceDetail.value = summaryData;
      }
    })
    .catch(fail);

  const treeLoad = readActionTreeRoot(traceId)
    .then((rootData) => {
      if (isCurrent()) {
        actionTree.value = emptyActionTree(rootData.summary, rootData);
      }
    })
    .catch(fail);

  try {
    await Promise.all([summaryLoad, treeLoad]);
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
    ensureCommandsForActiveTab(),
    ensureWaterfallForActiveTab(),
    ensureTimeAttributionForActiveTab(),
  ]);
}

async function ensureTracePartForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId) {
    return;
  }
  if (activeTab.value === TAB_IDS.timeline) {
    await ensureTracePart(traceId, 'timeline', readTraceTimeline);
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

async function ensureWaterfallForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId || activeTab.value !== TAB_IDS.waterfall) {
    return;
  }
  if (waterfall.value?.loadedTraceId === traceId) {
    return;
  }
  const token = Symbol();
  activeWaterfallLoad = token;
  try {
    const data = await readWaterfall(traceId);
    if (activeWaterfallLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      waterfall.value = withWaterfallTrace(data, traceId);
    }
  } catch (err) {
    if (activeWaterfallLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      error.value = String(err.message ?? err);
    }
  }
}

async function loadFullWaterfall() {
  const traceId = selectedTraceId.value;
  if (!traceId || !waterfall.value?.partial) {
    return;
  }
  const token = Symbol();
  activeWaterfallLoad = token;
  try {
    const data = await readActionTree(traceId);
    if (activeWaterfallLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      waterfall.value = withWaterfallTrace(
        {
          ...data,
          selected_actions: data.actions?.length ?? 0,
          total_actions: data.actions?.length ?? 0,
          partial: false,
        },
        traceId,
      );
    }
  } catch (err) {
    if (activeWaterfallLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      error.value = String(err.message ?? err);
    }
  }
}

async function ensureTimeAttributionForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId || activeTab.value !== TAB_IDS.waterfall) {
    return;
  }
  if (timeAttribution.value?.trace && traceIdMatches(timeAttribution.value.trace.id, traceId)) {
    return;
  }
  const token = Symbol();
  activeAttributionLoad = token;
  try {
    const data = await readTraceTimeAttribution(traceId);
    if (activeAttributionLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      timeAttribution.value = data;
    }
  } catch (err) {
    if (activeAttributionLoad === token && traceIdMatches(selectedTraceId.value, traceId)) {
      error.value = String(err.message ?? err);
    }
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
  await ensureWaterfallForActiveTab();
}

function emptyActionTree(summary = null, rootData = null) {
  return {
    actions: [],
    links: [],
    roots: [],
    summary,
    rootData,
    loadedTraceId: null,
  };
}

function emptyCommands() {
  return {
    actions: [],
    links: [],
    loadedTraceId: null,
  };
}

function emptyWaterfall() {
  return {
    actions: [],
    links: [],
    roots: [],
    selectedActions: 0,
    totalActions: 0,
    partial: false,
    loadedTraceId: null,
  };
}

function withWaterfallTrace(data, traceId) {
  return {
    actions: freezeTraceList(data.actions),
    links: freezeTraceList(data.links),
    roots: data.roots ?? [],
    selectedActions: data.selected_actions ?? data.actions?.length ?? 0,
    totalActions:
      data.total_actions ?? actionTree.value?.summary?.actions ?? data.actions?.length ?? 0,
    partial: Boolean(data.partial),
    loadedTraceId: traceId,
  };
}

function withCommandTrace(commandData, traceId) {
  return {
    actions: freezeTraceList(commandData.actions),
    links: freezeTraceList(commandData.links),
    loadedTraceId: traceId,
  };
}

function freezeTraceList(items) {
  return (items ?? []).map((item) => markRaw(item));
}

function freezeTracePayload(data) {
  if (!data || typeof data !== 'object') {
    return data;
  }
  const next = { ...data };
  for (const key of Object.keys(next)) {
    if (Array.isArray(next[key])) {
      next[key] = freezeTraceList(next[key]);
    }
  }
  return next;
}

function eventBackedTab(tab) {
  return (
    tab === TAB_IDS.events ||
    tab === TAB_IDS.files ||
    tab === TAB_IDS.network ||
    tab === TAB_IDS.resources
  );
}

function traceIdMatches(left, right) {
  return String(left ?? '') === String(right ?? '');
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

.trace-group-panel-single {
  grid-template-rows: minmax(0, 1fr);
}
</style>
