<!-- Browse persisted request fields and their content without loading the complete body. -->
<template>
  <section class="llm-request-blocks detail-section" :aria-label="t('title')">
    <LlmContentNode
      v-for="field in contentFields"
      :key="field.pointer"
      :descriptor="field"
      :load-node="loadNode"
      :label="t(field.token)"
      presentation="section"
      :page-size="entriesPerPage"
    />
    <section v-if="parameterFields.length" class="llm-request-parameters">
      <h3>{{ t('parameters') }}</h3>
      <LlmContentFields :fields="parameterFields" :load-node="loadNode" :page-size="entriesPerPage" />
    </section>
    <p v-if="loading" class="detail-muted">{{ t('loading') }}</p>
    <p v-if="error" class="detail-error" role="alert">{{ error }}</p>
    <button
      v-if="error || nextOffset !== null"
      class="detail-load-button llm-request-fields-more"
      type="button"
      :disabled="loading"
      @click="loadFields"
    >{{ t(error ? 'retry' : 'more') }}</button>
    <p v-if="!loading && !error && !fields.length && nextOffset === null" class="detail-muted">{{ t('empty') }}</p>
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { useModuleLocale } from '../../locale';
import LlmContentNode from './LlmContentNode.vue';
import LlmContentFields from './LlmContentFields.vue';
import { NodeReader } from './NodeReader';
import { REQUEST_CONTENT_LIMITS } from './constants';

const props = defineProps({
  traceId: { type: [String, Number], required: true },
  actionId: { type: String, required: true },
  readConcurrency: { type: Number, default: REQUEST_CONTENT_LIMITS.readConcurrency },
  entriesPerPage: { type: Number, default: REQUEST_CONTENT_LIMITS.entriesPerPage },
});
const { t } = useModuleLocale({
  'en-US': {
    title: 'Request content', hint: 'Open a message, tool or other field to browse its content. Long text is shown in pages.',
    loading: 'Loading fields', retry: 'Retry', more: 'More request fields', empty: 'This request has no fields.',
    messages: 'Messages', input: 'Input', system: 'System instructions', tools: 'Tool definitions', functions: 'Function definitions', prompt: 'Prompt', parameters: 'Request settings',
  },
  'zh-CN': {
    title: '请求内容', hint: '展开消息、工具或其他字段查看内容，长文本按页显示。',
    loading: '正在加载字段', retry: '重试', more: '更多请求字段', empty: '此请求没有字段。',
    messages: '消息', input: '输入', system: '系统指令', tools: '工具定义', functions: '函数定义', prompt: '提示内容', parameters: '请求参数',
  },
});
const descriptors = ref([]);
const nextOffset = ref(0);
const loading = ref(false);
const error = ref('');
const reader = new NodeReader(props.traceId, props.actionId, props.readConcurrency);
const fieldOrder = ['messages', 'input', 'prompt', 'system', 'tools', 'functions'];
const fields = computed(() => descriptors.value);
const contentFields = computed(() => descriptors.value.filter(field => fieldOrder.includes(field.token))
  .sort((left, right) => fieldOrder.indexOf(left.token) - fieldOrder.indexOf(right.token)));
const parameterFields = computed(() => descriptors.value.filter(field => !fieldOrder.includes(field.token)));

async function loadNode(query) {
  return reader.read(query);
}

async function loadFields() {
  if (loading.value || nextOffset.value === null) return;
  loading.value = true;
  error.value = '';
  try {
    // Only root descriptions are read here; each field loads its own content on demand.
    const root = await loadNode({ pointer: '', offset: nextOffset.value, limit: REQUEST_CONTENT_LIMITS.fieldsPerPage });
    if (root.type !== 'object') throw new Error('Request body must be an object');
    descriptors.value.push(...root.children);
    nextOffset.value = root.next_offset ?? null;
  } catch (cause) {
    error.value = String(cause.message ?? cause);
  } finally {
    loading.value = false;
  }
}

onMounted(loadFields);
onBeforeUnmount(() => reader.close());
</script>

<style scoped>
.llm-request-blocks { display: grid; gap: 14px; }
.llm-request-parameters { padding: 14px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); display: grid; gap: 6px; }
.llm-request-parameters h3 { margin: 0 0 8px; color: var(--ui-muted); font-size: var(--ui-font-sm); font-weight: 600; }
</style>
