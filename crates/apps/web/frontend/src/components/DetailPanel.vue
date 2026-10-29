<!-- Load selected action details and reveal request content on demand. -->
<template>
  <aside v-if="shouldRender" class="detail-panel">
    <div class="detail-header">
      <div>
        <span v-if="detailKind !== detailTitle">{{ detailKind }}</span>
        <h2>{{ detailTitle }}</h2>
      </div>
      <button class="icon-button subtle-button" type="button" title="Clear" @click="$emit('clear')">
        <X :size="18" aria-hidden="true" />
      </button>
    </div>

    <dl v-if="supplementalRowEntries.length" class="detail-rows">
      <template v-for="[key, value] in supplementalRowEntries" :key="key">
        <dt>{{ key }}</dt>
        <dd>{{ value }}</dd>
      </template>
    </dl>

    <template v-if="detail?.raw?.kind === 'llm.request'">
      <p v-if="panelError" class="detail-error">{{ panelError }}</p>
      <LlmInsightPanel :detail="detail" :trace-id="traceId" />
    </template>
    <template v-else-if="detail?.raw?.kind === 'llm.response'">
      <p v-if="panelError" class="detail-error">{{ panelError }}</p>
      <LlmResponseContent :key="detail.raw.id" :detail="detail" />
    </template>
    <template v-else-if="detail?.raw?.kind === 'http.message'">
      <p v-if="panelError" class="detail-error">{{ panelError }}</p>
      <HttpInsightPanel :detail="detail" :trace-id="traceId" />
    </template>
    <ProgressiveDisclosure
      v-else
      v-model:expanded="showAllDetails"
      :enabled="progressiveDetails || isLlmCall"
    >
      <template #primary>
        <dl v-if="detailRows.length" class="detail-rows">
          <template v-for="[key, value] in detailRows" :key="key">
            <dt>{{ key }}</dt>
            <dd>{{ value }}</dd>
          </template>
        </dl>

        <section v-if="panelError" class="detail-section">
          <h3>Error</h3>
          <p class="detail-error">{{ panelError }}</p>
        </section>

        <LlmCallPanel v-if="isLlmCall" :trace-id="traceId" :call="detail.raw" />
        <LlmInsightPanel
          v-else
          :detail="detail"
          :trace-id="traceId"
        />
        <HttpInsightPanel v-if="!isLlmCall" :detail="detail" />
        <CommandInsightPanel :detail="detail" />
        <McpInsightPanel
          :detail="detail"
          :payload="mcpPayload"
          :payload-loading="payloadLoading"
          :payload-error="mcpPayloadError"
        />
      </template>

      <template #secondary>
        <dl v-if="secondaryRows.length" class="detail-rows detail-secondary-rows">
          <template v-for="[key, value] in secondaryRows" :key="key">
            <dt>{{ key }}</dt>
            <dd>{{ value }}</dd>
          </template>
        </dl>
        <section v-if="Object.keys(detailAttributes).length" class="detail-section">
          <h3>Attributes</h3>
          <JsonTree
            :value="detailAttributes"
            :expanded-paths="jsonExpandedPaths('attributes')"
            @toggle-node="updateJsonExpansion('attributes', $event)"
          />
        </section>

        <section v-if="payloadText && !isMcpPayloadDetail" class="detail-section">
          <h3>Payload</h3>
          <pre>{{ payloadText }}</pre>
        </section>

        <section v-if="hasFilePathSet" class="detail-section">
          <h3>{{ t('detail.pathSet') }}</h3>
          <dl v-if="filePathSetRows.length" class="detail-rows path-set-rows">
            <template v-for="[key, value] in filePathSetRows" :key="key">
              <dt>{{ key }}</dt>
              <dd>{{ value }}</dd>
            </template>
          </dl>
          <ul v-if="filePathSetPaths.length" class="path-set-list">
            <li v-for="path in filePathSetPaths" :key="path.path_id">
              <code>{{ path.path }}</code>
            </li>
          </ul>
          <button
            v-if="filePathSetHasMore"
            class="detail-load-button"
            type="button"
            :disabled="filePathSetLoading"
            @click="loadMoreFilePathSet"
          >
            <ChevronDown :size="16" aria-hidden="true" />
            <span>{{ filePathSetLoading ? 'Loading' : 'More' }}</span>
          </button>
          <p v-else-if="filePathSetLoading" class="detail-muted">Loading</p>
        </section>

        <section v-if="detailRawValue" class="detail-section">
          <h3>JSON</h3>
          <JsonTree
            :value="detailRawValue"
            :expanded-paths="jsonExpandedPaths('raw')"
            @toggle-node="updateJsonExpansion('raw', $event)"
          />
        </section>
      </template>
    </ProgressiveDisclosure>
  </aside>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ChevronDown, X } from '@lucide/vue';

