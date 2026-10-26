<!-- Browse JSON children and UTF-8 string pages without retaining earlier text. -->
<template>
  <div class="lazy-json-node" :data-pointer="descriptor.pointer">
    <div v-if="inlineScalar" class="lazy-json-scalar">
      <span class="lazy-json-key">{{ descriptor.token }}</span>
      <Loader2 v-if="loading" class="spin-icon" :size="14" aria-hidden="true" />
      <pre v-else-if="node" class="lazy-json-value">{{ formattedValue }}</pre>
    </div>
    <button
      v-else
      class="lazy-json-row"
      type="button"
      :aria-expanded="expanded"
      :disabled="loading"
      @click="toggle"
    >
      <span class="lazy-json-main">
        <Loader2 v-if="loading" class="spin-icon" :size="14" aria-hidden="true" />
        <ChevronDown v-else-if="expanded" :size="14" aria-hidden="true" />
        <ChevronRight v-else :size="14" aria-hidden="true" />
        <span class="lazy-json-key">{{ descriptor.token }}</span>
      </span>
      <span class="lazy-json-summary">{{ summary }}</span>
    </button>

    <p v-if="error" class="lazy-json-error" role="alert">
      {{ error }}
      <button class="lazy-json-retry" type="button" :disabled="loading || loadingMore" @click="retry">{{ t('retry') }}</button>
    </p>

    <div v-if="(expanded || inlineScalar) && node" class="lazy-json-children">
      <template v-if="node.expandable">
        <LazyJsonTreeNode
          v-for="child in children"
          :key="child.pointer"
          :descriptor="child"
          :load-node="loadNode"
          :auto-scalars="autoScalars"
        />
        <button
          v-if="node.has_more"
          class="lazy-json-more"
          type="button"
          :disabled="loadingMore"
          @click.stop="loadMore"
        >
          <Loader2 v-if="loadingMore" class="spin-icon" :size="14" aria-hidden="true" />
          <ChevronDown v-else :size="14" aria-hidden="true" />
          <span>{{ loadingMore ? 'Loading' : 'More' }}</span>
        </button>
        <p v-if="!children.length && !node.has_more" class="lazy-json-empty">Empty</p>
      </template>
      <template v-else>
        <p v-if="node.truncated" class="lazy-json-completeness" :data-truncated="Boolean(node.truncated)">
          {{ t(node.truncated || node.offset ? 'preview' : 'complete', { start: node.offset ?? 0, end: (node.offset ?? 0) + node.returned_bytes, total: node.value_bytes }) }}
        </p>
        <pre v-if="!inlineScalar" class="lazy-json-value">{{ formattedValue }}</pre>
        <div v-if="previousOffsets.length || node.has_more" class="lazy-json-pages">
          <button class="lazy-json-previous" type="button" :disabled="loadingMore || !previousOffsets.length" @click="loadPreviousValue">
            {{ t('previous') }}
          </button>
          <button class="lazy-json-next" type="button" :disabled="loadingMore || !node.has_more" @click="loadNextValue">
            {{ t('next') }}
          </button>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue';
import { ChevronDown, ChevronRight, Loader2 } from '@lucide/vue';
import { useModuleLocale } from '../../locale';

const { t } = useModuleLocale({
  'en-US': { retry: 'Retry', preview: 'UTF-8 bytes {start}–{end} of {total}.', complete: 'Complete value: {total} UTF-8 bytes.', previous: 'Previous page', next: 'Next page' },
  'zh-CN': { retry: '重试', preview: 'UTF-8 字节范围 {start}–{end}，共 {total} 字节。', complete: '完整值：{total} 个 UTF-8 字节。', previous: '上一页', next: '下一页' },
});

const PAGE_SIZE = 50;

const props = defineProps({
  autoScalars: { type: Boolean, default: false },
  descriptor: {
    type: Object,
    required: true,
  },
  loadNode: {
    type: Function,
    required: true,
  },
});

const expanded = ref(false);
const loading = ref(false);
const loadingMore = ref(false);
const error = ref('');
const node = ref(null);
const children = ref([]);
const previousOffsets = ref([]);
let failedPage = null;
const inlineScalar = computed(() => props.autoScalars && !['object', 'array'].includes(props.descriptor.type));
onMounted(() => { if (inlineScalar.value) loadPage(0, false); });

