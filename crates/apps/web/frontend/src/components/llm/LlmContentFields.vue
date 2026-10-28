<!-- Read scalar fields once; keep compact values together and long content full-width. -->
<template>
  <table v-if="compactFields.length" class="llm-field-table">
    <tbody><tr v-for="field in compactFields" :key="field.pointer">
      <th scope="row">{{ field.token }}</th><td>{{ display(values[field.pointer].value) }}</td>
    </tr></tbody>
  </table>
  <template v-for="field in fields" :key="field.pointer">
    <LlmContentNode v-if="!scalar(field) || (values[field.pointer] && !compact(field))"
      :descriptor="field" :load-node="loadNode" :initial-node="values[field.pointer]"
      :presentation="presentationFor(field)" :initial-open="['body', 'block'].includes(presentationFor(field))" :page-size="pageSize" />
    <p v-else-if="errors[field.pointer]" class="detail-error" role="alert">{{ field.token }}: {{ errors[field.pointer] }} <button type="button" @click="read(field)">{{ t('retry') }}</button></p>
  </template>
  <p v-if="pending" class="detail-muted" role="status">{{ t('loading') }}</p>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { useModuleLocale } from '../../locale';
import LlmContentNode from './LlmContentNode.vue';
import { PROSE_FIELDS, REQUEST_CONTENT_LIMITS } from './constants';
const props = defineProps({ fields: { type: Array, required: true }, loadNode: Function,
  presentationFor: { type: Function, default: () => 'field' }, pageSize: Number });
const values = ref({});
const errors = ref({});
const pending = ref(0);
const requested = new Set();
const { t } = useModuleLocale({ 'en-US': { loading: 'Loading fields…', retry: 'Retry' }, 'zh-CN': { loading: '正在加载字段…', retry: '重试' } });
function scalar(field) {
  return !['object', 'array'].includes(field.type)
    && !['body', 'block', 'message'].includes(props.presentationFor(field)) && !PROSE_FIELDS.includes(field.token);
}
function display(value) { return typeof value === 'string' ? value : JSON.stringify(value); }
function compact(field) {
  const node = values.value[field.pointer];
  return scalar(field) && node && !node.has_more && !node.truncated
    && (display(node.value)?.length ?? 0) <= REQUEST_CONTENT_LIMITS.compactValueCharacters
    && !String(node.value).includes('\n');
}
const compactFields = computed(() => props.fields.filter(compact));
async function read(field) {
  requested.add(field.pointer);
  pending.value += 1;
  delete errors.value[field.pointer];
  try { values.value[field.pointer] = await props.loadNode({ pointer: field.pointer, offset: 0, limit: 1 }); }
  catch (error) { errors.value[field.pointer] = String(error.message ?? error); }
  finally { pending.value -= 1; }
}
watch(() => props.fields, fields => {
  for (const field of fields) if (scalar(field) && !requested.has(field.pointer)) read(field);
}, { immediate: true });
</script>

<style scoped>
.llm-field-table { width: 100%; border-collapse: collapse; table-layout: fixed; font-size: var(--ui-font-sm); }
th, td { padding: 9px 12px; text-align: left; vertical-align: top; border-bottom: 1px solid var(--ui-border); overflow-wrap: anywhere; }
th { width: 36%; color: var(--ui-muted); font-weight: 500; background: var(--ui-surface-soft); }
td { color: var(--ui-text); font-variant-numeric: tabular-nums; }
</style>
