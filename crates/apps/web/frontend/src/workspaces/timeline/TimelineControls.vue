<!-- Let readers choose action categories; window bounds and budgets stay internal. -->
<template>
  <section class="timeline-controls" :aria-busy="loading">
    <div class="timeline-controls-heading">
      <button class="timeline-kinds-toggle" type="button" :aria-expanded="expanded" @click="expanded = !expanded">
        <Filter :size="14" aria-hidden="true" /><span>{{ t('kinds') }}</span>
        <ChevronUp v-if="expanded" :size="14" aria-hidden="true" /><ChevronDown v-else :size="14" aria-hidden="true" />
      </button>
      <Loader2 v-if="loading" class="spin-icon" :size="15" role="status" :aria-label="t('loading')" />
    </div>
    <form v-if="expanded" @submit.prevent="apply">
      <fieldset>
        <legend>{{ t('kinds') }}</legend>
        <div class="timeline-kind-options">
          <label v-for="kind in TimelineFilters.kinds" :key="kind" class="timeline-kind-option">
            <input v-model="draft.selectedKinds" type="checkbox" :value="kind" />
            <span>{{ t(`actionKinds.${kind.replaceAll('.', '_')}`) }}</span>
          </label>
        </div>
      </fieldset>
      <button type="submit" :disabled="!draft.selectedKinds.length">{{ t('apply') }}</button>
    </form>
    <p v-if="error" role="alert">{{ t(error === 'range' ? 'invalidRange' : 'failed') }} <button v-if="error !== 'range'" type="button" :disabled="loading" @click="$emit('retry')">{{ t('retry') }}</button></p>
  </section>
</template>

<script setup>
import { reactive, ref, watch } from 'vue';
import { ChevronDown, ChevronUp, Filter, Loader2 } from '@lucide/vue';
import { useModuleLocale } from '../../locale';
import { TimelineFilters } from './filters.js';

const props = defineProps({ filters: Object, loading: Boolean, error: String });
const emit = defineEmits(['apply', 'retry']);
const draft = reactive(new TimelineFilters(props.filters));
const expanded = ref(false);
watch(() => props.filters, filters => {
  draft.reset(filters);
  if (draft.preset === 'all') draft.selectedKinds = [...TimelineFilters.kinds];
}, { immediate: true });
function apply() {
  if (!draft.selectedKinds.length) return;
  emit('apply', { ...props.filters, kinds: draft.selectedKinds.join(',') });
  expanded.value = false;
}
const { t } = useModuleLocale({
  'en-US': {
    invalidRange: 'Choose a time range within this trace.',
    kinds: 'Action categories', apply: 'Apply',
    actionKinds: {
      command_invocation: 'Command', agent_invocation: 'Agent invocation', llm_call: 'Model call',
      llm_request: 'Model request', llm_response: 'Model response', llm_tool_call: 'Tool call', llm_tool_result: 'Tool result',
      process_exec: 'Process execution', process_fork_attempt: 'Process fork', process_exit: 'Process exit',
      file_read: 'File read', file_write: 'File write', file_modify: 'File change', file_tty_io: 'Terminal I/O',
      file_bulk_read: 'Bulk file read', fs_enumerate: 'Directory listing', http_message: 'HTTP message',
      sse_stream: 'SSE stream', sse_event: 'SSE event', mcp_tool_call: 'MCP tool call',
      mcp_request: 'MCP request', mcp_response: 'MCP response', enforcement_decision: 'Policy decision',
    },
    loading: 'Loading visible range…', failed: 'Could not load this range.',
    retry: 'Retry',
  },
  'zh-CN': {
    invalidRange: '请选择此 trace 范围内的时间。',
    kinds: 'Action 类别', apply: '应用',
    actionKinds: {
      command_invocation: '命令', agent_invocation: 'Agent 调用', llm_call: '模型调用',
      llm_request: '模型请求', llm_response: '模型响应', llm_tool_call: '工具调用', llm_tool_result: '工具结果',
      process_exec: '进程执行', process_fork_attempt: '进程创建', process_exit: '进程退出',
      file_read: '文件读取', file_write: '文件写入', file_modify: '文件变更', file_tty_io: '终端读写',
      file_bulk_read: '批量文件读取', fs_enumerate: '目录列举', http_message: 'HTTP 消息',
      sse_stream: 'SSE 流', sse_event: 'SSE 事件', mcp_tool_call: 'MCP 工具调用',
      mcp_request: 'MCP 请求', mcp_response: 'MCP 响应', enforcement_decision: '策略决策',
    },
    loading: '正在加载当前范围…', failed: '当前范围加载失败。',
    retry: '重试',
  },
});
</script>

<style scoped>
.timeline-controls { padding: 8px 12px; }
.timeline-controls-heading { display: flex; align-items: center; gap: 10px; }
form { margin-top: 10px; padding: 14px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); }
fieldset { margin: 0 0 12px; padding: 0; border: 0; min-width: 0; }
legend { font-size: var(--ui-font-sm); font-weight: 600; padding: 0 0 12px; }
.timeline-kind-options { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 10px 16px; }
.timeline-kind-option { display: flex; align-items: center; gap: 8px; font-size: var(--ui-font-sm); cursor: pointer; }
.timeline-kind-option input { accent-color: var(--ui-accent); width: 15px; height: 15px; margin: 0; }
button { display: inline-flex; align-items: center; gap: 6px; padding: 6px 10px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); color: var(--ui-text); font: inherit; font-size: var(--ui-font-sm); cursor: pointer; }
button:disabled { opacity: .5; cursor: default; }
p { font-size: var(--ui-font-sm); margin: 8px 0; }
</style>