import {
  readActionFilePathSet,
  readActionMcpJsonRpcContent,
  readPayload,
} from '../api';
import CommandInsightPanel from './CommandInsightPanel.vue';
import HttpInsightPanel from './HttpInsightPanel.vue';
import JsonTree from './JsonTree.vue';
import LlmInsightPanel from './LlmInsightPanel.vue';
import LlmResponseContent from './llm/LlmResponseContent.vue';
import McpInsightPanel from './McpInsightPanel.vue';
import ProgressiveDisclosure from './ProgressiveDisclosure.vue';
import LlmCallPanel from './llm/LlmCallPanel.vue';
import { useModuleLocale } from '../locale';
import strings from './locale';
import { deriveMcpJsonRpcView, mcpJsonRpcContentSource } from '../mcp/jsonRpcContent';
import { mcpPayloadEvidenceIds } from '../mcp/payloadEvidence';
import { formatTime } from '../tabs/tableModel';

const { t } = useModuleLocale(strings);

const EMPTY_JSON_EXPANDED_PATHS = new Set();
const FOCUSED_ROW_KEYS = new Set([
  'target', 'status', 'completeness', 'started', 'ended', 'start_offset',
  'duration', 'display_duration', 'ttft', 'output_tokens', 'total_tokens',
  'model', 'exit_code', 'activities', 'interval',
]);

const props = defineProps({
  detail: {
    type: Object,
    default: null,
  },
  supplementalRows: {
    type: Object,
    default: null,
  },
  traceId: {
    type: [String, Number],
    default: null,
  },
  error: {
    type: String,
    default: '',
  },
  hideWhenEmpty: {
    type: Boolean,
    default: false,
  },
  progressiveDetails: {
    type: Boolean,
    default: false,
  },
  mcpContentMaxBytes: {
    type: Number,
    default: 4_194_304,
    validator: (value) => Number.isInteger(value) && value > 0,
  },
});

defineEmits(['clear']);

const payloadText = ref('');
const mcpPayloadView = ref(null);
const payloadError = ref('');
const payloadLoading = ref(false);
const filePathSetMeta = ref(null);
const filePathSetPaths = ref([]);
const filePathSetError = ref('');
const filePathSetLoading = ref(false);
const filePathSetNextOffset = ref(0);
const filePathSetHasMore = ref(false);
const showAllDetails = ref(false);
const jsonExpansionByKey = ref(new Map());
let activePayloadLoad = null;
let activeFilePathSetLoad = null;

const detailTitle = computed(() => props.detail?.title ?? t('detail.noSelection'));
const detailKind = computed(() => props.detail?.kind ?? 'detail');
const supplementalRowEntries = computed(() => Object.entries(props.supplementalRows ?? {}));
const isLlmCall = computed(() => props.detail?.raw?.kind === 'llm.call');
// Focus on the selected action; retain association metadata in the expanded view.
const detailRows = computed(() => Object.entries(props.detail?.rows ?? {}).filter(([key, value]) => (
  isLlmCall.value ? ['status', 'completeness', 'ttft'].includes(key)
    : !props.progressiveDetails || (FOCUSED_ROW_KEYS.has(key)
    && !(key === 'completeness' && value === 'complete'))
)).map(([key, value]) => [key,
  props.progressiveDetails && (key === 'started' || key === 'ended') ? formatTime(value) : value,
]));
const secondaryRows = computed(() => props.progressiveDetails
  ? Object.entries(props.detail?.rows ?? {}).filter(([key]) => (
    !FOCUSED_ROW_KEYS.has(key) && key !== 'classification' && key !== 'semantic_label'
  )) : []);
