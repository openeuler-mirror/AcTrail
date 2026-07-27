<template>
  <div class="lazy-json-node">
    <button
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

    <p v-if="error" class="lazy-json-error">{{ error }}</p>

    <div v-if="expanded && node" class="lazy-json-children">
      <template v-if="node.expandable">
        <LazyJsonTreeNode
          v-for="child in children"
          :key="child.pointer"
          :descriptor="child"
          :load-node="loadNode"
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
      <pre v-else class="lazy-json-value">{{ formattedValue }}</pre>
    </div>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { ChevronDown, ChevronRight, Loader2 } from '@lucide/vue';

const PAGE_SIZE = 50;

const props = defineProps({
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

async function loadPage(offset, append) {
  error.value = '';
  if (append) {
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
    node.value = loaded;
    children.value = append
      ? children.value.concat(loaded.children ?? [])
      : (loaded.children ?? []);
  } catch (err) {
    error.value = String(err.message ?? err);
  } finally {
    loading.value = false;
    loadingMore.value = false;
  }
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
  font-family: var(--font-mono, monospace);
}

.lazy-json-summary,
.lazy-json-empty {
  color: var(--text-muted, #7b8494);
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

.lazy-json-error {
  margin: 4px 8px;
  color: var(--danger, #c84a4a);
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
