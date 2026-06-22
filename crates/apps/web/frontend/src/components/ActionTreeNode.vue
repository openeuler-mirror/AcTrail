<template>
  <div
    class="action-tree-node"
    :data-action-node-id="node.id"
    :class="[
      `depth-${depth}`,
      `node-${node.nodeType}`,
      `kind-${node.kindClass}`,
      node.visualClass ? `visual-${node.visualClass}` : null,
      statusMarker ? `status-${statusMarker.tone}` : null,
      {
        'is-expanded': childrenVisible,
        'is-selected': selectedId === node.id,
        'is-match': node.queryMatch,
      },
    ]"
  >
    <div class="action-tree-branch">
      <div
        v-if="showLlmCallJump"
        class="llm-call-jump-controls"
        aria-label="Jump between sibling LLM calls"
      >
        <button
          type="button"
          :disabled="!llmCallNav?.previous"
          aria-label="Previous sibling LLM call"
          title="Previous LLM call"
          @click.stop="jumpTo(llmCallNav.previous)"
        >
          <span class="jump-triangle up" aria-hidden="true"></span>
        </button>
        <button
          type="button"
          :disabled="!llmCallNav?.next"
          aria-label="Next sibling LLM call"
          title="Next LLM call"
          @click.stop="jumpTo(llmCallNav.next)"
        >
          <span class="jump-triangle down" aria-hidden="true"></span>
        </button>
      </div>

      <div
        class="action-card"
        role="button"
        tabindex="0"
        :aria-pressed="selectedId === node.id"
        @click="selectNode"
        @keydown.enter.prevent="selectNode"
        @keydown.space.prevent="selectNode"
      >
        <span class="action-card-top">
          <span class="action-card-title">{{ node.title }}</span>
          <span v-if="statusMarker || node.durationBadge || hasChildren" class="action-card-controls">
            <span
              v-if="statusMarker"
              class="action-status-marker"
              :class="`tone-${statusMarker.tone}`"
              :title="`Status: ${statusMarker.raw}`"
              :aria-label="`Status: ${statusMarker.raw}`"
            >
              <component :is="statusMarker.icon" :size="15" :stroke-width="2.6" />
            </span>
            <DurationBadge v-if="node.durationBadge">{{ node.durationBadge }}</DurationBadge>
            <button
              v-if="hasChildren"
              class="action-card-toggle"
              type="button"
              :aria-expanded="childrenVisible"
              :aria-label="childrenVisible ? 'Collapse action children' : 'Expand action children'"
              :title="childrenVisible ? 'Collapse' : 'Expand'"
              @click.stop="toggleExpanded"
            >
              <ChevronDown v-if="childrenVisible" :size="15" />
              <ChevronRight v-else :size="15" />
            </button>
          </span>
        </span>
        <span class="action-card-meta">
          <span>{{ node.kind }}</span>
          <span v-if="node.meta">{{ node.meta }}</span>
        </span>
      </div>

      <div v-if="childrenVisible" class="action-child-lane">
        <template v-for="(child, index) in node.children" :key="child.id">
          <ActionTreeNode
            :node="child"
            :depth="depth + 1"
            :force-expanded="forceExpanded"
            :selected-id="selectedId"
            :llm-call-nav="llmCallNavigation(index)"
            @select="$emit('select', $event)"
            @expand="$emit('expand', $event)"
            @load-more="$emit('load-more', $event)"
            @jump="$emit('jump', $event)"
          />
          <div
            v-if="index === prefetchSentinelIndex"
            ref="prefetchSentinel"
            class="action-prefetch-sentinel"
            aria-hidden="true"
          ></div>
        </template>
        <div v-if="node.loading" class="action-node-state">Loading</div>
        <div v-else-if="node.loadingMore" class="action-node-state">Loading</div>
        <div v-else-if="node.error" class="action-node-state error">{{ node.error }}</div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { CheckCircle2, ChevronDown, ChevronRight, CircleHelp, Clock3, XCircle } from '@lucide/vue';

import { UI_LIMITS } from '../tabs/core/action-tree/config';
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
  llmCallNav: {
    type: Object,
    default: null,
  },
});

