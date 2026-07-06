<template>
  <section class="tab-detail-layout action-tree-layout" :class="{ 'detail-open': detailPanelVisible }">
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
      <div ref="actionTreeCanvas" class="action-tree-canvas">
        <svg
          v-if="messagePairArcs.length"
          class="message-pair-overlay"
          :width="arcOverlaySize.width"
          :height="arcOverlaySize.height"
          :viewBox="`0 0 ${arcOverlaySize.width} ${arcOverlaySize.height}`"
          aria-hidden="true"
        >
          <path
            v-for="arc in messagePairArcs"
            :key="arc.id"
            :class="arc.className"
            :d="arc.path"
          />
        </svg>
        <ActionTreeNode
          v-if="treeModel.root"
          :key="traceKey"
          :node="treeModel.root"
          :force-expanded="treeModel.queryActive"
          :selected-id="selectedDetailId"
          :expanded-ids="expandedNodeIds"
          @select="selectNode"
          @expand="loadChildren"
          @set-expanded="setNodeExpanded"
          @load-more="loadMoreChildren"
          @jump="jumpToNode"
        />
        <div v-else class="action-tree-empty">No action tree root</div>
      </div>
      <div class="action-tree-nav-panel" aria-label="Action tree LLM navigation">
        <span class="action-tree-nav-label">LLM</span>
        <button
          class="action-tree-nav-button"
          type="button"
          :disabled="llmNavigationBusy || !treeModel.root"
          title="Jump to first LLM call"
          @click="jumpToFirstLlm"
        >
          <SkipForward v-if="!llmNavigationBusy" :size="15" aria-hidden="true" />
          <Loader2 v-else class="spin-icon" :size="15" aria-hidden="true" />
          <span>First LLM</span>
        </button>
        <button
          class="action-tree-nav-button"
          type="button"
          :disabled="llmNavigationBusy || !treeModel.root"
          title="Jump to next LLM call"
          @click="jumpToNextLlm"
        >
          <StepForward :size="15" aria-hidden="true" />
          <span>Next LLM</span>
        </button>
        <span v-if="llmNavigationError" class="action-tree-nav-error">{{ llmNavigationError }}</span>
      </div>
    </section>
    <DetailPanel
      :detail="selectedDetail"
      :trace-id="traceKey"
      :error="detailError"
      hide-when-empty
      @clear="clearDetail"
    />
  </section>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { Loader2, SkipForward, StepForward } from '@lucide/vue';

import { readActionDetail, readActionTreeChildren } from '../../../api';
import ActionTreeNode from '../../../components/ActionTreeNode.vue';
import DetailPanel from '../../../components/DetailPanel.vue';
import {
  buildActionTreeChildNodes,
  buildActionTreeRootNode,
  buildVisibleActionTreeModel,
  mergeActionTreeChildren,
} from './model';
import { buildMessagePairArcOverlay } from './httpExchangeArcs';
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
const actionTreeCanvas = ref(null);
const selectedDetailId = ref(null);
const selectedDetail = ref(null);
const detailError = ref('');
const messagePairArcs = ref([]);
const expandedNodeIds = ref(new Set());
const llmNavigationBusy = ref(false);
const llmNavigationError = ref('');
const arcOverlaySize = ref({ width: 0, height: 0 });
let activeDetailLoad = null;
let arcRefreshFrame = 0;
let canvasMutationObserver = null;
let canvasResizeObserver = null;

const treeModel = computed(() =>
  rootNode.value
    ? buildVisibleActionTreeModel({
        root: rootNode.value,
        query: props.query,
      })
    : { lanes: [], root: null, queryActive: false },
);
const detailPanelVisible = computed(() => Boolean(selectedDetail.value || detailError.value));

watch(
  () => [props.traceKey, props.actionTree?.rootData, props.traceDetail],
  () => {
    clearDetail();
    expandedNodeIds.value = new Set();
    llmNavigationError.value = '';
    rootNode.value = props.actionTree?.rootData
      ? buildActionTreeRootNode({
          traceDetail: props.traceDetail,
          rootData: props.actionTree.rootData,
        })
      : null;
  },
  { immediate: true },
);

