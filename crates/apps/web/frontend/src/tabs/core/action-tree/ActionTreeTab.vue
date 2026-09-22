<template>
  <section class="tab-detail-layout action-tree-layout" :class="{ 'detail-open': detailPanelVisible }">
    <section class="graph-panel tab-detail-main">
      <div class="tree-sticky-header">
        <div class="lane-labels" aria-hidden="true">
          <span v-for="lane in treeModel.lanes" :key="lane" class="lane-label">{{ lane }}</span>
        </div>
        <div v-if="selectedDetail" class="selected-strip">
          <span v-if="selectedDetail.kind !== selectedDetail.title">{{ selectedDetail.kind }}</span>
          <strong>{{ selectedDetail.title }}</strong>
        </div>
      </div>
      <div ref="actionTreeCanvas" class="action-tree-canvas">
        <svg
          v-if="httpExchangeArcs.length"
          class="http-exchange-overlay"
          :width="arcOverlaySize.width"
          :height="arcOverlaySize.height"
          :viewBox="`0 0 ${arcOverlaySize.width} ${arcOverlaySize.height}`"
          aria-hidden="true"
        >
          <path
            v-for="arc in httpExchangeArcs"
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
      progressive-details
      @clear="clearDetail"
    />
  </section>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { Loader2, SkipForward, StepForward } from '@lucide/vue';

import { readActionDetail, readActionTreeChildren, readActionTreeLlmNav } from '../../../api';
import ActionTreeNode from '../../../components/ActionTreeNode.vue';
import DetailPanel from '../../../components/DetailPanel.vue';
import {
  buildActionTreeChildNodes,
  buildFullActionDetail,
  buildActionTreeRootNode,
  buildVisibleActionTreeModel,
  mergeActionTreeChildren,
} from './model';
import { buildHttpExchangeArcOverlay } from './httpExchangeArcs';
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
const expandedNodeIds = ref(new Set());
const llmNavigationBusy = ref(false);
const llmNavigationError = ref('');
const httpExchangeArcs = ref([]);
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
    scheduleHttpExchangeArcRefresh();
  },
  { flush: 'post' },
);

onMounted(() => {
  connectCanvasObservers();
  scheduleHttpExchangeArcRefresh();
});

onBeforeUnmount(() => {
  disconnectCanvasObservers();
  cancelHttpExchangeArcRefresh();
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
      selectedDetail.value = buildFullActionDetail(node.detail, action);
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
    const path = await findLlmPathViaServer('first');
    if (path === undefined) {
      const fallback = await findFirstLlmPath(rootNode.value);
      if (!fallback) {
        llmNavigationError.value = 'No LLM call';
        return;
      }
      await activateLlmPath(fallback);
      return;
    }
    if (!path) {
      llmNavigationError.value = 'No LLM call';
      return;
    }
    await activateServerLlmPath(path);
  });
}

async function jumpToNextLlm() {
  await runLlmNavigation(async () => {
    const afterId = selectedDetailId.value ?? '';
    const path = await findLlmPathViaServer('next', afterId);
    if (path === undefined) {
      const fallback = afterId
        ? await findNextLlmPath(rootNode.value, afterId)
        : await findFirstLlmPath(rootNode.value);
      if (!fallback) {
        llmNavigationError.value = 'No next LLM call';
        return;
      }
      await activateLlmPath(fallback);
      return;
    }
    if (!path) {
      llmNavigationError.value = 'No next LLM call';
      return;
    }
    await activateServerLlmPath(path);
  });
}

async function findLlmPathViaServer(mode, afterId = '') {
  try {
    const data = await readActionTreeLlmNav(props.traceKey, {
      mode,
      afterId: afterId || undefined,
    });
    if (!data?.found || !Array.isArray(data.path) || !data.path.length) {
      return null;
    }
    return data.path;
  } catch {
    return undefined;
  }
}