const emit = defineEmits(['select', 'expand', 'load-more', 'jump']);
const expanded = ref(false);
const prefetchSentinel = ref(null);
let prefetchObserver = null;
const hasChildren = computed(() => props.node.hasChildren || props.node.children.length > 0);
const showLlmCallJump = computed(
  () => props.node.kind === 'llm.call' && (props.llmCallNav?.previous || props.llmCallNav?.next),
);
const statusMarker = computed(() => statusDescriptor(props.node.status));
const childrenVisible = computed(
  () => hasChildren.value && (expanded.value || (props.forceExpanded && props.node.childrenLoaded)),
);
const prefetchSentinelIndex = computed(() => {
  if (!props.node.hasMoreChildren || !props.node.children.length) {
    return null;
  }
  const remaining = UI_LIMITS.actionTreeChildPrefetchRemaining;
  if (!Number.isInteger(remaining) || remaining < 0) {
    throw new Error('invalid UI_LIMITS.actionTreeChildPrefetchRemaining');
  }
  return Math.max(0, props.node.children.length - remaining - 1);
});

watch(
  () => props.node.id,
  () => {
    expanded.value = false;
  },
);

watch(
  () => [
    props.node.children.length,
    props.node.hasMoreChildren,
    props.node.loadingMore,
    childrenVisible.value,
  ],
  () => {
    refreshPrefetchObserver();
  },
  { flush: 'post' },
);

onBeforeUnmount(() => {
  disconnectPrefetchObserver();
});

function selectNode() {
  emit('select', props.node);
}

function toggleExpanded() {
  if (hasChildren.value) {
    if (!props.node.childrenLoaded && !props.node.loading) {
      emit('expand', props.node);
    }
    expanded.value = !expanded.value;
  }
}

function jumpTo(target) {
  if (target) {
    emit('jump', target);
  }
}

function llmCallNavigation(index) {
  const child = props.node.children[index];
  if (child?.kind !== 'llm.call') {
    return null;
  }
  return {
    previous: siblingLlmCall(index, -1),
    next: siblingLlmCall(index, 1),
  };
}

function siblingLlmCall(fromIndex, step) {
  for (
    let index = fromIndex + step;
    index >= 0 && index < props.node.children.length;
    index += step
  ) {
    const candidate = props.node.children[index];
    if (candidate?.kind === 'llm.call') {
      return candidate;
    }
  }
  return null;
}

async function refreshPrefetchObserver() {
  disconnectPrefetchObserver();
  if (!childrenVisible.value || !props.node.hasMoreChildren || props.node.loadingMore) {
    return;
  }
  await nextTick();
  const sentinel = Array.isArray(prefetchSentinel.value)
    ? prefetchSentinel.value[0]
    : prefetchSentinel.value;
  if (!sentinel) {
    return;
  }
  prefetchObserver = new IntersectionObserver((entries) => {
    if (entries.some((entry) => entry.isIntersecting)) {
      emit('load-more', props.node);
    }
  });
  prefetchObserver.observe(sentinel);
}

function disconnectPrefetchObserver() {
  if (!prefetchObserver) {
    return;
  }
  prefetchObserver.disconnect();
  prefetchObserver = null;
}

