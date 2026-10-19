<!-- Read request messages as cards, keeping transport nesting out of the reading layout. -->
<template>
  <article class="llm-content-node" :class="[presentation, { 'is-container': container, 'is-prose': prose, 'is-open': expanded }]" :data-pointer="descriptor.pointer">
    <button v-if="container && !transparent" class="llm-content-heading" type="button" :aria-expanded="expanded" :disabled="loading" @click="toggle">
      <span class="llm-content-title">
        <span v-if="header.role || header.type" class="llm-role">{{ header.role || header.type }}</span>
        <span v-if="header.role && header.type && header.type !== header.role" class="llm-role">{{ header.type }}</span>
        <strong>{{ title }}</strong>
      </span>
      <span class="llm-content-trailing">
        <span v-if="count !== null" class="llm-content-count">{{ count }}</span>
        <Loader2 v-if="loading" class="spin-icon" :size="15" />
        <ChevronDown v-else-if="expanded" :size="15" />
        <ChevronRight v-else :size="15" />
      </span>
    </button>
    <template v-else-if="!container">
      <span v-if="!prose" class="llm-field-label">{{ title }}</span>
      <span v-else-if="!transparent" class="llm-prose-label">{{ title }}</span>
      <Loader2 v-if="loading" class="spin-icon" :size="15" />
      <div v-if="node" class="llm-content-value">{{ value }}</div>
    </template>
    <div v-if="container && transparent && loading" role="status" :aria-label="t('loading')"><Loader2 class="spin-icon" :size="15" /></div>
    <div v-if="container && opened" v-show="expanded" class="llm-content-body">
      <BlockLoadControls v-if="node && (descriptor.type === 'array' || node.has_more)" :loaded="children.length" :total="node.total_children" :has-more="node.has_more" :loading="loading"
        :default-size="descriptor.type === 'array' ? pageSize : REQUEST_CONTENT_LIMITS.fieldsPerPage" @load="load(node.next_offset, 'append', $event)" />
      <LlmContentFields :fields="visibleChildren" :load-node="loadNode" :presentation-for="childPresentation" :page-size="pageSize" />
      <p v-if="node && !children.length" class="llm-content-empty">{{ t('empty') }}</p>
    </div>
    <p v-if="error" class="detail-error" role="alert">{{ error }} <button type="button" :disabled="loading" @click="retry">{{ t('retry') }}</button></p>
    <nav v-if="!container && node && (node.has_more || previousOffsets.length)" class="llm-content-pages" :aria-label="t('pages')">
      <span>{{ t('range', { start: (node.offset ?? 0) + 1, end: (node.offset ?? 0) + node.returned_bytes, total: node.value_bytes }) }}</span>
      <button class="llm-content-previous" type="button" :disabled="loading || !previousOffsets.length" @click="load(previousOffsets.at(-1), 'previous')">{{ t('previous') }}</button>
      <button class="llm-content-next" type="button" :disabled="loading || !node.has_more" @click="load(node.next_offset, 'next')">{{ t('next') }}</button>
    </nav>
  </article>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { ChevronDown, ChevronRight, Loader2 } from '@lucide/vue';
import { useModuleLocale } from '../../locale';
import { PROSE_FIELDS, REQUEST_CONTENT_LIMITS } from './constants';
import BlockLoadControls from './BlockLoadControls.vue';
import LlmContentFields from './LlmContentFields.vue';

const props = defineProps({
  descriptor: { type: Object, required: true },
  loadNode: { type: Function, required: true },
  presentation: { type: String, default: 'field' },
  label: { type: String, default: '' },
  initialOpen: { type: Boolean, default: false },
  initialNode: { type: Object, default: null },
  pageSize: { type: Number, default: REQUEST_CONTENT_LIMITS.entriesPerPage },
});
const { t } = useModuleLocale({
  'en-US': { message: 'Message {number}', tool: 'Tool {number}', block: 'Content {number}', more: 'Show more', empty: 'Empty', retry: 'Retry', loading: 'Loading content', previous: 'Previous page', next: 'Next page', pages: 'Text pages', range: 'Bytes {start}–{end} / {total}', parameters: 'Parameters', description: 'Description', content: 'Content', text: 'Text' },
  'zh-CN': { message: '消息 {number}', tool: '工具 {number}', block: '内容 {number}', more: '显示更多', empty: '无内容', retry: '重试', loading: '正在加载内容', previous: '上一页', next: '下一页', pages: '正文分页', range: '字节 {start}–{end} / {total}', parameters: '参数', description: '说明', content: '内容', text: '正文' },
});
const node = ref(props.initialNode);
const children = ref([]);
const header = ref({});
const expanded = ref(props.initialOpen);
const opened = ref(props.initialOpen);
const loading = ref(false);
const error = ref('');
const previousOffsets = ref([]);
let failed = null;
const container = computed(() => ['object', 'array'].includes(props.descriptor.type));
const entry = computed(() => ['message', 'tool', 'body'].includes(props.presentation));
const transparent = computed(() => props.presentation === 'body' && !header.value.name
  && (!header.value.type || ['text', 'input_text', 'output_text'].includes(header.value.type)));
const prose = computed(() => props.presentation === 'body' || (!container.value && props.presentation === 'message')
  || PROSE_FIELDS.includes(props.descriptor.token));
const title = computed(() => {
  if (header.value.name) return header.value.name;
  if (props.label) return props.label;
  const token = String(props.descriptor.token);
  if (/^\d+$/.test(token)) return t(props.presentation === 'message' ? 'message' : props.presentation === 'tool' ? 'tool' : 'block', { number: Number(token) + 1 });
  if (['input_schema', 'parameters'].includes(token)) return t('parameters');
  if (['description', 'content', 'text'].includes(token)) return t(token);
  return token;
});
const count = computed(() => props.presentation === 'section' && container.value ? (node.value?.total_children ?? props.descriptor.child_count ?? null) : null);
const value = computed(() => typeof node.value?.value === 'string' ? node.value.value : JSON.stringify(node.value?.value));
const visibleChildren = computed(() => children.value.filter(child => !Object.hasOwn(header.value, child.token)));

