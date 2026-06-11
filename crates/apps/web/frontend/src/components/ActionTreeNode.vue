<template>
  <div
    class="action-tree-node"
    :class="[
      `depth-${depth}`,
      `node-${node.nodeType}`,
      `kind-${node.kindClass}`,
      node.visualClass ? `visual-${node.visualClass}` : null,
      {
        'is-expanded': childrenVisible,
        'is-selected': selectedId === node.id,
        'is-match': node.queryMatch,
      },
    ]"
  >
    <div class="action-tree-branch">
      <button
        class="action-card"
        type="button"
        :aria-expanded="hasChildren ? childrenVisible : undefined"
        @click="handleClick"
      >
        <span class="action-card-top">
          <span class="action-card-title">{{ node.title }}</span>
          <span v-if="node.durationBadge || hasChildren" class="action-card-controls">
            <DurationBadge v-if="node.durationBadge">{{ node.durationBadge }}</DurationBadge>
            <span v-if="hasChildren" class="action-card-toggle" aria-hidden="true">
              <ChevronDown v-if="childrenVisible" :size="15" />
              <ChevronRight v-else :size="15" />
            </span>
          </span>
        </span>
        <span class="action-card-meta">
          <span>{{ node.kind }}</span>
          <span v-if="node.meta">{{ node.meta }}</span>
        </span>
      </button>

      <div v-if="childrenVisible" class="action-child-lane">
        <ActionTreeNode
          v-for="child in node.children"
          :key="child.id"
          :node="child"
          :depth="depth + 1"
          :force-expanded="forceExpanded"
          :selected-id="selectedId"
          @select="$emit('select', $event)"
          @expand="$emit('expand', $event)"
        />
        <div v-if="node.loading" class="action-node-state">Loading</div>
        <div v-else-if="node.error" class="action-node-state error">{{ node.error }}</div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { ChevronDown, ChevronRight } from '@lucide/vue';

import DurationBadge from './DurationBadge.vue';

defineOptions({ name: 'ActionTreeNode' });

const props = defineProps({
  node: {
    type: Object,
    required: true,
  },
  depth: {
    type: Number,
    default: 0,
  },
  forceExpanded: {
    type: Boolean,
    default: false,
  },
  selectedId: {
    type: String,
    default: null,
  },
});

const emit = defineEmits(['select', 'expand']);
const expanded = ref(false);
const hasChildren = computed(() => props.node.hasChildren || props.node.children.length > 0);
const childrenVisible = computed(
  () => hasChildren.value && (expanded.value || (props.forceExpanded && props.node.childrenLoaded)),
);

watch(
  () => props.node.id,
  () => {
    expanded.value = false;
  },
);

function handleClick() {
  emit('select', props.node);
  if (hasChildren.value) {
    if (!props.node.childrenLoaded && !props.node.loading) {
      emit('expand', props.node);
    }
    expanded.value = !expanded.value;
  }
}
</script>

<style scoped>
.action-tree-node {
  position: relative;
}

.action-tree-node + .action-tree-node {
  margin-top: var(--action-row-gap);
}

.action-tree-branch {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: var(--action-lane-gap);
}

.action-card {
  width: var(--action-node-width);
  min-height: var(--action-node-min-height);
  display: grid;
  align-content: start;
  gap: 8px;
  padding: 11px 12px 11px 14px;
  border: 1px solid var(--border);
  border-left: 4px solid var(--teal);
  border-radius: 8px;
  background: var(--surface);
  box-shadow: var(--shadow);
  color: var(--text);
  font-size: 13px;
  line-height: 1.35;
  text-align: left;
  cursor: pointer;
}

.action-card:hover,
.action-tree-node.is-selected > .action-tree-branch > .action-card {
  border-color: var(--teal);
}

.action-tree-node.is-selected > .action-tree-branch > .action-card {
  box-shadow:
    0 0 0 2px rgba(15, 118, 110, 0.18),
    var(--shadow);
}

.action-tree-node.is-match > .action-tree-branch > .action-card {
  background: #fffbeb;
}

.action-tree-node.node-agent > .action-tree-branch > .action-card {
  border-color: #8fc5be;
  border-left-color: var(--teal-deep);
  background: #f1fbf8;
  font-weight: 800;
}

.action-tree-node.kind-llm-response > .action-tree-branch > .action-card,
.action-tree-node.kind-llm-request > .action-tree-branch > .action-card,
.action-tree-node.kind-llm-call > .action-tree-branch > .action-card {
  border-left-color: var(--amber);
}

.action-tree-node.visual-agent-call > .action-tree-branch > .action-card {
  border-color: #b8c7e8;
  border-left-color: #3158a3;
  background: #f3f6fc;
}

.action-tree-node.visual-agent-call.is-selected > .action-tree-branch > .action-card {
  border-color: #3158a3;
  box-shadow:
    0 0 0 2px rgba(49, 88, 163, 0.18),
    var(--shadow);
}

.action-tree-node.kind-payload-segment > .action-tree-branch > .action-card,
.action-tree-node.node-evidence > .action-tree-branch > .action-card {
  border-left-color: #64748b;
  background: #fbfcfc;
}

.action-card-top {
  min-width: 0;
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
}

.action-card-title {
  min-width: 0;
  overflow-wrap: anywhere;
  font-weight: 800;
}

.action-card-controls {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: flex-start;
  gap: 6px;
}

.action-card-toggle {
  flex: 0 0 auto;
  display: inline-grid;
  place-items: center;
  width: 22px;
  height: 22px;
  border: 1px solid #c8d7d5;
  border-radius: 6px;
  color: var(--teal-deep);
}

.action-card-meta {
  min-width: 0;
  display: grid;
  gap: 2px;
  color: var(--muted);
  font-size: 11px;
  font-weight: 600;
}

.action-card-meta span {
  min-width: 0;
  overflow-wrap: anywhere;
}

.action-child-lane {
  position: relative;
  min-width: var(--action-node-width);
  display: grid;
  gap: var(--action-row-gap);
}

.action-tree-node.is-expanded > .action-tree-branch::after,
.action-child-lane::before,
.action-child-lane > .action-tree-node::before {
  content: "";
  position: absolute;
  border-color: #9bbebb;
}

.action-tree-node.is-expanded > .action-tree-branch::after {
  left: var(--action-node-width);
  top: var(--action-node-center-y);
  width: calc(var(--action-lane-gap) / 2);
  border-top: 1px solid #9bbebb;
}

.action-child-lane::before {
  left: calc(var(--action-lane-gap) / -2);
  top: var(--action-node-center-y);
  bottom: var(--action-node-center-y);
  border-left: 1px solid #9bbebb;
}

.action-child-lane > .action-tree-node::before {
  left: calc(var(--action-lane-gap) / -2);
  top: var(--action-node-center-y);
  width: calc(var(--action-lane-gap) / 2);
  border-top: 1px solid #9bbebb;
}

.action-node-state {
  width: var(--action-node-width);
  min-height: 34px;
  display: grid;
  align-items: center;
  padding: 8px 10px;
  border: 1px dashed #bdd7d2;
  border-radius: 8px;
  background: #fbfcfc;
  color: var(--muted);
  font-size: 12px;
  font-weight: 700;
}

.action-node-state.error {
  border-color: #fecdd3;
  background: #fff1f2;
  color: var(--rose);
}
</style>