function statusDescriptor(status) {
  if (!status) {
    return null;
  }
  const raw = String(status).trim();
  if (!raw) {
    return null;
  }
  const normalized = raw.toLowerCase().replace(/[\s-]+/g, '_');
  if (['success', 'healthy', 'completed', 'complete', 'ok'].includes(normalized)) {
    return { tone: 'success', icon: CheckCircle2, raw };
  }
  if (['error', 'failed', 'failure', 'unhealthy'].includes(normalized)) {
    return { tone: 'error', icon: XCircle, raw };
  }
  if (['in_progress', 'running', 'started', 'pending', 'partial'].includes(normalized)) {
    return { tone: 'progress', icon: Clock3, raw };
  }
  return { tone: 'unknown', icon: CircleHelp, raw };
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

.llm-call-jump-controls {
  position: absolute;
  left: -30px;
  top: 7px;
  z-index: 3;
  display: grid;
  gap: 5px;
  opacity: 0.72;
  transition:
    opacity 140ms ease,
    transform 140ms ease;
}

.action-tree-node.kind-llm-call:hover > .action-tree-branch > .llm-call-jump-controls,
.action-tree-node.kind-llm-call.is-selected > .action-tree-branch > .llm-call-jump-controls {
  opacity: 1;
}

.llm-call-jump-controls button {
  width: 24px;
  height: 20px;
  display: grid;
  place-items: center;
  padding: 0;
  border: 1px solid #f4c56d;
  border-radius: 7px;
  background: #fff8e7;
  color: #92400e;
  box-shadow: 0 4px 10px rgba(146, 64, 14, 0.12);
  cursor: pointer;
  transition:
    background-color 130ms ease,
    border-color 130ms ease,
    box-shadow 130ms ease,
    transform 130ms ease;
}

.llm-call-jump-controls button:hover:not(:disabled) {
  border-color: var(--amber);
  background: #ffefd0;
  box-shadow:
    0 0 0 3px rgba(245, 158, 11, 0.16),
    0 8px 18px rgba(146, 64, 14, 0.18);
  transform: translateX(-2px);
}

.llm-call-jump-controls button:active:not(:disabled) {
  transform: translateX(-1px) translateY(1px);
  box-shadow:
    0 0 0 2px rgba(245, 158, 11, 0.14),
    inset 0 1px 2px rgba(146, 64, 14, 0.2);
}

.llm-call-jump-controls button:focus-visible {
  outline: none;
  box-shadow:
    0 0 0 3px rgba(245, 158, 11, 0.22),
    0 8px 18px rgba(146, 64, 14, 0.18);
}

.llm-call-jump-controls button:disabled {
  cursor: default;
  opacity: 0.35;
  box-shadow: none;
}

.jump-triangle {
  width: 0;
  height: 0;
  display: block;
  border-left: 5px solid transparent;
  border-right: 5px solid transparent;
}

.jump-triangle.up {
  border-bottom: 7px solid currentColor;
}

.jump-triangle.down {
  border-top: 7px solid currentColor;
}

.action-card {
  position: relative;
  isolation: isolate;
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
  outline: none;
  transition:
    background-color 140ms ease,
    border-color 140ms ease,
    box-shadow 140ms ease,
    transform 140ms ease;
}

.action-card:hover,
.action-tree-node.is-selected > .action-tree-branch > .action-card {
  border-color: var(--teal);
}

.action-card:hover {
  transform: translateY(-1px);
  box-shadow:
    0 10px 24px rgba(15, 118, 110, 0.11),
    var(--shadow);
}

.action-card:active {
  transform: translateY(1px);
  box-shadow:
    0 3px 9px rgba(15, 23, 42, 0.16),
    inset 0 1px 0 rgba(255, 255, 255, 0.7);
}

.action-card:focus-visible {
  box-shadow:
    0 0 0 3px rgba(15, 118, 110, 0.22),
    var(--shadow);
}

.action-tree-node.is-selected > .action-tree-branch > .action-card {
  background:
    linear-gradient(180deg, rgba(240, 253, 250, 0.98), rgba(232, 247, 244, 0.98)),
    var(--surface);
  box-shadow:
    0 0 0 2px rgba(15, 118, 110, 0.24),
    0 14px 32px rgba(15, 118, 110, 0.18),
    0 4px 10px rgba(15, 23, 42, 0.12);
  transform: translateY(-2px);
}

.action-tree-node.is-selected > .action-tree-branch > .action-card::after {
  content: "";
  position: absolute;
  inset: 5px;
  z-index: -1;
  border-radius: 6px;
  box-shadow: inset 0 0 0 1px rgba(13, 148, 136, 0.12);
  pointer-events: none;
}

.action-tree-node.is-selected > .action-tree-branch > .action-card:active {
  transform: translateY(0);
  box-shadow:
    0 0 0 2px rgba(15, 118, 110, 0.22),
    0 6px 16px rgba(15, 118, 110, 0.16),
    inset 0 1px 2px rgba(15, 23, 42, 0.12);
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
  background:
    linear-gradient(180deg, rgba(243, 246, 252, 0.98), rgba(232, 239, 250, 0.98)),
    #f3f6fc;
  box-shadow:
    0 0 0 2px rgba(49, 88, 163, 0.24),
    0 14px 32px rgba(49, 88, 163, 0.17),
    0 4px 10px rgba(15, 23, 42, 0.12);
}

.action-tree-node.node-actionGroup > .action-tree-branch > .action-card,
.action-tree-node.visual-action-group > .action-tree-branch > .action-card {
  border-color: #c8d7d5;
  border-left-color: #475569;
  background: #f7f9f9;
}

.action-tree-node.node-actionGroup.is-selected > .action-tree-branch > .action-card,
.action-tree-node.visual-action-group.is-selected > .action-tree-branch > .action-card {
  border-color: #475569;
  background:
    linear-gradient(180deg, rgba(248, 250, 252, 0.98), rgba(238, 242, 247, 0.98)),
    #f7f9f9;
  box-shadow:
    0 0 0 2px rgba(71, 85, 105, 0.2),
    0 14px 30px rgba(71, 85, 105, 0.14),
    0 4px 10px rgba(15, 23, 42, 0.12);
}

.action-tree-node.status-error > .action-tree-branch > .action-card {
  border-color: #fca5a5;
  border-left-color: #dc2626;
  background:
    linear-gradient(180deg, rgba(255, 247, 247, 0.98), rgba(254, 242, 242, 0.98)),
    var(--surface);
  box-shadow:
    0 0 0 1px rgba(220, 38, 38, 0.12),
    var(--shadow);
}

.action-tree-node.status-error > .action-tree-branch > .action-card:hover,
.action-tree-node.status-error.is-selected > .action-tree-branch > .action-card {
  border-color: #dc2626;
}

.action-tree-node.status-error > .action-tree-branch > .action-card:hover {
  box-shadow:
    0 10px 24px rgba(220, 38, 38, 0.14),
    var(--shadow);
}

.action-tree-node.status-error.is-selected > .action-tree-branch > .action-card {
  background:
    linear-gradient(180deg, rgba(255, 241, 242, 0.99), rgba(254, 226, 226, 0.99)),
    var(--surface);
  box-shadow:
    0 0 0 2px rgba(220, 38, 38, 0.24),
    0 14px 32px rgba(220, 38, 38, 0.17),
    0 4px 10px rgba(15, 23, 42, 0.12);
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
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 6px;
  max-width: 118px;
}

.action-status-marker {
  flex: 0 0 auto;
  display: inline-grid;
  place-items: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 1px solid currentColor;
  border-radius: 999px;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.72);
}