function childPresentation(child) {
  const token = props.descriptor.token;
  if (props.descriptor.type === 'array') {
    if (['messages', 'input'].includes(token)) return 'message';
    if (['tools', 'functions', 'tool_calls'].includes(token)) return 'tool';
    if (props.presentation === 'body' || token === 'system') return 'body';
  }
  if (entry.value && ['content', 'text', 'description'].includes(child.token)) return 'body';
  return 'field';
}

async function load(offset = 0, navigation = null, limit = props.descriptor.type === 'array' ? props.pageSize : REQUEST_CONTENT_LIMITS.fieldsPerPage) {
  if (loading.value) return;
  loading.value = true;
  error.value = '';
  try {
    const result = await props.loadNode({ pointer: props.descriptor.pointer, offset,
      limit });
    // Header metadata is read without fetching message bodies or tool schemas.
    let metadata = header.value;
    if (entry.value && result.type === 'object' && navigation !== 'append') {
      const fields = result.children.filter(child => ['role', 'name', 'type'].includes(child.token) && child.type === 'string');
      metadata = Object.fromEntries((await Promise.all(fields.map(async child => {
        const result = await props.loadNode({ pointer: child.pointer, offset: 0, limit: 1 });
        return result.truncated || result.has_more ? null : [child.token, result.value];
      }))).filter(Boolean));
    }
    if (navigation === 'next') previousOffsets.value.push(node.value.offset ?? 0);
    if (navigation === 'previous') previousOffsets.value.pop();
    header.value = metadata;
    node.value = result;
    children.value = navigation === 'append' ? children.value.concat(result.children ?? []) : (result.children ?? []);
    failed = null;
  } catch (cause) {
    failed = { offset, navigation, limit };
    error.value = String(cause.message ?? cause);
  } finally {
    loading.value = false;
  }
}
async function toggle() {
  if (!node.value) await load();
  if (!node.value) return;
  expanded.value = !expanded.value;
  if (expanded.value) opened.value = true;
}
async function retry() {
  if (failed) await load(failed.offset, failed.navigation, failed.limit);
}
onMounted(() => {
  if (!node.value && (!container.value || entry.value || props.initialOpen)) load();
});
</script>

<style scoped>
.llm-content-node { min-width: 0; color: var(--ui-text); }
.llm-content-node.is-container:not(.body) { border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); overflow: hidden; }
.llm-content-node.message { border-left: 3px solid var(--ui-accent); }
.llm-content-node.tool { border-left: 3px solid var(--trace-insight-tools-border); }
.llm-content-node.section.is-container { border: 0; background: transparent; overflow: visible; }
.llm-content-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; width: 100%; padding: 12px 14px; border: 0; background: var(--ui-surface-soft); color: inherit; font: inherit; cursor: pointer; text-align: left; }
.llm-content-heading:hover { background: var(--ui-accent-faint); }
.llm-content-heading:focus-visible { outline: 2px solid var(--ui-accent); outline-offset: -2px; }
.llm-content-heading:disabled { cursor: wait; }
.llm-content-title { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; min-width: 0; overflow-wrap: anywhere; }
.llm-content-title strong { font-size: var(--ui-font-sm); font-weight: 600; }
.llm-role { border-radius: var(--ui-radius-pill); background: var(--ui-accent-faint); color: var(--ui-accent); padding: 3px 8px; font-size: var(--ui-font-xs); font-weight: 600; }
.llm-content-trailing { display: flex; align-items: center; gap: 8px; color: var(--ui-muted); flex-shrink: 0; }
.llm-content-count { font-size: var(--ui-font-xs); font-variant-numeric: tabular-nums; }
.llm-content-body { display: grid; gap: 12px; padding: 14px; border-top: 1px solid var(--ui-border); }
.section > .llm-content-heading { padding: 8px 0; background: transparent; }
.section > .llm-content-body { padding: 8px 0; border: 0; }
.body > .llm-content-body { padding: 0; border: 0; }
.llm-content-node:not(.is-container):not(.is-prose) { display: grid; grid-template-columns: minmax(80px, 1fr) minmax(0, 2fr); align-items: baseline; gap: 12px; padding: 8px 0; border-bottom: 1px solid var(--ui-border); }
.llm-field-label, .llm-prose-label { color: var(--ui-muted); font-size: var(--ui-font-sm); overflow-wrap: anywhere; }
.llm-prose-label { display: block; margin-bottom: 8px; font-weight: 600; }
.llm-content-value { min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.7; font: inherit; }
.is-prose > .llm-content-value { line-height: 1.75; max-height: 480px; overflow: auto; padding: 12px 14px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface-soft); }
.body.is-prose > .llm-content-value { padding: 0; border: 0; border-radius: 0; background: transparent; }
.llm-content-pages { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; padding-top: 10px; grid-column: 1 / -1; }
.llm-content-pages span { flex: 1 1 100%; color: var(--ui-muted); font-size: var(--ui-font-xs); }
.llm-content-pages button, .llm-content-more { padding: 6px 10px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); color: var(--ui-text); background: var(--ui-surface); font: inherit; font-size: var(--ui-font-sm); cursor: pointer; }
.llm-content-pages button:disabled, .llm-content-more:disabled { opacity: .5; cursor: default; }
.llm-content-more { justify-self: start; }
.llm-content-empty { margin: 0; color: var(--ui-muted); font-size: var(--ui-font-sm); }
</style>
