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
        :aria-label="t('actionNode.jumpControls')"
      >
        <button
          type="button"
          :disabled="!llmCallNav?.previous"
          :aria-label="t('actionNode.previousSibling')"
          :title="t('actionNode.previous')"
          @click.stop="jumpTo(llmCallNav.previous)"
        >
          <span class="jump-triangle up" aria-hidden="true"></span>
        </button>
        <button
          type="button"
          :disabled="!llmCallNav?.next"
          :aria-label="t('actionNode.nextSibling')"
          :title="t('actionNode.next')"
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
              :aria-label="childrenVisible ? t('actionNode.collapseChildren') : t('actionNode.expandChildren')"
              :title="childrenVisible ? t('actionNode.collapse') : t('actionNode.expand')"
              @click.stop="toggleExpanded"
            >
              <ChevronDown v-if="childrenVisible" :size="15" />
              <ChevronRight v-else :size="15" />
            </button>
          </span>
        </span>
        <span v-if="cardMetaItems.length" class="action-card-meta">
          <span
            v-for="item in cardMetaItems"
            :key="`${item.kind}:${item.label}`"
            class="action-meta-chip"
            :class="`meta-${item.kind}`"
            :title="item.label"
          >
            {{ item.label }}
          </span>
        </span>
      </div>

      <div v-if="childrenVisible" class="action-child-lane">
        <template v-for="(child, index) in node.children" :key="child.id">
          <ActionTreeNode
            :node="child"
            :depth="depth + 1"
            :force-expanded="forceExpanded"
            :selected-id="selectedId"
            :expanded-ids="expandedIds"
            :llm-call-nav="llmCallNavigation(index)"
            @select="$emit('select', $event)"
            @expand="$emit('expand', $event)"
            @set-expanded="$emit('set-expanded', $event)"
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

import { LLM_NAVIGATION_KINDS, UI_LIMITS } from '../tabs/core/action-tree/config';
import { useModuleLocale } from '../locale';
import DurationBadge from './DurationBadge.vue';
import strings from './locale';

defineOptions({ name: 'ActionTreeNode' });

const { t } = useModuleLocale(strings);

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
  expandedIds: {
    type: Object,
    default: () => new Set(),
  },
});

const emit = defineEmits(['select', 'expand', 'set-expanded', 'load-more', 'jump']);
const expanded = ref(false);
const prefetchSentinel = ref(null);
let prefetchObserver = null;
const hasChildren = computed(() => props.node.hasChildren || props.node.children.length > 0);
const showLlmCallJump = computed(
  () => LLM_NAVIGATION_KINDS.includes(props.node.kind) && (props.llmCallNav?.previous || props.llmCallNav?.next),
);
const statusMarker = computed(() => statusDescriptor(props.node.status));
const cardMetaItems = computed(() => {
  if (Array.isArray(props.node.metaItems) && props.node.metaItems.length) {
    return props.node.metaItems.filter((item) => item?.label);
  }
  const fallback = String(props.node.meta ?? '').trim();
  return fallback ? [{ kind: 'summary', label: fallback }] : [];
});
const childrenVisible = computed(
  () =>
    hasChildren.value &&
    (expanded.value ||
      props.expandedIds?.has?.(props.node.id) ||
      (props.forceExpanded && props.node.childrenLoaded)),
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
    const nextExpanded = !childrenVisible.value;
    if (!props.node.childrenLoaded && !props.node.loading) {
      emit('expand', props.node);
    }
    expanded.value = nextExpanded;
    emit('set-expanded', { node: props.node, expanded: nextExpanded });
  }
}

function jumpTo(target) {
  if (target) {
    emit('jump', target);
  }
}

function llmCallNavigation(index) {
  const child = props.node.children[index];
  if (!LLM_NAVIGATION_KINDS.includes(child?.kind)) {
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
    if (LLM_NAVIGATION_KINDS.includes(candidate?.kind)) {
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

<style scoped src="./ActionTreeNode/styles.css"></style>
