<template>
  <aside v-if="shouldRender" class="detail-panel">
    <div class="detail-header">
      <div>
        <span>{{ detailKind }}</span>
        <h2>{{ detailTitle }}</h2>
      </div>
      <button class="icon-button subtle-button" type="button" title="Clear" @click="$emit('clear')">
        <X :size="18" aria-hidden="true" />
      </button>
    </div>

    <ProgressiveDisclosure
      v-model:expanded="showAllDetails"
      :enabled="progressiveDetails"
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

        <LlmInsightPanel
          :detail="detail"
          :request-content="llmRequestContent"
          :request-loading="llmRequestLoading"
          :request-error="llmRequestError"
          :request-content-available="llmRequestContentAvailable"
          @load-request-content="loadLlmRequestContent"
        />
        <HttpInsightPanel :detail="detail" />
        <CommandInsightPanel :detail="detail" />
        <McpInsightPanel
          :detail="detail"
          :payload="mcpPayload"
          :payload-loading="payloadLoading"
          :payload-error="mcpPayloadError"
        />
      </template>

      <template #secondary>
        <LlmRequestCanonicalBody
          :trace-id="traceId"
          :action-id="llmRequestActionId(detail)"
          :metadata="llmRequestContentMetadata"
        />

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
          <h3>Path Set</h3>
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
  readActionLlmRequestContent,
  readActionMcpJsonRpcContent,
  readPayload,
} from '../api';
import CommandInsightPanel from './CommandInsightPanel.vue';
import HttpInsightPanel from './HttpInsightPanel.vue';
import JsonTree from './JsonTree.vue';
import LlmInsightPanel from './LlmInsightPanel.vue';
import McpInsightPanel from './McpInsightPanel.vue';
import ProgressiveDisclosure from './ProgressiveDisclosure.vue';
import LlmRequestCanonicalBody from './llm/LlmRequestCanonicalBody.vue';
import { deriveMcpJsonRpcView, mcpJsonRpcContentSource } from '../mcp/jsonRpcContent';
import { mcpPayloadEvidenceIds } from '../mcp/payloadEvidence';

const LLM_REQUEST_DETAIL_MAX_BYTES = 128 * 1024;
const EMPTY_JSON_EXPANDED_PATHS = new Set();

const props = defineProps({
  detail: {
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
const llmRequestContent = ref(null);
const llmRequestError = ref('');
const llmRequestLoading = ref(false);
const showAllDetails = ref(false);
const jsonExpansionByKey = ref(new Map());
let activePayloadLoad = null;
let activeFilePathSetLoad = null;
let activeLlmRequestLoad = null;

const detailTitle = computed(() => props.detail?.title ?? 'No selection');
const detailKind = computed(() => props.detail?.kind ?? 'detail');
const detailRows = computed(() => Object.entries(props.detail?.rows ?? {}));
const detailAttributes = computed(() => props.detail?.attributes ?? {});
const detailRawValue = computed(() => props.detail?.raw ?? null);
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
const llmRequestContentMetadata = computed(() => {
  const action = props.detail?.raw;
  if (action?.kind !== 'llm.request') {
    return null;
  }
  const attributes = action.attributes ?? {};
  return {
    state: attributes['llm.request.content_state'],
    bytes: attributes['llm.request.canonical_body_bytes'],
    blocks: attributes['llm.request.block_count'],
    hash: attributes['llm.request.canonical_body_hash'],
  };
});
const llmRequestContentAvailable = computed(() => {
  const metadata = llmRequestContentMetadata.value;
  const bytes = Number(metadata?.bytes);
  return metadata?.state === 'canonical_blocks'
    && Number.isFinite(bytes)
    && bytes <= LLM_REQUEST_DETAIL_MAX_BYTES;
});
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
    resetPayloadLoad();
    resetFilePathSetLoad();
    resetLlmRequestLoad();
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
      payloadError.value = 'MCP payload has no canonical content link or retained raw evidence';
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

function resetLlmRequestLoad() {
  activeLlmRequestLoad = Symbol();
  llmRequestContent.value = null;
  llmRequestError.value = '';
  llmRequestLoading.value = false;
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

async function loadLlmRequestContent() {
  const actionId = llmRequestActionId(props.detail);
  const token = activeLlmRequestLoad;
  if (!props.traceId || !actionId || !llmRequestContentAvailable.value || llmRequestLoading.value) {
    return;
  }
  try {
    llmRequestLoading.value = true;
    llmRequestError.value = '';
    const response = await readActionLlmRequestContent(props.traceId, actionId, {
      maxBytes: LLM_REQUEST_DETAIL_MAX_BYTES,
    });
    if (activeLlmRequestLoad === token) {
      llmRequestContent.value = response.content ?? null;
    }
  } catch (err) {
    if (activeLlmRequestLoad === token) {
      llmRequestError.value = String(err.message ?? err);
    }
  } finally {
    if (activeLlmRequestLoad === token) {
      llmRequestLoading.value = false;
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

function llmRequestActionId(detail) {
  const action = detail?.raw;
  if (action?.kind === 'llm.request') {
    return action.id;
  }
  return null;
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
