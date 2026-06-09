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

        <component
          :is="activeTabDefinition.component"
          v-bind="activeTabProps"
          @select-detail="handleDetailSelect"
        />
      </main>

      <aside class="detail-panel">
        <div class="detail-header">
          <div>
            <span>{{ detailKind }}</span>
            <h2>{{ detailTitle }}</h2>
          </div>
          <button class="icon-button subtle-button" type="button" title="Clear" @click="clearDetail">
            <X :size="18" aria-hidden="true" />
          </button>
        </div>

        <dl v-if="detailRows.length" class="detail-rows">
          <template v-for="[key, value] in detailRows" :key="key">
            <dt>{{ key }}</dt>
            <dd>{{ value }}</dd>
          </template>
        </dl>

        <section v-if="Object.keys(detailAttributes).length" class="detail-section">
          <h3>Attributes</h3>
          <JsonTree :value="detailAttributes" />
        </section>

        <section v-if="payloadText" class="detail-section">
          <h3>Payload</h3>
          <pre>{{ payloadText }}</pre>
        </section>

        <section v-if="detailRawValue" class="detail-section">
          <h3>JSON</h3>
          <JsonTree :value="detailRawValue" />
        </section>
      </aside>
    </div>

    <div v-if="error" class="error-bar">{{ error }}</div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref, watch } from 'vue';
import { RefreshCw, Search, X } from '@lucide/vue';

import { listTraces, readActionTree, readActionTreeRoot, readPayload, readTrace } from './api';
import JsonTree from './components/JsonTree.vue';
import TraceTabs from './tabs/TraceTabs.vue';
import { TAB_DEFINITIONS, TAB_IDS } from './tabs/registry';

const tabs = TAB_DEFINITIONS;
const activeTab = ref(TAB_IDS.overview);
const traces = ref([]);
const selectedTraceId = ref(null);
const traceDetail = ref(null);
const actionTree = ref(emptyActionTree());
const selectedDetailId = ref(null);
const selectedDetail = ref(null);
const query = ref('');
const error = ref('');
const payloadText = ref('');
let activeTraceLoad = null;
let activeActionTreeLoad = null;
let activePayloadLoad = null;

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
    traceDetail: traceDetail.value,
    actionTree: actionTree.value,
    query: query.value,
  };
  if (activeTab.value === TAB_IDS.actionTree) {
    props.traceKey = selectedTraceId.value ?? 'no-trace';
    props.selectedDetailId = selectedDetailId.value;
    props.selectedDetail = selectedDetail.value;
  }
  return props;
});
const detail = computed(() => selectedDetail.value);
const detailTitle = computed(() => detail.value?.title ?? 'No selection');
const detailKind = computed(() => detail.value?.kind ?? 'detail');
const detailRows = computed(() => Object.entries(detail.value?.rows ?? {}));
const detailAttributes = computed(() => detail.value?.attributes ?? {});
const detailRawValue = computed(() => detail.value?.raw ?? null);

const metrics = computed(() => {
  const counts = traceDetail.value?.counts ?? {};
  const semantic = actionTree.value?.summary ?? {};
  return [
    { label: 'Events', value: counts.events ?? 0 },
    { label: 'Payloads', value: traceDetail.value?.payloads?.length ?? 0 },
    { label: 'Actions', value: semantic.actions ?? actionTree.value?.actions?.length ?? 0 },
    { label: 'Processes', value: traceDetail.value?.processes?.length ?? 0 },
  ];
});

onMounted(refresh);

watch(selectedTraceId, async (traceId) => {
  if (!traceId) {
    return;
  }
  await loadTrace(traceId);
});

watch(activeTab, async () => {
  await ensureFullActionTreeForActiveTab();
});

watch(detail, async (nextDetail) => {
  const token = Symbol();
  activePayloadLoad = token;
  payloadText.value = '';
  if (!nextDetail?.payloadId || !selectedTraceId.value) {
    return;
  }
  const traceId = selectedTraceId.value;
  try {
    const payload = await readPayload(traceId, nextDetail.payloadId);
    if (activePayloadLoad === token && selectedTraceId.value === traceId && detail.value === nextDetail) {
      payloadText.value = payload.text ?? '';
    }
  } catch (err) {
    if (activePayloadLoad === token && selectedTraceId.value === traceId && detail.value === nextDetail) {
      error.value = String(err.message ?? err);
    }
  }
});

async function refresh() {
  try {
    error.value = '';
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
  clearDetail();
}

async function loadTrace(traceId) {
  const token = Symbol();
  activeTraceLoad = token;
  clearDetail();
  traceDetail.value = null;
  actionTree.value = emptyActionTree();
  try {
    error.value = '';
    const [detailData, rootData] = await Promise.all([readTrace(traceId), readActionTreeRoot(traceId)]);
    if (activeTraceLoad === token && selectedTraceId.value === traceId) {
      traceDetail.value = detailData;
      actionTree.value = emptyActionTree(rootData.summary, rootData);
      await ensureFullActionTreeForActiveTab();
    }
  } catch (err) {
    if (activeTraceLoad === token && selectedTraceId.value === traceId) {
      error.value = String(err.message ?? err);
    }
  }
}

async function ensureFullActionTreeForActiveTab() {
  const traceId = selectedTraceId.value;
  if (!traceId || !tabNeedsFullActionTree(activeTab.value)) {
    return;
  }
  if (actionTree.value?.loadedTraceId === traceId) {
    return;
  }
  const token = Symbol();
  activeActionTreeLoad = token;
  try {
    const data = await readActionTree(traceId);
    if (activeActionTreeLoad === token && selectedTraceId.value === traceId) {
      actionTree.value = withSummary(data, traceId);
    }
  } catch (err) {
    if (activeActionTreeLoad === token && selectedTraceId.value === traceId) {
      error.value = String(err.message ?? err);
    }
  }
}

function tabNeedsFullActionTree(tabId) {
  return tabId === TAB_IDS.commands;
}

function handleDetailSelect(nextDetail) {
  selectedDetailId.value = nextDetail?.selectionId ?? null;
  selectedDetail.value = nextDetail;
}

function clearDetail() {
  selectedDetailId.value = null;
  selectedDetail.value = null;
  payloadText.value = '';
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

function withSummary(actionTreeData, traceId) {
  return {
    ...actionTreeData,
    rootData: actionTree.value?.rootData ?? null,
    summary: {
      actions: actionTreeData.actions?.length ?? 0,
      links: actionTreeData.links?.length ?? 0,
      roots: actionTreeData.roots?.length ?? 0,
    },
    loadedTraceId: traceId,
  };
}
</script>
