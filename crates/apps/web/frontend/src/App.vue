<template>
  <div class="app-shell">
    <header class="topbar">
      <div class="brand">
        <span class="brand-mark">A</span>
        <div>
          <h1>AcTrail</h1>
          <p>{{ activeTraceName }}</p>
        </div>
      </div>
      <div class="toolbar">
        <label class="search-box">
          <Search :size="18" aria-hidden="true" />
          <input v-model="query" type="search" placeholder="Filter" />
        </label>
        <button class="icon-button" type="button" title="Refresh" @click="refresh">
          <RefreshCw :size="18" aria-hidden="true" />
        </button>
      </div>
    </header>

    <div v-if="loading" class="load-progress" role="progressbar" aria-label="Loading trace data">
      <span class="load-progress-bar"></span>
    </div>

    <div class="workbench">
      <aside class="trace-rail">
        <div class="rail-title">Traces</div>
        <button
          v-for="trace in traces"
          :key="trace.id"
          class="trace-row"
          :class="{ active: selectedTraceId === trace.id }"
          type="button"
          @click="selectTrace(trace.id)"
        >
          <span>{{ trace.name }}</span>
          <small>{{ trace.display_id }}</small>
        </button>
      </aside>

      <main class="workspace">
        <section class="metrics-strip">
          <div v-for="metric in metrics" :key="metric.label" class="metric">
            <span>{{ metric.label }}</span>
            <strong>{{ metric.value }}</strong>
          </div>
        </section>

        <TraceTabs v-model="activeTab" :tabs="tabs" />

        <div v-if="showLoadingPanel" class="loading-panel">
          <span class="loading-spinner" aria-hidden="true"></span>
          <p>Loading and analyzing captured data…</p>
          <small>This may take a moment for larger traces.</small>
        </div>
        <component
          v-else
          :is="activeTabDefinition.component"
          v-bind="activeTabProps"
        />
      </main>
    </div>

    <div v-if="error" class="error-bar">{{ error }}</div>
  </div>
</template>

<script setup>
import { computed, markRaw, onMounted, shallowRef, ref, watch } from 'vue';
import { RefreshCw, Search } from '@lucide/vue';

import {
  clearServerCache,
  listTraces,
  readActionTree,
  readActionTreeRoot,
  readCommands,
  readTraceDiagnostics,
  readTraceEvents,
  readTracePayloads,
  readTraceProcesses,
  readTraceSummary,
  readTraceTimeline,
} from './api';
import TraceTabs from './tabs/TraceTabs.vue';
import { TAB_DEFINITIONS, TAB_IDS } from './tabs/registry';

const tabs = TAB_DEFINITIONS;
const activeTab = ref(TAB_IDS.overview);
const traces = ref([]);
const selectedTraceId = ref(null);
const traceDetail = shallowRef(null);
const actionTree = ref(emptyActionTree());
const commands = shallowRef(emptyCommands());
const waterfall = shallowRef(emptyWaterfall());
const query = ref('');
const error = ref('');
const loading = ref(false);
let activeTraceLoad = null;
let activeCommandsLoad = null;
let activeWaterfallLoad = null;
let activeTracePartLoad = null;

const selectedTrace = computed(() =>
  traces.value.find((trace) => trace.id === selectedTraceId.value),
);
const activeTraceName = computed(
  () => traceDetail.value?.trace?.name ?? selectedTrace.value?.name ?? 'No trace selected',
);

const activeTabDefinition = computed(
  () => tabs.find((tab) => tab.id === activeTab.value) ?? tabs[0],
);
const activeTabProps = computed(() => {
  const props = {
    traceKey: selectedTraceId.value,
    traceDetail: traceDetail.value,
    actionTree: actionTree.value,
    query: query.value,
  };
  if (activeTab.value === TAB_IDS.commands) {
    props.commands = commands.value;
  }
  if (activeTab.value === TAB_IDS.waterfall) {
    props.waterfall = waterfall.value;
  }
  return props;
});

const metrics = computed(() => {
  const counts = traceDetail.value?.counts ?? {};
  const semantic = actionTree.value?.summary ?? {};
  return [
    { label: 'Events', value: counts.events ?? 0 },
    { label: 'Payloads', value: counts.payloads ?? traceDetail.value?.payloads?.length ?? 0 },
    { label: 'Actions', value: semantic.actions ?? actionTree.value?.actions?.length ?? 0 },
    { label: 'Processes', value: traceDetail.value?.processes?.length ?? counts.process ?? 0 },
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

onMounted(refresh);

watch(selectedTraceId, async (traceId) => {
  if (!traceId) {
    return;
  }
  await loadTrace(traceId);
});

watch(activeTab, async () => {
  await ensureDataForActiveTab();
});

async function refresh() {
  try {
    error.value = '';
    await clearServerCache();
    const data = await listTraces();
    traces.value = data.traces ?? [];
    if (!selectedTraceId.value && traces.value.length) {
      selectedTraceId.value = traces.value[0].id;
    } else if (selectedTraceId.value) {
      await loadTrace(selectedTraceId.value);
    }
  } catch (err) {
    error.value = String(err.message ?? err);
  }
}

function selectTrace(traceId) {
  selectedTraceId.value = traceId;
}

async function loadTrace(traceId) {
  const token = Symbol();
  activeTraceLoad = token;
  traceDetail.value = null;
  actionTree.value = emptyActionTree();
  commands.value = emptyCommands();
  waterfall.value = emptyWaterfall();
  loading.value = true;
  error.value = '';

  const isCurrent = () => activeTraceLoad === token && selectedTraceId.value === traceId;
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
    if (activeTracePartLoad === token && selectedTraceId.value === traceId) {
      traceDetail.value = {
        ...(traceDetail.value ?? {}),
        ...freezeTracePayload(data),
      };
    }
  } catch (err) {
    if (activeTracePartLoad === token && selectedTraceId.value === traceId) {
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
    if (activeCommandsLoad === token && selectedTraceId.value === traceId) {
      commands.value = withCommandTrace(data, traceId);
    }
  } catch (err) {
    if (activeCommandsLoad === token && selectedTraceId.value === traceId) {
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
    const data = await readActionTree(traceId);
    if (activeWaterfallLoad === token && selectedTraceId.value === traceId) {
      waterfall.value = withWaterfallTrace(data, traceId);
    }
  } catch (err) {
    if (activeWaterfallLoad === token && selectedTraceId.value === traceId) {
      error.value = String(err.message ?? err);
    }
  }
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
    loadedTraceId: null,
  };
}

function withWaterfallTrace(data, traceId) {
  return {
    actions: freezeTraceList(data.actions),
    links: freezeTraceList(data.links),
    roots: data.roots ?? [],
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
</script>