const detailAttributes = computed(() => props.detail?.attributes ?? {});
const detailRawValue = computed(() => props.detail?.raw ?? null);
watch(() => [props.traceId, props.detail?.selectionId], () => { showAllDetails.value = false; });
const MCP_PAYLOAD_DETAIL_KINDS = new Set([
  'mcp.request',
  'mcp.response',
  'mcp.stdin',
  'mcp.stdout',
]);
const isMcpPayloadDetail = computed(() => MCP_PAYLOAD_DETAIL_KINDS.has(props.detail?.raw?.kind));
const mcpPayload = computed(() => mcpPayloadView.value ?? payloadText.value);
const mcpPayloadError = computed(() => (isMcpPayloadDetail.value ? payloadError.value : ''));
const genericPayloadError = computed(() => (isMcpPayloadDetail.value ? '' : payloadError.value));
const panelError = computed(() => props.error || genericPayloadError.value || filePathSetError.value);
const shouldRender = computed(() => !props.hideWhenEmpty || Boolean(props.detail || panelError.value));
const hasFilePathSet = computed(
  () => Boolean(filePathSetMeta.value) || filePathSetPaths.value.length > 0 || filePathSetLoading.value,
);
const filePathSetRows = computed(() => {
  const meta = filePathSetMeta.value;
  if (!meta) {
    return [];
  }
  return Object.entries({
    state: meta.state,
    unique_path_count: meta.unique_path_count,
    stored_path_count: meta.stored_path_count,
    chunking_scheme: meta.chunking_scheme,
  });
});

watch(
  () => [props.detail, props.traceId],
  ([nextDetail, traceId]) => {
    showAllDetails.value = false;
    resetPayloadLoad();
    resetFilePathSetLoad();
    let semanticMcpContent = null;
    let payloadSourceValid = true;
    try {
      semanticMcpContent = mcpJsonRpcContentSource(nextDetail?.raw);
    } catch (err) {
      payloadSourceValid = false;
      payloadError.value = String(err.message ?? err);
    }
    const payloadIds = payloadSourceValid ? detailPayloadIds(nextDetail) : [];
    if (semanticMcpContent && traceId) {
      loadMcpJsonRpcContent({
        traceId,
        actionId: semanticMcpContent.actionId,
        kind: semanticMcpContent.viewKind,
        fallbackPayloadIds: payloadIds,
        token: activePayloadLoad,
      });
    } else if (payloadIds.length && traceId) {
      loadPayloads(traceId, payloadIds, activePayloadLoad);
    } else if (isMcpPayloadDetail.value && traceId && payloadSourceValid) {
      payloadError.value = t('detail.mcpPayloadMissing');
    }
    if (nextDetail?.filePathSetActionId && traceId) {
      loadFilePathSetPage({
        traceId,
        actionId: nextDetail.filePathSetActionId,
        pageSize: nextDetail.filePathSetPageSize,
        offset: 0,
        append: false,
        token: activeFilePathSetLoad,
      });
    }
  },
  { immediate: true },
);

function resetPayloadLoad() {
  activePayloadLoad = Symbol();
  payloadText.value = '';
  mcpPayloadView.value = null;
  payloadError.value = '';
  payloadLoading.value = false;
}

function resetFilePathSetLoad() {
  activeFilePathSetLoad = Symbol();
  filePathSetMeta.value = null;
  filePathSetPaths.value = [];
  filePathSetError.value = '';
  filePathSetLoading.value = false;
  filePathSetNextOffset.value = 0;
  filePathSetHasMore.value = false;
}

