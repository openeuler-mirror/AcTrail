<!-- Switch request summaries and body details while retaining only the selected request view. -->
<template>
  <div v-if="isRequest" class="llm-insight-panel">
    <p v-if="endpoint" class="request-destination"><strong>{{ attributes['http.request.method'] }}</strong> <code>{{ endpoint }}</code></p>
    <div class="llm-view-switch">
      <ToggleSwitch
        class="request-mode-switch"
        :checked="detailView"
        :disabled="!requestContentAvailable"
        :label="t('detail')"
        :off-label="t('summary')"
        :aria-label="t('switchView')"
        @change="detailView = $event"
      />
    </div>
    <section v-if="!detailView" class="llm-request-summary">
      <dl class="detail-rows">
        <template v-for="[label, value] in facts" :key="label">
          <dt>{{ label }}</dt><dd>{{ value }}</dd>
        </template>
      </dl>
      <article v-if="messageSummary" class="llm-message-preview">
        <header>{{ t('messagePreview') }}</header>
        <div class="llm-message-summary">{{ messageSummary }}</div>
      </article>
    </section>
    <KeepAlive :key="selectionKey" :max="1">
      <LlmRequestBlocks
        v-if="detailView && requestContentAvailable"
        :key="selectionKey"
        :trace-id="traceId"
        :action-id="detail.raw.id"
      />
    </KeepAlive>
  </div>
  <InsightPanel v-else :insight="insight" />
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { buildLlmDetailInsight, llmRequestMessage } from '../llm/insight';
import { useModuleLocale } from '../locale';
import { formatTime } from '../tabs/tableModel';
import InsightPanel from './InsightPanel.vue';
import ToggleSwitch from './ToggleSwitch.vue';
import LlmRequestBlocks from './llm/LlmRequestBlocks.vue';
import { REQUEST_CONTENT_LIMITS } from './llm/constants';

const { t } = useModuleLocale({
  'en-US': { summary: 'Summary', detail: 'Detail', switchView: 'Switch between summary and detail',
    model: 'Model', provider: 'Provider', endpoint: 'Endpoint', status: 'Status',
    started: 'Started', duration: 'Duration', size: 'Request bytes', messagePreview: 'Message preview' },
  'zh-CN': { summary: '摘要', detail: '详情', switchView: '切换摘要与详情',
    model: '模型', provider: '服务商', endpoint: '请求地址', status: '状态',
    started: '开始时间', duration: '耗时', size: '请求字节数', messagePreview: '消息预览' },
});
const props = defineProps({
  traceId: { type: [String, Number], default: null },
  detail: { type: Object, default: null },
});
const detailView = ref(false);
const action = computed(() => props.detail?.raw);
const attributes = computed(() => action.value?.attributes ?? {});
const isRequest = computed(() => action.value?.kind === 'llm.request');
const selectionKey = computed(() => `${props.traceId}:${action.value?.id ?? ''}`);
const requestContentAvailable = computed(() => Boolean(props.traceId && action.value?.id
  && attributes.value['llm.request.content_state'] === 'canonical_blocks'));
const endpoint = computed(() => {
  const attrs = attributes.value;
  if (attrs['url.full']) return attrs['url.full'];
  const host = attrs['server.address'] ?? attrs.host ?? '';
  const path = attrs['url.path'] ?? '';
  const scheme = attrs['url.scheme'];
  return `${scheme && host ? `${scheme}://` : ''}${host}${path}`;
});
const insight = computed(() => {
  if (isRequest.value) return null;
  if (action.value?.kind === 'llm.call') {
    const attrs = attributes.value;
    return { instanceId: action.value.id, kind: 'llm.call', heading: '',
      chips: [
        { label: t('model'), value: attrs['llm.call.model'] },
        { label: 'HTTP', value: attrs['http.response.status_code'] },
      ].filter(item => item.value != null && item.value !== ''), blocks: [] };
  }
  return buildLlmDetailInsight(props.detail);
});
const facts = computed(() => {
  const attrs = attributes.value;
  return [
    [t('model'), attrs['llm.request.model']],
    [t('provider'), attrs['llm.request.provider_id']],
    [t('started'), action.value?.start_time ? formatTime(action.value.start_time) : null],
    [t('size'), attrs['llm.request.payload_bytes']],
  ].filter(([, value]) => value !== null && value !== undefined && value !== '');
});
const messageSummary = computed(() => {
  const message = llmRequestMessage(action.value, { preview: true });
  const limit = REQUEST_CONTENT_LIMITS.summaryCharacters;
  return message.length > limit ? `${message.slice(0, limit)}…` : message;
});
watch(selectionKey, () => { detailView.value = false; });
</script>

<style scoped>
.llm-view-switch { display: flex; align-items: center; gap: var(--ui-space-sm); margin: 8px 0 16px; }
.llm-message-preview { margin-top: 16px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); overflow: hidden; }
.llm-message-preview header { padding: 10px 14px; border-bottom: 1px solid var(--ui-border); background: var(--ui-surface-soft); color: var(--ui-muted); font-size: var(--ui-font-sm); font-weight: 600; }
.llm-message-summary { padding: 14px; white-space: pre-wrap; overflow-wrap: anywhere; font: inherit; line-height: 1.7; color: var(--ui-text); max-height: 320px; overflow: auto; }
.request-destination { margin: 8px 0; overflow-wrap: anywhere; }
.request-destination code { font: inherit; }
</style>
