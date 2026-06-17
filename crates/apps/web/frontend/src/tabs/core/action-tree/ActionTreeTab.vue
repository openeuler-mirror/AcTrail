<template>
  <section class="tab-detail-layout">
    <section class="graph-panel tab-detail-main">
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
          @load-more="loadMoreChildren"
        />
        <div v-else class="action-tree-empty">No action tree root</div>
      </div>
    </section>
    <DetailPanel
      :detail="selectedDetail"
      :trace-id="traceKey"
      :error="detailError"
      @clear="clearDetail"
    />
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

import { readActionDetail, readActionTreeChildren } from '../../../api';
import ActionTreeNode from '../../../components/ActionTreeNode.vue';
import DetailPanel from '../../../components/DetailPanel.vue';
import {
  buildActionTreeChildNodes,
  buildActionTreeRootNode,
  buildVisibleActionTreeModel,
  mergeActionTreeChildren,
} from './model';
import { TREE_NODE_TYPES, UI_LIMITS } from './config';

const props = defineProps({
  traceKey: {
    type: [String, Number],
    default: null,
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
});

const rootNode = ref(null);
const selectedDetailId = ref(null);
const selectedDetail = ref(null);
const detailError = ref('');
let activeDetailLoad = null;

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
    clearDetail();
    rootNode.value = props.actionTree?.rootData
      ? buildActionTreeRootNode({
          traceDetail: props.traceDetail,
          rootData: props.actionTree.rootData,
        })
      : null;
  },
  { immediate: true },
);

async function selectNode(node) {
  const token = Symbol();
  activeDetailLoad = token;
  detailError.value = '';
  selectedDetailId.value = node.detail?.selectionId ?? node.id;
  selectedDetail.value = node.detail ?? null;
  if (node.nodeType !== TREE_NODE_TYPES.action || !node.id) {
    return;
  }
  try {
    const action = await readActionDetail(props.traceKey, node.id);
    if (activeDetailLoad === token && selectedDetailId.value === node.id) {
      selectedDetail.value = fullActionDetail(node.detail, action);
    }
  } catch (err) {
    if (activeDetailLoad === token && selectedDetailId.value === node.id) {
      detailError.value = String(err.message ?? err);
    }
  }
}

function fullActionDetail(currentDetail, action) {
  const pathSetDetail =
    (action.kind === 'file.bulk_read' || action.kind === 'fs.enumerate')
      ? {
          filePathSetActionId: action.id,
          filePathSetPageSize: UI_LIMITS.actionTreeChildPageSize,
        }
      : {};
  return {
    ...currentDetail,
    ...pathSetDetail,
    rows: {
      ...(currentDetail.rows ?? {}),
      evidence: action.evidence?.length ?? 0,
    },
    attributes: action.attributes ?? {},
    evidence: action.evidence ?? [],
    raw: action,
  };
}

function clearDetail() {
  activeDetailLoad = Symbol();
  selectedDetailId.value = null;
  selectedDetail.value = null;
  detailError.value = '';
}

async function loadChildren(node) {
  const target = findNode(rootNode.value, node.id) ?? node;
  if (target.childrenLoaded || target.loading || !target.hasChildren) {
    return;
  }
  await loadChildPage(node, target, 0, false);
}

async function loadMoreChildren(node) {
  const target = findNode(rootNode.value, node.id) ?? node;
  if (
    !target.childrenLoaded ||
    !target.hasMoreChildren ||
    target.loading ||
    target.loadingMore
  ) {
    return;
  }
  await loadChildPage(node, target, target.nextChildOffset, true);
}

async function loadChildPage(visibleNode, target, offset, append) {
  const pageSize = UI_LIMITS.actionTreeChildPageSize;
  if (!Number.isInteger(pageSize) || pageSize < 1) {
    throw new Error('invalid UI_LIMITS.actionTreeChildPageSize');
  }
  try {
    if (append) {
      setLoadingMoreState(visibleNode, target, true);
    } else {
      setLoadingState(visibleNode, target, true);
    }
    const childData = await readActionTreeChildren(props.traceKey, target.id, {
      offset,
      limit: pageSize,
    });
    const children = buildActionTreeChildNodes({
      parentNode: target,
      childData,
      traceDetail: props.traceDetail,
    });
    target.children = append ? mergeActionTreeChildren(target.children, children) : children;
    target.childrenLoaded = true;
    target.totalChildren = childData?.total ?? target.children.length;
    target.nextChildOffset = childData?.next_offset ?? target.children.length;
    target.hasMoreChildren = Boolean(childData?.has_more);
    target.hasChildren = target.totalChildren > 0 || target.children.length > 0;
    syncVisibleNode(visibleNode, target);
  } catch (err) {
    target.error = String(err.message ?? err);
    syncVisibleNode(visibleNode, target);
  } finally {
    if (append) {
      setLoadingMoreState(visibleNode, target, false);
    } else {
      setLoadingState(visibleNode, target, false);
    }
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

function setLoadingMoreState(visibleNode, targetNode, loading) {
  targetNode.loadingMore = loading;
  targetNode.error = loading ? '' : targetNode.error;
  if (visibleNode !== targetNode) {
    visibleNode.loadingMore = targetNode.loadingMore;
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
  visibleNode.totalChildren = targetNode.totalChildren;
  visibleNode.nextChildOffset = targetNode.nextChildOffset;
  visibleNode.hasMoreChildren = targetNode.hasMoreChildren;
  visibleNode.loading = targetNode.loading;
  visibleNode.loadingMore = targetNode.loadingMore;
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
