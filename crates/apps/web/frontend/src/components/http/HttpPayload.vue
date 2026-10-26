<!-- Read one bounded HTTP body page at a time and retain only navigation offsets. -->
<template>
  <section class="http-payload">
    <div v-if="loading" role="status" :aria-label="t('loading')"><Loader2 class="spin-icon" :size="18" aria-hidden="true" /></div>
    <p v-if="error" class="detail-error" role="alert">
      {{ error }} <button type="button" :disabled="loading" @click="load(failedOffset, failedDirection)">{{ t('retry') }}</button>
    </p>
    <template v-if="page">
      <p v-if="page.kind === 'base64'" class="detail-muted">{{ t('binary') }}</p>
      <pre class="http-payload-text">{{ page.text }}</pre>
      <nav v-if="previousOffsets.length || page.has_more" class="http-payload-pages" :aria-label="t('pages')">
        <button class="http-payload-previous" type="button" :disabled="loading || !previousOffsets.length" @click="load(previousOffsets.at(-1), 'previous')">{{ t('previous') }}</button>
        <button class="http-payload-next" type="button" :disabled="loading || !page.has_more" @click="load(page.next_offset, 'next')">{{ t('next') }}</button>
      </nav>
    </template>
    <p v-else-if="!loading && !error" class="detail-muted">{{ t('empty') }}</p>
  </section>
</template>

<script setup>
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { Loader2 } from '@lucide/vue';
import { readActionHttpContent } from '../../api';
import { useModuleLocale } from '../../locale';

const props = defineProps({ traceId: { type: [String, Number], required: true }, actionId: { type: String, required: true } });
const { t } = useModuleLocale({
  'en-US': { loading: 'Loading body', retry: 'Retry', empty: 'No captured body.', binary: 'Binary body (Base64)', pages: 'Body pages', previous: 'Previous page', next: 'Next page' },
  'zh-CN': { loading: '正在加载正文', retry: '重试', empty: '没有采集到正文。', binary: '二进制正文（Base64）', pages: '正文分页', previous: '上一页', next: '下一页' },
});
const page = ref(null);
const previousOffsets = ref([]);
const loading = ref(false);
const error = ref('');
const controller = new AbortController();
let failedOffset = 0;
let failedDirection = null;
async function load(offset = 0, direction = null) {
  if (loading.value) return;
  loading.value = true;
  error.value = '';
  try {
    const response = await readActionHttpContent(props.traceId, props.actionId, { offset, signal: controller.signal });
    if (controller.signal.aborted) return;
    if (direction === 'next') previousOffsets.value.push(page.value.offset);
    if (direction === 'previous') previousOffsets.value.pop();
    page.value = response.content;
  } catch (cause) {
    if (!controller.signal.aborted) {
      failedOffset = offset;
      failedDirection = direction;
      error.value = String(cause.message ?? cause);
    }
  } finally {
    loading.value = false;
  }
}
onMounted(() => load());
onBeforeUnmount(() => controller.abort());
</script>

<style scoped>
.http-payload-text { max-height: 60vh; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; }
.http-payload-pages { display: flex; gap: var(--ui-space-sm); margin-top: 8px; }
</style>
