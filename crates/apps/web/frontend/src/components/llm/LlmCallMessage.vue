<template>
  <article class="llm-call-message" :aria-label="heading">
    <template v-if="!action">
      <h3>{{ heading }}</h3>
      <p v-if="loading" class="detail-muted" role="status">{{ t('llmCall.loading') }}</p>
      <p v-else-if="!actionId" class="detail-muted">{{ t('llmCall.unlinked') }}</p>
    </template>
    <p v-if="error" class="detail-error" role="alert">{{ error }}</p>
    <button v-if="error" type="button" class="detail-load-button" @click="loadAction">
      {{ t('llmCall.retry') }}
    </button>
    <template v-if="action">
      <InsightPanel :insight="insight" />
      <p v-if="!insight.blocks.length" class="detail-muted">{{ t('llmCall.noContent') }}</p>
      <template v-if="isRequest && contentAvailable">
        <button
          v-if="!content"
          type="button"
          class="detail-load-button"
          :disabled="contentLoading"
          @click="loadContent"
        >{{ contentLoading ? t('llmInsight.loading') : t('llmInsight.load') }}</button>
        <p v-if="contentError" class="detail-error" role="alert">{{ contentError }}</p>
        <details class="llm-call-raw" @toggle="canonicalOpen = $event.target.open">
          <summary>{{ t('canonicalBody.title') }}</summary>
          <LlmRequestBlocks
            v-if="canonicalOpen"
            :trace-id="traceId"
            :action-id="actionId"
          />
        </details>
      </template>
      <details class="llm-call-raw" @toggle="technicalOpen = $event.target.open">
        <summary>{{ t('llmCall.technicalDetails') }}</summary>
        <dl class="detail-rows">
          <dt>ID</dt><dd>{{ action.id }}</dd>
          <dt>{{ t('llmCall.started') }}</dt><dd>{{ action.start_time }}</dd>
          <dt>{{ t('llmCall.ended') }}</dt><dd>{{ action.end_time || '—' }}</dd>
        </dl>
        <JsonTree v-if="technicalOpen" :value="action.attributes" />
      </details>
    </template>
  </article>
</template>

<script setup>
import { computed, onBeforeUnmount, ref, watch } from 'vue';
import { readActionDetail, readActionLlmRequestContent } from '../../api';
import { buildLlmDetailInsight } from '../../llm/insight';
import { buildHttpDetailInsight } from '../../http/insight';
import { useModuleLocale } from '../../locale';
import InsightPanel from '../InsightPanel.vue';
import JsonTree from '../JsonTree.vue';
import LlmRequestBlocks from './LlmRequestBlocks.vue';
import strings from '../locale';

const props = defineProps({
  traceId: { type: [String, Number], required: true },
  actionId: { type: String, default: '' },
  kind: { type: String, required: true },
  contentMaxBytes: { type: Number, default: 128 * 1024 },
});
const { t } = useModuleLocale(strings);
const action = ref(null);
const loading = ref(false);
const error = ref('');
const content = ref(null);
const contentLoading = ref(false);
const contentError = ref('');
const canonicalOpen = ref(false);
const technicalOpen = ref(false);
let controller = null;
const isRequest = computed(() => props.kind === 'llm.request');
const heading = computed(() => t(isRequest.value ? 'llmCall.request' : 'llmCall.response'));
const contentMetadata = computed(() => ({
  state: action.value?.attributes?.['llm.request.content_state'],
  payloadBytes: action.value?.attributes?.['llm.request.payload_bytes'],
  blocks: action.value?.attributes?.['llm.request.block_count'],
}));
const contentAvailable = computed(() => contentMetadata.value.state === 'canonical_blocks');
const insight = computed(() => {
  if (!action.value) return null;
  const detail = { raw: action.value };
  const value = props.kind === 'http.message'
    ? buildHttpDetailInsight(detail)
    : buildLlmDetailInsight(detail, content.value);
  const attributes = action.value.attributes ?? {};
  const httpStatus = attributes['http.response.status_code'] ?? attributes.status_code;
  const httpReason = attributes['http.response.reason'] ?? attributes.reason;
  return {
    ...value,
    heading: heading.value,
    chips: [
      { label: t('llmCall.status'), value: action.value.status },
      { label: t('llmCall.completeness'), value: action.value.completeness },
      ...(httpStatus ? [{ label: 'HTTP', value: [httpStatus, httpReason].filter(Boolean).join(' ') }] : []),
      ...(value?.chips ?? []).filter((chip) => chip.label !== 'trajectory'),
    ],
    blocks: (value?.blocks ?? []).map((block) => (
      ['message-preview', 'reasoning', 'message-context', 'tool-results', 'tool-calls'].includes(block.id)
        ? { ...block, collapsible: true, defaultCollapsed: true } : block
    )),
  };
});

watch(() => [props.traceId, props.actionId, props.kind], loadAction, { immediate: true });
onBeforeUnmount(() => controller?.abort());

async function loadAction() {
  controller?.abort();
  controller = new AbortController();
  const current = controller;
  action.value = null;
  content.value = null;
  error.value = '';
  contentError.value = '';
  contentLoading.value = false;
  loading.value = Boolean(props.traceId && props.actionId);
  if (!loading.value) return;
  try {
    const result = await readActionDetail(props.traceId, props.actionId, { signal: current.signal });
    if (current.signal.aborted) return;
    if (result.id !== props.actionId || result.kind !== props.kind) {
      throw new Error(t('llmCall.invalidAction'));
    }
    action.value = result;
  } catch (failure) {
    if (!current.signal.aborted) error.value = String(failure.message ?? failure);
  } finally {
    if (!current.signal.aborted) loading.value = false;
  }
}

async function loadContent() {
  if (!contentAvailable.value || contentLoading.value || content.value) return;
  const current = controller;
  contentLoading.value = true;
  contentError.value = '';
  try {
    const result = await readActionLlmRequestContent(props.traceId, props.actionId, {
      maxBytes: props.contentMaxBytes,
      signal: current.signal,
    });
    if (current.signal.aborted) return;
    if (!result.content || result.content.action_id !== props.actionId) {
      throw new Error(t('llmCall.contentMissing'));
    }
    if (result.content.truncated) {
      throw new Error(t('detail.requestInsightsTooLarge', { kib: props.contentMaxBytes / 1024 }));
    }
    content.value = result.content;
  } catch (failure) {
    if (!current.signal.aborted) contentError.value = String(failure.message ?? failure);
  } finally {
    if (!current.signal.aborted) contentLoading.value = false;
  }
}
</script>

<style scoped>
.llm-call-message {
  min-width: 0;
  padding: 12px;
  border: 1px solid var(--ui-border);
  border-radius: 8px;
  background: var(--ui-surface);
}
.llm-call-message > h3 { margin: 0 0 8px; }
.llm-call-message :deep(.action-insight) { margin-top: 0; }
.llm-call-message :deep(.insight-block-body pre) { max-height: 10rem; overflow: auto; }
.llm-call-raw { margin-top: 12px; }
.llm-call-raw > summary { color: var(--ui-muted); cursor: pointer; font-size: 12px; }
</style>