const summary = computed(() => {
  if (node.value?.expandable) {
    return branchSummary(node.value.type, node.value.total_children);
  }
  if (node.value) {
    return node.value.type;
  }
  return branchSummary(props.descriptor.type, props.descriptor.child_count);
});
const formattedValue = computed(() => {
  if (!node.value || node.value.expandable) {
    return '';
  }
  return typeof node.value.value === 'string'
    ? node.value.value
    : JSON.stringify(node.value.value);
});

async function toggle() {
  if (loading.value) {
    return;
  }
  if (!node.value) {
    await loadPage(0, false);
    if (node.value) {
      expanded.value = true;
    }
    return;
  }
  expanded.value = !expanded.value;
}

async function loadMore() {
  if (!node.value?.has_more || loadingMore.value) {
    return;
  }
  await loadPage(node.value.next_offset ?? children.value.length, true);
}

async function loadNextValue() {
  if (loadingMore.value || !node.value?.has_more) return;
  await loadPage(node.value.next_offset, false, 'next');
}

async function loadPreviousValue() {
  if (loadingMore.value || !previousOffsets.value.length) return;
  await loadPage(previousOffsets.value.at(-1), false, 'previous');
}

async function loadPage(offset, append, navigation = null) {
  error.value = '';
  if (append || navigation) {
    loadingMore.value = true;
  } else {
    loading.value = true;
  }
  try {
    const loaded = await props.loadNode({
      pointer: props.descriptor.pointer,
      offset,
      limit: PAGE_SIZE,
    });
    if (!loaded) {
      throw new Error('JSON content is unavailable');
    }
    // Commit navigation only after success so retries keep the displayed page.
    if (navigation === 'next') previousOffsets.value.push(node.value.offset ?? 0);
    if (navigation === 'previous') previousOffsets.value.pop();
    node.value = loaded;
    children.value = append
      ? children.value.concat(loaded.children ?? [])
      : (loaded.children ?? []);
    failedPage = null;
  } catch (err) {
    failedPage = { offset, append, navigation };
    error.value = String(err.message ?? err);
  } finally {
    loading.value = false;
    loadingMore.value = false;
  }
}

async function retry() {
  if (!failedPage || loading.value || loadingMore.value) return;
  const { offset, append, navigation } = failedPage;
  await loadPage(offset, append, navigation);
  if (node.value) expanded.value = true;
}

function branchSummary(type, childCount) {
  if (type === 'object') {
    return Number.isInteger(childCount) ? `${childCount} keys` : 'object';
  }
  if (type === 'array') {
    return Number.isInteger(childCount) ? `${childCount} items` : 'array';
  }
  return type ?? 'value';
}
</script>

<style scoped>
.lazy-json-node {
  min-width: 0;
}

.lazy-json-row,
.lazy-json-more {
  width: 100%;
  border: 0;
  border-radius: 7px;
  background: transparent;
  color: inherit;
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.lazy-json-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 6px 8px;
}

.lazy-json-row:hover,
.lazy-json-more:hover {
  background: color-mix(in srgb, currentColor 7%, transparent);
}

.lazy-json-main,
.lazy-json-more {
  display: flex;
  align-items: center;
  gap: 6px;
}

.lazy-json-key {
  overflow-wrap: anywhere;
  font-family: var(--ui-mono, monospace);
}

.lazy-json-summary,
.lazy-json-empty {
  color: var(--ui-muted, #7b8494);
  font-size: 0.78rem;
}

.lazy-json-children {
  display: grid;
  gap: 4px;
  min-width: 0;
  margin: 4px 0 0 18px;
}

.lazy-json-more {
  padding: 6px 8px;
}

.lazy-json-value {
  max-height: 360px;
  margin: 2px 0 4px;
  padding: 9px 10px;
  overflow: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.lazy-json-pages {
  display: flex;
  gap: 8px;
}

.lazy-json-scalar {
  display: grid;
  grid-template-columns: minmax(90px, 1fr) minmax(0, 2fr);
  align-items: start;
  gap: 8px;
  padding: 6px 8px;
}
.lazy-json-scalar .lazy-json-value { margin: 0; padding: 0; }

.lazy-json-error {
  margin: 4px 8px;
  color: var(--ui-danger, #c84a4a);
  font-size: 0.8rem;
}

.spin-icon {
  animation: lazy-json-spin 0.9s linear infinite;
}

@keyframes lazy-json-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