.action-status-marker svg {
  flex: 0 0 auto;
}

.action-status-marker.tone-success {
  background: #ecfdf3;
  color: #047857;
  border-color: #86efac;
}

.action-status-marker.tone-error {
  background: #fff1f2;
  color: #be123c;
  border-color: #fb7185;
}

.action-status-marker.tone-progress {
  background: #fffbeb;
  color: #b45309;
  border-color: #facc15;
}

.action-status-marker.tone-unknown {
  background: #f8fafc;
  color: #475569;
  border-color: #cbd5e1;
}

.action-card-toggle {
  flex: 0 0 auto;
  display: inline-grid;
  place-items: center;
  width: 22px;
  height: 22px;
  border: 1px solid #c8d7d5;
  border-radius: 6px;
  background: #f8fbfa;
  color: var(--teal-deep);
  cursor: pointer;
  padding: 0;
  transition:
    background-color 120ms ease,
    border-color 120ms ease,
    box-shadow 120ms ease,
    transform 120ms ease;
}

.action-card-toggle:hover {
  border-color: var(--teal);
  background: #e8f7f4;
  box-shadow: 0 4px 10px rgba(15, 118, 110, 0.14);
}

.action-card-toggle:active {
  transform: translateY(1px);
  box-shadow: inset 0 1px 2px rgba(15, 23, 42, 0.16);
}

.action-card-toggle:focus-visible {
  outline: none;
  box-shadow: 0 0 0 3px rgba(15, 118, 110, 0.22);
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

.action-prefetch-sentinel {
  width: var(--action-node-width);
  height: 1px;
}
</style>
