<!-- Present HTTP destination and status before resolving its captured body. -->
<template>
  <section v-if="detail?.raw?.kind === 'http.message'" class="http-request-detail">
    <p v-if="endpoint" class="http-destination">{{ endpoint }}</p>
    <dl v-if="facts.length" class="detail-rows">
      <template v-for="[key, value] in facts" :key="key"><dt>{{ key }}</dt><dd>{{ value }}</dd></template>
    </dl>
    <RequestContent v-if="traceId" :key="`${traceId}:${detail.raw.id}`" :trace-id="traceId" :action-id="detail.raw.id" />
  </section>
  <InsightPanel v-else :insight="insight" />
</template>

<script setup>
import { computed } from 'vue';

import { buildHttpDetailInsight } from '../http/insight';
import InsightPanel from './InsightPanel.vue';
import RequestContent from './http/RequestContent.vue';
import { useModuleLocale } from '../locale';

const props = defineProps({
  traceId: { type: [String, Number], default: null },
  detail: {
    type: Object,
    default: null,
  },
});

const insight = computed(() => buildHttpDetailInsight(props.detail));
const { t } = useModuleLocale({
  'en-US': { status: 'Status', contentType: 'Content type', size: 'Body bytes', duration: 'Duration' },
  'zh-CN': { status: '状态', contentType: '内容类型', size: '正文字节数', duration: '耗时' },
});
const endpoint = computed(() => {
  const attrs = props.detail?.raw?.attributes ?? {};
  if (attrs['url.full']) return attrs['url.full'];
  const block = insight.value?.blocks.find(item => item.id === 'http-endpoint');
  const rows = Object.fromEntries(block?.rows ?? []);
  return rows.endpoint_url ?? rows.website ?? rows.endpoint ?? '';
});
const facts = computed(() => {
  const attrs = props.detail?.raw?.attributes ?? {};
  return [
    [t('status'), attrs['http.response.status_code'] ?? attrs.status_code],
    [t('contentType'), attrs.content_type],
    [t('size'), attrs.content_length],
    [t('duration'), props.detail?.rows?.duration],
  ].filter(([, value]) => value != null && value !== '');
});
</script>

<style scoped>
.http-destination { overflow-wrap: anywhere; }
.http-request-detail .request-content { margin-top: 16px; }
</style>
