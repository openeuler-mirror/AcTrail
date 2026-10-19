<!-- Resolve the selected HTTP body's owner, then mount its content viewer. -->
<template>
  <section class="request-content">
    <p v-if="error" class="detail-error" role="alert">
      {{ error }} <button type="button" @click="load">{{ t('retry') }}</button>
    </p>
    <div v-else-if="result === null" role="status" :aria-label="t('loading')">
      <Loader2 class="spin-icon" :size="18" aria-hidden="true" />
    </div>
    <LlmRequestBlocks v-else-if="result.content?.type === 'llm'" :trace-id="traceId" :action-id="result.content.action_id" />
    <HttpPayload v-else-if="result.content?.type === 'http'" :trace-id="traceId" :action-id="actionId" />
    <p v-else class="detail-muted">{{ t('empty') }}</p>
  </section>
</template>

<script setup>
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { Loader2 } from '@lucide/vue';
import { readActionContentSource } from '../../api';
import { useModuleLocale } from '../../locale';
import LlmRequestBlocks from '../llm/LlmRequestBlocks.vue';
import HttpPayload from './HttpPayload.vue';

const props = defineProps({ traceId: { type: [String, Number], required: true }, actionId: { type: String, required: true } });
const { t } = useModuleLocale({
  'en-US': { loading: 'Loading content', retry: 'Retry', empty: 'No captured body.' },
  'zh-CN': { loading: '正在加载内容', retry: '重试', empty: '没有采集到正文。' },
});
const result = ref(null);
const error = ref('');
const controller = new AbortController();
async function load() {
  error.value = '';
  try {
    const response = await readActionContentSource(props.traceId, props.actionId, { signal: controller.signal });
    if (!controller.signal.aborted) result.value = response;
  } catch (cause) {
    if (!controller.signal.aborted) error.value = String(cause.message ?? cause);
  }
}
onMounted(load);
onBeforeUnmount(() => controller.abort());
</script>
