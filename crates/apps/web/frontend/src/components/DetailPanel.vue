<template>
  <aside class="detail-panel">
    <div class="detail-header">
      <div>
        <span>{{ detailKind }}</span>
        <h2>{{ detailTitle }}</h2>
      </div>
      <button class="icon-button subtle-button" type="button" title="Clear" @click="$emit('clear')">
        <X :size="18" aria-hidden="true" />
      </button>
    </div>

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

    <section v-if="Object.keys(detailAttributes).length" class="detail-section">
      <h3>Attributes</h3>
      <JsonTree :value="detailAttributes" />
    </section>

    <section v-if="payloadText" class="detail-section">
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
      <JsonTree :value="detailRawValue" />
    </section>
  </aside>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ChevronDown, X } from '@lucide/vue';

import { readActionFilePathSet, readPayload } from '../api';
import JsonTree from './JsonTree.vue';

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
});

defineEmits(['clear']);

const payloadText = ref('');
const payloadError = ref('');
const filePathSetMeta = ref(null);
const filePathSetPaths = ref([]);
const filePathSetError = ref('');
const filePathSetLoading = ref(false);
const filePathSetNextOffset = ref(0);
const filePathSetHasMore = ref(false);
let activePayloadLoad = null;
let activeFilePathSetLoad = null;

const detailTitle = computed(() => props.detail?.title ?? 'No selection');
const detailKind = computed(() => props.detail?.kind ?? 'detail');
const detailRows = computed(() => Object.entries(props.detail?.rows ?? {}));
const detailAttributes = computed(() => props.detail?.attributes ?? {});
const detailRawValue = computed(() => props.detail?.raw ?? null);
const panelError = computed(() => props.error || payloadError.value || filePathSetError.value);
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
    if (nextDetail?.payloadId && traceId) {
      loadPayload(traceId, nextDetail.payloadId, activePayloadLoad);
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
  payloadError.value = '';
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

async function loadPayload(traceId, payloadId, token) {
  try {
    const payload = await readPayload(traceId, payloadId);
    if (activePayloadLoad === token) {
      payloadText.value = payload.text ?? '';
    }
  } catch (err) {
    if (activePayloadLoad === token) {
      payloadError.value = String(err.message ?? err);
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
</script>