watch(
  () => [props.traceKey, props.query, treeModel.value.root],
  () => {
    scheduleMessagePairArcRefresh();
  },
  { flush: 'post' },
);

onMounted(() => {
  connectCanvasObservers();
  scheduleMessagePairArcRefresh();
});

onBeforeUnmount(() => {
  disconnectCanvasObservers();
  cancelMessagePairArcRefresh();
});

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

async function jumpToNode(node) {
  const selectionId = node.detail?.selectionId ?? node.id;
  await selectNode(node);
  await nextTick();
  scrollNodeIntoView(selectionId);
}

async function jumpToFirstLlm() {
  await runLlmNavigation(async () => {
    const path = await findFirstLlmPath(rootNode.value);
    if (!path) {
      llmNavigationError.value = 'No LLM call';
      return;
    }
    await activateLlmPath(path);
  });
}

async function jumpToNextLlm() {
  await runLlmNavigation(async () => {
    const path = selectedDetailId.value
      ? await findNextLlmPath(rootNode.value, selectedDetailId.value)
      : await findFirstLlmPath(rootNode.value);
    if (!path) {
      llmNavigationError.value = 'No next LLM call';
      return;
    }
    await activateLlmPath(path);
  });
}

async function runLlmNavigation(callback) {
  if (!rootNode.value || llmNavigationBusy.value) {
    return;
  }
  llmNavigationBusy.value = true;
  llmNavigationError.value = '';
  try {
    await callback();
  } catch (err) {
    llmNavigationError.value = String(err.message ?? err);
  } finally {
    llmNavigationBusy.value = false;
  }
}

async function activateLlmPath(path) {
  const target = path[path.length - 1];
  if (!target) {
    throw new Error('LLM navigation returned an empty path');
  }
  expandAncestorPath(path.slice(0, -1));
  await nextTick();
  await selectNode(target);
  await nextTick();
  scrollNodeIntoView(target.detail?.selectionId ?? target.id);
}

function expandAncestorPath(path) {
  const next = new Set(expandedNodeIds.value);
  for (const node of path) {
    if (node?.id && (node.hasChildren || node.children?.length)) {
      next.add(node.id);
    }
  }
  expandedNodeIds.value = next;
}

function setNodeExpanded({ node, expanded }) {
  if (!node?.id) {
    return;
  }
  const next = new Set(expandedNodeIds.value);
  if (expanded) {
    next.add(node.id);
  } else {
    next.delete(node.id);
  }
  expandedNodeIds.value = next;
}

async function findFirstLlmPath(node, ancestors = []) {
  if (!node) {
    return null;
  }
  if (isLlmCallNode(node)) {
    return [...ancestors, node];
  }
  await ensureNodeChildrenLoaded(node);
  let index = 0;
  while (true) {
    while (index < node.children.length) {
      const found = await findFirstLlmPath(node.children[index], [...ancestors, node]);
      if (found) {
        return found;
      }
      index += 1;
    }
    if (!node.hasMoreChildren) {
      return null;
    }
    await loadMoreNodeChildren(node);
  }
}

async function findNextLlmPath(node, selectedId) {
  let sawSelected = !selectedId;
  let afterSelected = !selectedId;

  async function visit(candidate, ancestors = []) {
    if (!candidate) {
      return null;
    }
    if (selectedId && nodeMatchesSelection(candidate, selectedId)) {
      sawSelected = true;
      afterSelected = true;
    } else if (afterSelected && isLlmCallNode(candidate)) {
      return [...ancestors, candidate];
    }

    await ensureNodeChildrenLoaded(candidate);
    let index = 0;
    while (true) {
      while (index < candidate.children.length) {
        const found = await visit(candidate.children[index], [...ancestors, candidate]);
        if (found) {
          return found;
        }
        index += 1;
      }
      if (!candidate.hasMoreChildren) {
        return null;
      }
      await loadMoreNodeChildren(candidate);
    }
  }

  const found = await visit(node);
  if (found || sawSelected) {
    return found;
  }
  return findFirstLlmPath(node);
}

