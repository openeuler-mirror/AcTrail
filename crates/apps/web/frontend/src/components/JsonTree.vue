<template>
  <div class="json-tree" :class="{ root: isRoot, leaf: !isBranch }">
    <template v-if="isRoot">
      <div v-if="isBranch" class="json-children root-children">
        <JsonTree
          v-for="entry in children"
          :key="entry.path"
          :node-key="entry.key"
          :path="entry.path"
          :value="entry.value"
          :expanded-paths="expandedPaths"
          @toggle-node="$emit('toggle-node', $event)"
        />
      </div>
      <pre v-else class="json-leaf">{{ formattedLeaf }}</pre>
    </template>

    <template v-else>
      <button
        class="json-row"
        type="button"
        :aria-expanded="expanded"
        @click="toggle"
      >
        <span class="json-row-main">
          <ChevronDown v-if="expanded" :size="14" aria-hidden="true" />
          <ChevronRight v-else :size="14" aria-hidden="true" />
          <span class="json-key">{{ displayKey }}</span>
        </span>
        <span class="json-kind">{{ summary }}</span>
      </button>

      <div v-if="expanded" class="json-children">
        <template v-if="isBranch">
          <JsonTree
            v-for="entry in children"
            :key="entry.path"
            :node-key="entry.key"
            :path="entry.path"
            :value="entry.value"
            :expanded-paths="expandedPaths"
            @toggle-node="$emit('toggle-node', $event)"
          />
        </template>
        <pre v-else class="json-value-block">{{ formattedLeaf }}</pre>
      </div>
    </template>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ChevronDown, ChevronRight } from '@lucide/vue';

const props = defineProps({
  nodeKey: {
    type: [String, Number],
    default: null,
  },
  value: {
    type: null,
    required: true,
  },
  path: {
    type: String,
    default: '$',
  },
  expandedPaths: {
    type: Object,
    default: null,
  },
});

const emit = defineEmits(['toggle-node']);
const localExpanded = ref(false);
const isRoot = computed(() => props.nodeKey === null);
const displayKey = computed(() => String(props.nodeKey));
const normalizedValue = computed(() => normalizeValue(props.value));
const isBranch = computed(() => isObjectLike(normalizedValue.value));
const children = computed(() =>
  isBranch.value ? branchEntries(normalizedValue.value, props.path) : [],
);
const summary = computed(() => summarize(normalizedValue.value));
const formattedLeaf = computed(() => formatLeaf(normalizedValue.value));
const controlled = computed(() => props.expandedPaths instanceof Set);
const expanded = computed(() =>
  controlled.value ? props.expandedPaths.has(props.path) : localExpanded.value,
);

watch(
  () => props.value,
  () => {
    if (!controlled.value) {
      localExpanded.value = false;
    }
  },
);

function toggle() {
  const next = !expanded.value;
  if (controlled.value) {
    emit('toggle-node', { path: props.path, expanded: next });
    return;
  }
  localExpanded.value = next;
}

function branchEntries(value, parentPath) {
  if (Array.isArray(value)) {
    return value.map((item, index) => entry(String(index), item, `${parentPath}.${index}`));
  }
  return Object.entries(value).map(([key, item]) =>
    entry(key, item, `${parentPath}.${escapePathKey(key)}`),
  );
}

function entry(key, value, path) {
  const normalized = normalizeValue(value);
  return {
    key,
    path,
    value: normalized,
  };
}

function normalizeValue(value) {
  if (typeof value !== 'string') {
    return value;
  }
  const text = value.trim();
  if (!text.startsWith('{') && !text.startsWith('[')) {
    return value;
  }
  try {
    const parsed = JSON.parse(text);
    return isObjectLike(parsed) ? parsed : value;
  } catch {
    return value;
  }
}

function summarize(value) {
  if (Array.isArray(value)) {
    return `${value.length} items`;
  }
  if (value && typeof value === 'object') {
    return `${Object.keys(value).length} keys`;
  }
  if (value === null) {
    return 'null';
  }
  return typeof value;
}

function formatLeaf(value) {
  if (typeof value === 'string') {
    return value;
  }
  return JSON.stringify(value);
}

function isObjectLike(value) {
  return value !== null && typeof value === 'object';
}

function escapePathKey(key) {
  return key.replaceAll('.', '\\.');
}
</script>

<style scoped>
.json-tree {
  min-width: 0;
}

.json-children {
  display: grid;
  gap: 4px;
  min-width: 0;
  margin: 4px 0 0 18px;
}

.root-children {
  margin: 0;
}

.json-row {
  width: 100%;
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 7px 8px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--trace-json-row-bg);
  color: var(--text);
  cursor: pointer;
}

.json-row:hover {
  border-color: var(--trace-json-row-hover-border);
  background: var(--trace-json-row-hover-bg);
}

.json-row-main {
  flex: 1 1 auto;
  min-width: 0;
  display: inline-flex;
  align-items: flex-start;
  gap: 6px;
}

.json-key {
  min-width: 0;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 12px;
  font-weight: 700;
  overflow-wrap: anywhere;
  white-space: normal;
}

.json-kind {
  flex: 0 0 auto;
  color: var(--muted);
  font-size: 11px;
}

.json-leaf,
.json-value-block {
  margin: 4px 0 8px;
  padding: 9px 10px;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--trace-code-bg);
  color: var(--trace-code-text);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
