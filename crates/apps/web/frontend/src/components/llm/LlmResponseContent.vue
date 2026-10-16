<!-- Response content has one reading order: answer, tool calls, optional reasoning. -->
<template>
  <div class="llm-response-content">
    <div v-if="facts.length" class="response-facts">
      <div v-for="[label, value] in facts" :key="label" class="response-fact"><span>{{ label }}</span><strong>{{ value }}</strong></div>
    </div>
    <dl v-if="times.length" class="response-times">
      <template v-for="[label, value] in times" :key="label"><dt>{{ label }}</dt><dd>{{ value }}</dd></template>
    </dl>
    <div v-if="answer || reasoning" class="response-view-control">
      <ToggleSwitch :checked="richText" :label="t('richText')" :aria-label="t('richText')" @change="richText = $event" />
    </div>
    <section v-if="answer" class="response-answer" :aria-label="t('answer')">
      <MarkdownPreview :content="answer" :rich_text_enabled="richText" />
    </section>
    <section v-if="tools.length" class="response-tools">
      <h3>{{ t('tools') }} <span>{{ tools.length }}</span></h3>
      <article v-for="tool in tools" :key="tool.id" class="response-tool">
        <header>{{ tool.title }}</header>
        <pre v-if="tool.text">{{ tool.text }}</pre>
      </article>
    </section>
    <details v-if="reasoning" class="response-reasoning" @toggle="reasoningOpened = reasoningOpened || $event.target.open">
      <summary>{{ t('reasoning') }}</summary>
      <MarkdownPreview v-if="reasoningOpened" class="response-reasoning-body" :content="reasoning" :rich_text_enabled="richText" />
    </details>
    <p v-if="!answer && !tools.length && !reasoning" class="detail-muted">{{ t('empty') }}</p>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { useModuleLocale } from '../../locale';
import { buildLlmDetailInsight } from '../../llm/insight';
import { formatTime } from '../../tabs/tableModel';
import MarkdownPreview from '../markdown/MarkdownPreview.vue';
import ToggleSwitch from '../ToggleSwitch.vue';
const props = defineProps({ detail: { type: Object, required: true } });
const { t } = useModuleLocale({
  'en-US': { answer: 'Response', richText: 'Rich text', model: 'Model', provider: 'Provider', tools: 'Tool calls', reasoning: 'Reasoning', started: 'Started', ended: 'Ended', duration: 'Duration', ttft: 'Time to first token', input: 'Input tokens', cached: 'Cached input tokens', output: 'Output tokens', reasoningTokens: 'Reasoning tokens', total: 'Total tokens', empty: 'No response content recorded.' },
  'zh-CN': { answer: '回复正文', richText: '富文本', model: '模型', provider: '服务商', tools: '工具调用', reasoning: '推理内容', started: '开始时间', ended: '结束时间', duration: '耗时', ttft: '首 token 延迟', input: '输入 token', cached: '缓存输入 token', output: '输出 token', reasoningTokens: '推理 token', total: '总 token', empty: '未记录响应内容。' },
});
const richText = ref(true);
const reasoningOpened = ref(false);
const attrs = computed(() => props.detail.raw?.attributes ?? {});
const answer = computed(() => attrs.value['llm.response.content_text']);
const reasoning = computed(() => attrs.value['llm.response.reasoning_text']);
const tools = computed(() => buildLlmDetailInsight(props.detail)?.blocks.find(block => block.id === 'tool-calls')?.items ?? []);
const facts = computed(() => [
  [t('model'), attrs.value['llm.response.model']],
  [t('provider'), attrs.value['llm.response.provider_id']],
  [t('duration'), props.detail.rows?.duration],
  [t('ttft'), props.detail.rows?.ttft],
  [t('input'), attrs.value['llm.response.prompt_tokens']],
  [t('cached'), attrs.value['llm.response.cached_prompt_tokens']],
  [t('output'), attrs.value['llm.response.completion_tokens']],
  [t('reasoningTokens'), attrs.value['llm.response.reasoning_tokens']],
  [t('total'), attrs.value['llm.response.total_tokens']],
].filter(([, value]) => value !== null && value !== undefined && value !== ''));
const times = computed(() => [
  [t('started'), props.detail.rows?.started ? formatTime(props.detail.rows.started) : null],
  [t('ended'), props.detail.rows?.ended ? formatTime(props.detail.rows.ended) : null],
].filter(([, value]) => value));
</script>

<style scoped>
.llm-response-content { display: grid; gap: 18px; min-width: 0; }
.response-facts { display: flex; flex-wrap: wrap; gap: 8px; }
.response-fact { flex: 1 1 120px; min-width: 0; display: flex; flex-direction: column; gap: 5px; padding: 10px 12px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface-soft); }
.response-fact span { color: var(--ui-muted); font-size: var(--ui-font-xs); }
.response-fact strong { font-size: var(--ui-font-sm); font-weight: 600; font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
.response-times { display: grid; grid-template-columns: auto 1fr; gap: 6px 12px; margin: 0; font-size: var(--ui-font-xs); color: var(--ui-muted); }
.response-times dd { margin: 0; text-align: right; font-variant-numeric: tabular-nums; }
.response-view-control { display: flex; justify-content: flex-end; }
h3 { margin: 0 0 10px; font-size: var(--ui-font-sm); font-weight: 600; }
h3 span { color: var(--ui-muted); margin-left: 6px; font-weight: 400; }
.response-answer { padding: 14px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); }
.response-tool { border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); overflow: hidden; }
.response-tool header { margin: 0; padding: 10px 14px; border-bottom: 1px solid var(--ui-border); background: var(--ui-surface-soft); font-size: var(--ui-font-sm); font-weight: 600; }
.response-reasoning-body { margin-top: 12px; }
.response-tool + .response-tool { margin-top: 10px; }
.response-tool pre { margin: 0; padding: 14px; max-height: 360px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; background: transparent; border: 0; font-size: var(--ui-font-sm); line-height: 1.6; }
.response-reasoning { border-top: 1px solid var(--ui-border); padding-top: 12px; }
summary { color: var(--ui-muted); font-size: var(--ui-font-sm); cursor: pointer; }
</style>