async function ensureNodeChildrenLoaded(node) {
  if (!node || node.childrenLoaded || (!node.hasChildren && !node.children.length)) {
    return;
  }
  await loadChildPage(node, node, 0, false, { throwOnError: true });
}

async function loadMoreNodeChildren(node) {
  if (!node?.hasMoreChildren || node.loadingMore) {
    return;
  }
  const previousOffset = node.nextChildOffset;
  const previousCount = node.children.length;
  await loadChildPage(node, node, node.nextChildOffset, true, { throwOnError: true });
  if (
    node.hasMoreChildren &&
    node.nextChildOffset === previousOffset &&
    node.children.length === previousCount
  ) {
    throw new Error('Action tree children pagination made no progress');
  }
}

function isLlmCallNode(node) {
  return node?.kind === 'llm.call';
}

function nodeMatchesSelection(node, selectionId) {
  return node?.id === selectionId || node?.detail?.selectionId === selectionId;
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

async function loadChildPage(visibleNode, target, offset, append, { throwOnError = false } = {}) {
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
    scheduleMessagePairArcRefresh();
  } catch (err) {
    target.error = String(err.message ?? err);
    syncVisibleNode(visibleNode, target);
    scheduleMessagePairArcRefresh();
    if (throwOnError) {
      throw err;
    }
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

function connectCanvasObservers() {
  const canvas = actionTreeCanvas.value;
  if (!canvas) {
    return;
  }
  canvasMutationObserver = new MutationObserver(() => {
    scheduleMessagePairArcRefresh();
  });
  canvasMutationObserver.observe(canvas, {
    childList: true,
    subtree: true,
  });
  if (typeof ResizeObserver !== 'undefined') {
    canvasResizeObserver = new ResizeObserver(() => {
      scheduleMessagePairArcRefresh();
    });
    canvasResizeObserver.observe(canvas);
  }
}

function disconnectCanvasObservers() {
  canvasMutationObserver?.disconnect();
  canvasMutationObserver = null;
  canvasResizeObserver?.disconnect();
  canvasResizeObserver = null;
}

function scheduleMessagePairArcRefresh() {
  cancelMessagePairArcRefresh();
  arcRefreshFrame = window.requestAnimationFrame(async () => {
    arcRefreshFrame = 0;
    await nextTick();
    refreshMessagePairArcs();
  });
}

function cancelMessagePairArcRefresh() {
  if (!arcRefreshFrame) {
    return;
  }
  window.cancelAnimationFrame(arcRefreshFrame);
  arcRefreshFrame = 0;
}

function refreshMessagePairArcs() {
  const canvas = actionTreeCanvas.value;
  if (!canvas || !treeModel.value.root) {
    messagePairArcs.value = [];
    arcOverlaySize.value = { width: 0, height: 0 };
    return;
  }
  const overlay = buildMessagePairArcOverlay(treeModel.value.root, canvas);
  messagePairArcs.value = overlay.arcs;
  arcOverlaySize.value = overlay.size;
}

function scrollNodeIntoView(nodeId) {
  const canvas = actionTreeCanvas.value;
  if (!canvas || !nodeId) {
    return;
  }
  const target = Array.from(canvas.querySelectorAll('[data-action-node-id]')).find(
    (element) => element.dataset.actionNodeId === nodeId,
  );
  target?.scrollIntoView({
    block: 'center',
    inline: 'center',
    behavior: 'smooth',
  });
}
</script>

<style scoped>
.action-tree-layout {
  grid-template-columns: minmax(0, 1fr);
}

.action-tree-layout.detail-open {
  grid-template-columns: minmax(0, 1fr) var(--detail-panel-width);
}

.graph-panel {
  position: relative;
  min-height: 0;
  height: 100%;
  overflow: auto;
  background:
    linear-gradient(90deg, var(--trace-action-tree-grid) 1px, transparent 1px),
    var(--trace-action-tree-bg);
  background-size: var(--action-lane-width) 100%;
}

.tree-sticky-header {
  position: sticky;
  top: 0;
  z-index: 6;
  width: max-content;
  min-width: 100%;
  background: var(--trace-action-tree-header-bg);
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
  position: relative;
  width: max-content;
  min-width: 100%;
  padding: 34px 36px 132px;
}

.message-pair-overlay {
  position: absolute;
  inset: 0 auto auto 0;
  z-index: 0;
  overflow: visible;
  pointer-events: none;
}

.action-tree-canvas > .action-tree-node {
  position: relative;
  z-index: 1;
}

.http-exchange-arc {
  fill: none;
  stroke: var(--trace-action-tree-arc);
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-dasharray: 6 6;
  vector-effect: non-scaling-stroke;
}

.mcp-exchange-arc {
  fill: none;
  stroke: rgba(124, 58, 237, 0.5);
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-dasharray: 6 6;
  vector-effect: non-scaling-stroke;
}

.action-tree-empty {
  position: relative;
  z-index: 1;
  width: var(--action-node-width);
  min-height: var(--action-node-min-height);
  display: grid;
  place-items: center;
  border: 1px dashed var(--trace-action-empty-border);
  border-radius: 8px;
  background: var(--trace-action-empty-bg);
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
  border: 1px solid var(--trace-action-selected-strip-border);
  border-radius: 8px;
  background: var(--trace-action-selected-strip-bg);
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

.action-tree-nav-panel {
  position: sticky;
  left: 16px;
  bottom: 16px;
  z-index: 9;
  width: 138px;
  max-width: calc(100vw - 44px);
  display: grid;
  align-items: stretch;
  gap: 6px;
  margin: -120px 0 16px 16px;
  padding: 8px;
  border: 1px solid var(--trace-action-selected-strip-border);
  border-radius: 10px;
  background: var(--trace-action-selected-strip-bg);
  color: var(--text);
  box-shadow: var(--shadow);
  backdrop-filter: var(--stats-control-filter, blur(10px));
}

.action-tree-nav-label {
  padding: 0 2px 2px;
  color: var(--muted);
  font-size: 11px;
  font-weight: 850;
  text-transform: uppercase;
}

.action-tree-nav-button {
  height: 30px;
  display: inline-flex;
  align-items: center;
  justify-content: flex-start;
  gap: 6px;
  padding: 0 10px;
  border: 1px solid var(--trace-action-jump-border);
  border-radius: 8px;
  background: var(--trace-action-jump-bg);
  color: var(--trace-action-jump-text);
  font-size: 12px;
  font-weight: 800;
  cursor: pointer;
  transition:
    background-color 130ms ease,
    border-color 130ms ease,
    box-shadow 130ms ease,
    transform 130ms ease;
}

.action-tree-nav-button:hover:not(:disabled) {
  border-color: var(--teal);
  background: var(--trace-action-jump-hover-bg);
  box-shadow: var(--trace-action-jump-hover-shadow);
  transform: translateY(-1px);
}

.action-tree-nav-button:active:not(:disabled) {
  transform: translateY(1px);
  box-shadow: var(--trace-action-jump-active-shadow);
}

.action-tree-nav-button:focus-visible {
  outline: none;
  box-shadow: var(--trace-action-jump-focus-shadow);
}

.action-tree-nav-button:disabled {
  cursor: default;
  opacity: 0.52;
  box-shadow: none;
}

.action-tree-nav-error {
  max-width: 100%;
  overflow: hidden;
  color: var(--trace-error-text);
  font-size: 12px;
  font-weight: 750;
  text-overflow: ellipsis;
  white-space: normal;
}

.spin-icon {
  animation: action-tree-spin 900ms linear infinite;
}

@keyframes action-tree-spin {
  to {
    transform: rotate(360deg);
  }
}

@media (max-width: 1100px) {
  .action-tree-layout.detail-open {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