async function loadPayloads(traceId, payloadIds, token) {
  try {
    payloadLoading.value = true;
    const payloads = await Promise.all(
      payloadIds.map((payloadId) => readPayload(traceId, payloadId)),
    );
    if (activePayloadLoad === token) {
      payloadText.value = payloads.map((payload) => payload.text ?? '').join('');
    }
  } catch (err) {
    if (activePayloadLoad === token) {
      payloadError.value = String(err.message ?? err);
    }
  } finally {
    if (activePayloadLoad === token) {
      payloadLoading.value = false;
    }
  }
}

async function loadMcpJsonRpcContent({
  traceId,
  actionId,
  kind,
  fallbackPayloadIds,
  token,
}) {
  try {
    payloadLoading.value = true;
    const response = await readActionMcpJsonRpcContent(traceId, actionId, {
      maxBytes: props.mcpContentMaxBytes,
    });
    if (activePayloadLoad !== token) {
      return;
    }
    if (!response.content && fallbackPayloadIds.length) {
      payloadLoading.value = false;
      await loadPayloads(traceId, fallbackPayloadIds, token);
      return;
    }
    mcpPayloadView.value = deriveMcpJsonRpcView(kind, response.content ?? null);
  } catch (err) {
    if (activePayloadLoad === token) {
      payloadError.value = String(err.message ?? err);
    }
  } finally {
    if (activePayloadLoad === token) {
      payloadLoading.value = false;
    }
  }
}

async function loadMoreFilePathSet() {
  if (!props.detail?.filePathSetActionId || !props.traceId) {
    return;
  }
  await loadFilePathSetPage({
    traceId: props.traceId,
    actionId: props.detail.filePathSetActionId,
    pageSize: props.detail.filePathSetPageSize,
    offset: filePathSetNextOffset.value,
    append: true,
    token: activeFilePathSetLoad,
  });
}

async function loadFilePathSetPage({ traceId, actionId, pageSize, offset, append, token }) {
  if (!Number.isInteger(pageSize) || pageSize < 1) {
    filePathSetError.value = 'invalid file path set page size';
    return;
  }
  try {
    filePathSetLoading.value = true;
    const page = await readActionFilePathSet(traceId, actionId, {
      offset,
      limit: pageSize,
    });
    if (activeFilePathSetLoad !== token) {
      return;
    }
    filePathSetMeta.value = page.path_set ?? null;
    filePathSetPaths.value = append
      ? filePathSetPaths.value.concat(page.paths ?? [])
      : (page.paths ?? []);
    filePathSetNextOffset.value = page.next_offset ?? filePathSetPaths.value.length;
    filePathSetHasMore.value = Boolean(page.has_more);
  } catch (err) {
    if (activeFilePathSetLoad === token) {
      filePathSetError.value = String(err.message ?? err);
    }
  } finally {
    if (activeFilePathSetLoad === token) {
      filePathSetLoading.value = false;
    }
  }
}

function detailPayloadIds(detail) {
  if (detail?.payloadId !== null && detail?.payloadId !== undefined) {
    return [detail.payloadId];
  }
  return mcpPayloadEvidenceIds(detail?.raw);
}

function jsonExpandedPaths(section) {
  return jsonExpansionByKey.value.get(jsonExpansionKey(section)) ?? EMPTY_JSON_EXPANDED_PATHS;
}

function updateJsonExpansion(section, event) {
  if (!event?.path) {
    return;
  }
  const key = jsonExpansionKey(section);
  const nextMap = new Map(jsonExpansionByKey.value);
  const nextPaths = new Set(nextMap.get(key) ?? []);
  if (event.expanded) {
    nextPaths.add(event.path);
  } else {
    nextPaths.delete(event.path);
  }
  if (nextPaths.size) {
    nextMap.set(key, nextPaths);
  } else {
    nextMap.delete(key);
  }
  jsonExpansionByKey.value = nextMap;
}

function jsonExpansionKey(section) {
  const kind = props.detail?.raw?.kind ?? props.detail?.kind ?? 'detail';
  return `${kind}:${section}`;
}
</script>
