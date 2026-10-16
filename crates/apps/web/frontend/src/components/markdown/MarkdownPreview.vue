<template>
  <div v-if="rich_text_enabled" class="markdown-preview" v-html="html" />
  <pre v-else class="markdown-preview-raw">{{ content }}</pre>
</template>

<script>
import MarkdownIt from 'markdown-it';
// Observed text is untrusted: no embedded HTML or remote image requests.
const markdown = new MarkdownIt({ html: false, breaks: true }).disable('image');
</script>
<script setup>
import { computed } from 'vue';
const props = defineProps({ content: { type: String, default: '' }, rich_text_enabled: { type: Boolean, default: true } });
const html = computed(() => props.rich_text_enabled ? markdown.render(props.content) : '');
</script>

<style scoped>
.markdown-preview { min-width: 0; color: var(--ui-text); font-size: var(--ui-font-sm); line-height: 1.75; overflow-wrap: anywhere; }
.markdown-preview :deep(> :first-child) { margin-top: 0; }
.markdown-preview :deep(> :last-child) { margin-bottom: 0; }
.markdown-preview :deep(p) { margin: 0 0 12px; }
.markdown-preview :deep(h1), .markdown-preview :deep(h2), .markdown-preview :deep(h3), .markdown-preview :deep(h4) { margin: 20px 0 10px; font-size: 1.05em; line-height: 1.5; font-weight: 650; color: var(--ui-text); }
.markdown-preview :deep(ul), .markdown-preview :deep(ol) { margin: 8px 0 14px; padding-left: 22px; }
.markdown-preview :deep(li + li) { margin-top: 5px; }
.markdown-preview :deep(code) { font-family: ui-monospace, monospace; font-size: .92em; background: var(--ui-surface-soft); border-radius: 4px; padding: 2px 4px; }
.markdown-preview :deep(pre) { margin: 12px 0; padding: 12px 14px; border: 1px solid var(--trace-code-border); border-radius: var(--ui-radius-md); background: var(--trace-code-bg); color: var(--trace-code-text); overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; }
.markdown-preview :deep(pre code) { padding: 0; border-radius: 0; background: transparent; color: inherit; }
.markdown-preview :deep(blockquote) { margin: 12px 0; padding: 4px 12px; border-left: 3px solid var(--ui-border); color: var(--ui-muted); }
.markdown-preview :deep(a) { color: var(--ui-accent); text-decoration: underline; }
.markdown-preview :deep(table) { display: block; max-width: 100%; overflow: auto; border-collapse: collapse; margin: 12px 0; }
.markdown-preview :deep(th), .markdown-preview :deep(td) { padding: 7px 10px; border: 1px solid var(--ui-border); text-align: left; }
.markdown-preview :deep(th) { background: var(--ui-surface-soft); }
.markdown-preview :deep(hr) { border: 0; border-top: 1px solid var(--ui-border); margin: 16px 0; }
.markdown-preview-raw { margin: 0; padding: 0; border: 0; background: transparent; color: var(--ui-text); white-space: pre-wrap; overflow-wrap: anywhere; font-family: ui-monospace, monospace; font-size: var(--ui-font-sm); line-height: 1.7; }
</style>