async function activateServerLlmPath(entries) {
  const pageSize = UI_LIMITS.actionTreeChildPageSize;
  if (!Number.isInteger(pageSize) || pageSize < 1) {
    throw new Error('invalid UI_LIMITS.actionTreeChildPageSize');
  }
  let parentNode = rootNode.value;
  let targetNode = null;
  const nextExpanded = new Set(expandedNodeIds.value);
  if (parentNode?.id) {
    nextExpanded.add(parentNode.id);
  }
  for (const entry of entries) {
    if (!parentNode) {
      throw new Error('LLM navigation path node not found');
    }
    if (parentNode.id !== entry.parent_action_id) {
      const found = findNode(rootNode.value, entry.parent_action_id);
      if (!found) {
        throw new Error(`LLM navigation parent ${entry.parent_action_id} not found`);
      }
      parentNode = found;
    }
    // Fetch a window centered on the backend offset: the target is guaranteed
    // to be a member of this window even if the UI re-sorts/regroups the page
    // differently from the backend.
    const windowStart = Math.max(0, entry.offset - Math.floor(pageSize / 2));
    let childData = await readActionTreeChildren(props.traceKey, parentNode.id, {
      offset: windowStart,
      limit: pageSize,
    });
    let children = buildActionTreeChildNodes({
      parentNode,
      childData,
      traceDetail: props.traceDetail,
    });
    if (!locateActionNode(children, entry.action_id)) {
      // Defensive fallback: fetch the exact backend window starting at the
      // target so ordering differences cannot hide it.
      childData = await readActionTreeChildren(props.traceKey, parentNode.id, {
        offset: entry.offset,
        limit: pageSize,
      });
      children = buildActionTreeChildNodes({
        parentNode,
        childData,
        traceDetail: props.traceDetail,
      });
    }
    parentNode.children = children;
    parentNode.childrenLoaded = true;
    parentNode.totalChildren = childData?.total ?? children.length;
    parentNode.nextChildOffset = childData?.next_offset ?? children.length;
    parentNode.hasMoreChildren = Boolean(childData?.has_more);
    parentNode.hasChildren = parentNode.totalChildren > 0 || children.length > 0;
    if (parentNode !== rootNode.value && parentNode.id) {
      nextExpanded.add(parentNode.id);
    }
    const located = locateActionNode(children, entry.action_id);
    if (!located) {
      throw new Error(`LLM navigation action ${entry.action_id} not found under ${parentNode.id}`);
    }
    for (const container of located.containers) {
      if (container.id && container !== parentNode) {
        nextExpanded.add(container.id);
      }
    }
    targetNode = located.node;
    parentNode = located.node;
  }
  if (!targetNode) {
    throw new Error('LLM navigation returned an empty path');
  }
  expandedNodeIds.value = nextExpanded;
  await nextTick();
  await selectNode(targetNode);
  await nextTick();
  scrollNodeIntoView(targetNode.detail?.selectionId ?? targetNode.id);
}

function locateActionNode(nodes, id, containers = []) {
  for (const node of nodes ?? []) {
    if (node.id === id) {
      return { node, containers };
    }
    if (node.children?.length) {
      const found = locateActionNode(node.children, id, [...containers, node]);
      if (found) {
        return found;
      }
    }
  }
  return null;
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
    scheduleHttpExchangeArcRefresh();
  } catch (err) {
    target.error = String(err.message ?? err);
    syncVisibleNode(visibleNode, target);
    scheduleHttpExchangeArcRefresh();
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
    scheduleHttpExchangeArcRefresh();
  });
  canvasMutationObserver.observe(canvas, {
    childList: true,
    subtree: true,
  });
  if (typeof ResizeObserver !== 'undefined') {
    canvasResizeObserver = new ResizeObserver(() => {
      scheduleHttpExchangeArcRefresh();
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

function scheduleHttpExchangeArcRefresh() {
  cancelHttpExchangeArcRefresh();
  arcRefreshFrame = window.requestAnimationFrame(async () => {
    arcRefreshFrame = 0;
    await nextTick();
    refreshHttpExchangeArcs();
  });
}

function cancelHttpExchangeArcRefresh() {
  if (!arcRefreshFrame) {
    return;
  }
  window.cancelAnimationFrame(arcRefreshFrame);
  arcRefreshFrame = 0;
}

function refreshHttpExchangeArcs() {
  const canvas = actionTreeCanvas.value;
  if (!canvas || !treeModel.value.root) {
    httpExchangeArcs.value = [];
    arcOverlaySize.value = { width: 0, height: 0 };
    return;
  }
  const overlay = buildHttpExchangeArcOverlay(treeModel.value.root, canvas);
  httpExchangeArcs.value = overlay.arcs;
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

<style scoped src="./ActionTreeTab.css"></style>
