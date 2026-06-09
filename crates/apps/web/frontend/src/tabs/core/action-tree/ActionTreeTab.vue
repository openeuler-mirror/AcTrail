<template>
  <section class="graph-panel">
    <div class="tree-sticky-header">
      <div class="lane-labels" aria-hidden="true">
        <span v-for="lane in treeModel.lanes" :key="lane" class="lane-label">{{ lane }}</span>
      </div>
      <div v-if="selectedDetail" class="selected-strip">
        <span>{{ selectedDetail.kind }}</span>
        <strong>{{ selectedDetail.title }}</strong>
      </div>
    </div>
    <div class="action-tree-canvas">
      <ActionTreeNode
        v-if="treeModel.root"
        :key="traceKey"
        :node="treeModel.root"
        :force-expanded="treeModel.queryActive"
        :selected-id="selectedDetailId"
        @select="selectNode"
        @expand="loadChildren"
      />
      <div v-else class="action-tree-empty">No action tree root</div>
    </div>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

import { readActionTreeChildren } from '../../../api';
import ActionTreeNode from '../../../components/ActionTreeNode.vue';
import {
  buildActionTreeChildNodes,
  buildActionTreeRootNode,
  buildVisibleActionTreeModel,
} from './model';

const props = defineProps({
  traceKey: {
    type: [String, Number],
    required: true,
  },
  traceDetail: {
    type: Object,
    default: null,
  },
  actionTree: {
    type: Object,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
  selectedDetailId: {
    type: String,
    default: null,
  },
  selectedDetail: {
    type: Object,
    default: null,
  },
});

const emit = defineEmits(['select-detail']);

const rootNode = ref(null);

const treeModel = computed(() =>
  rootNode.value
    ? buildVisibleActionTreeModel({
        root: rootNode.value,
        query: props.query,
      })
    : { lanes: [], root: null, queryActive: false },
);

watch(
  () => [props.traceKey, props.actionTree?.rootData, props.traceDetail],
  () => {
    rootNode.value = props.actionTree?.rootData
      ? buildActionTreeRootNode({
          traceDetail: props.traceDetail,
          rootData: props.actionTree.rootData,
        })
      : null;
  },
  { immediate: true },
);

function selectNode(node) {
  emit('select-detail', node.detail);
}

async function loadChildren(node) {
  const target = findNode(rootNode.value, node.id) ?? node;
  if (target.childrenLoaded || target.loading || !target.hasChildren) {
    return;
  }
  setLoadingState(node, target, true);
  try {
    const childData = await readActionTreeChildren(props.traceKey, target.id);
    target.children = buildActionTreeChildNodes({
      parentNode: target,
      childData,
      traceDetail: props.traceDetail,
    });
    target.childrenLoaded = true;
    target.hasChildren = target.children.length > 0;
    syncVisibleNode(node, target);
  } catch (err) {
    target.error = String(err.message ?? err);
    syncVisibleNode(node, target);
  } finally {
    setLoadingState(node, target, false);
  }
}

function findNode(node, id) {
  if (!node) {
    return null;
  }
  if (node.id === id) {
    return node;
  }
  for (const child of node.children ?? []) {
    const found = findNode(child, id);
    if (found) {
      return found;
    }
  }
  return null;
}

function setLoadingState(visibleNode, targetNode, loading) {
  targetNode.loading = loading;
  targetNode.error = loading ? '' : targetNode.error;
  if (visibleNode !== targetNode) {
    visibleNode.loading = targetNode.loading;
    visibleNode.error = targetNode.error;
  }
}

function syncVisibleNode(visibleNode, targetNode) {
  if (visibleNode === targetNode) {
    return;
  }
  visibleNode.children = targetNode.children;
  visibleNode.childrenLoaded = targetNode.childrenLoaded;
  visibleNode.hasChildren = targetNode.hasChildren;
  visibleNode.loading = targetNode.loading;
  visibleNode.error = targetNode.error;
}
</script>

<style scoped>
.graph-panel {
  position: relative;
  min-height: 0;
  height: 100%;
  overflow: auto;
  background:
    linear-gradient(90deg, rgba(15, 118, 110, 0.06) 1px, transparent 1px),
    var(--bg);
  background-size: var(--action-lane-width) 100%;
}

.tree-sticky-header {
  position: sticky;
  top: 0;
  z-index: 6;
  width: max-content;
  min-width: 100%;
  background: linear-gradient(180deg, rgba(244, 247, 247, 0.98), rgba(244, 247, 247, 0.76));
  backdrop-filter: blur(6px);
}

.lane-labels {
  display: flex;
  gap: var(--action-lane-gap);
  width: max-content;
  padding: 18px 36px 10px;
}

.lane-label {
  width: var(--action-node-width);
  color: var(--muted);
  font-size: 12px;
  font-weight: 800;
  text-transform: uppercase;
}

.action-tree-canvas {
  width: max-content;
  min-width: 100%;
  padding: 34px 36px 32px;
}

.action-tree-empty {
  width: var(--action-node-width);
  min-height: var(--action-node-min-height);
  display: grid;
  place-items: center;
  border: 1px dashed #bdd7d2;
  border-radius: 8px;
  background: #fbfcfc;
  color: var(--muted);
  font-size: 12px;
  font-weight: 700;
}

.selected-strip {
  max-width: min(760px, calc(100vw - var(--trace-rail-width) - var(--detail-panel-width)));
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 0 36px 10px;
  padding: 8px 10px;
  border: 1px solid #bdd7d2;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.88);
  box-shadow: var(--shadow);
}

.selected-strip span {
  flex: 0 0 auto;
  color: var(--muted);
  font-size: 11px;
  font-weight: 800;
  text-transform: uppercase;
}

.selected-strip strong {
  min-width: 0;
  overflow: hidden;
  color: var(--teal-deep);
  font-size: 13px;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
