<template>
  <section class="trajectory-layout">
    <aside class="trajectory-summary">
      <h2>LLM Trajectory</h2>
      <p class="summary-note">Strict-prefix relationships within this trace.</p>
      <dl>
        <template v-for="[label, value] in summaryRows" :key="label">
          <dt>{{ label }}</dt>
          <dd>{{ value }}</dd>
        </template>
      </dl>
      <div v-if="layout.trajectories.length" class="trajectory-key">
        <h3>Contexts</h3>
        <div
          v-for="trajectory in layout.trajectories"
          :key="trajectory.id"
          class="trajectory-key-row"
          :title="trajectory.id"
        >
          <i :style="{ backgroundColor: trajectory.color }"></i>
          <strong>{{ trajectory.label }}</strong>
          <span>{{ trajectory.model }}</span>
        </div>
      </div>
      <div class="trajectory-legend">
        <span><i class="legend-line solid"></i>Strict prefix</span>
        <span><i class="legend-line dashed"></i>Content related (future)</span>
      </div>
      <p v-if="graph?.partial" class="capability-warning">
        This graph is partial because some trajectory data was unavailable.
      </p>
      <p v-else-if="!graph?.capabilities?.related_edges" class="summary-note">
        Content-related dashed edges and compaction inference are not enabled yet.
      </p>
    </aside>

    <FullscreenSurface
      class="trajectory-viewport"
      label="LLM trajectory graph"
      :aside-open="Boolean(selectedDetail || detailError)"
    >
      <div v-if="!layout.nodes.length" class="trajectory-empty">
        No LLM trajectory data for this trace.
      </div>
      <svg
        v-else
        class="trajectory-canvas"
        :width="layout.width"
        :height="layout.height"
        :viewBox="`0 0 ${layout.width} ${layout.height}`"
        role="img"
        aria-label="LLM trajectory graph"
      >
        <defs>
          <marker id="trajectory-arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="5" markerHeight="5" orient="auto-start-reverse">
            <path d="M 0 0 L 10 5 L 0 10 z" fill="context-stroke" />
          </marker>
        </defs>
        <text class="time-label" x="18" y="26">time ↓</text>
        <template
          v-for="trajectory in layout.trajectories"
          :key="`reuse:${trajectory.id}`"
        >
          <g v-if="trajectory.reusedLane" class="lane-reuse-marker">
            <path
              :d="`M ${trajectory.x - 9} ${trajectory.reuseY + 4} l 6 -8 M ${trajectory.x + 2} ${trajectory.reuseY + 4} l 6 -8`"
            />
            <text :x="trajectory.x + 18" :y="trajectory.reuseY + 4">
              New context {{ trajectory.label }} · lane reused
            </text>
          </g>
        </template>
        <path
          v-for="edge in layout.edges"
          :key="`${edge.kind}:${edge.source}:${edge.target}`"
          class="trajectory-edge"
          :class="`edge-${edge.kind}`"
          :d="edge.path"
          :stroke="edge.color"
          marker-end="url(#trajectory-arrow)"
        />
        <g
          v-for="node in layout.nodes"
          :key="node.id"
          class="trajectory-node"
          :class="{ selected: selectedNodeId === node.id }"
          role="button"
          tabindex="0"
          @click="selectNode(node)"
          @keydown.enter.prevent="selectNode(node)"
          @keydown.space.prevent="selectNode(node)"
        >
          <circle
            v-if="node.compaction_boundary"
            class="compaction-ring"
            :cx="node.x"
            :cy="node.y"
            r="17"
          />
          <circle
            class="node-dot"
            :cx="node.x"
            :cy="node.y"
            r="11"
            :fill="node.color"
          />
          <text
            class="node-label node-label-title"
            :x="node.x + 22"
            :y="node.y - 3"
            :fill="node.color"
          >{{ node.label.title }}</text>
          <text
            class="node-label node-label-metadata"
            :x="node.x + 22"
            :y="node.y + 17"
          >{{ node.label.metadata }}</text>
          <title>{{ node.id }} · {{ formatTime(node.start_time) }} · {{ node.transition }}</title>
        </g>
      </svg>
      <template #aside>
        <DetailPanel
          :detail="selectedDetail"
          :trace-id="traceKey"
          :error="detailError"
          hide-when-empty
          progressive-details
          @clear="clearDetail"
        />
      </template>
    </FullscreenSurface>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

import { readActionDetail } from '../../../api';
import DetailPanel from '../../../components/DetailPanel.vue';
import FullscreenSurface from '../../../components/FullscreenSurface.vue';
import { buildTrajectoryLayout } from './model';

const props = defineProps({
  traceKey: {
    type: [String, Number],
    default: null,
  },
  trajectoryGraph: {
    type: Object,
    default: null,
  },
});

const selectedNodeId = ref(null);
const selectedDetail = ref(null);
const detailError = ref('');
let activeDetailLoad = null;

const graph = computed(() => props.trajectoryGraph ?? emptyGraph());
const layout = computed(() => buildTrajectoryLayout(graph.value));
const summaryRows = computed(() => {
  const stats = graph.value.stats ?? {};
  return [
    ['Requests', stats.node_count ?? 0],
    ['Trajectories', stats.trajectory_count ?? 0],
    ['Append edges', stats.append_count ?? 0],
    ['Fork edges', stats.fork_count ?? 0],
    ['Duplicate roots', stats.duplicate_count ?? 0],
    ['Strongly linked', formatRatio(stats.strongly_linked_node_ratio)],
    ['Duplicate ratio', formatRatio(stats.duplicate_node_ratio)],
  ];
});

watch(
  () => props.traceKey,
  () => clearDetail(),
);

async function selectNode(node) {
  const token = Symbol();
  activeDetailLoad = token;
  selectedNodeId.value = node.id;
  selectedDetail.value = null;
  detailError.value = '';
  try {
    const action = await readActionDetail(props.traceKey, node.id);
    if (activeDetailLoad !== token || selectedNodeId.value !== node.id) {
      return;
    }
    selectedDetail.value = {
      kind: action.kind,
      title: action.title,
      rows: {
        Time: formatTime(node.start_time),
        Trajectory: node.trajectory_id,
        Position: node.trajectory_position,
        Transition: node.transition,
        'Start reason': node.start_reason,
      },
      trajectoryContext: {
        label: node.trajectory_label,
        toolResultCount: node.tool_result_count,
        toolResultDelta: node.tool_result_delta,
      },
      attributes: action.attributes ?? {},
      raw: action,
    };
  } catch (error) {
    if (activeDetailLoad === token && selectedNodeId.value === node.id) {
      detailError.value = String(error?.message ?? error);
    }
  }
}

function clearDetail() {
  activeDetailLoad = null;
  selectedNodeId.value = null;
  selectedDetail.value = null;
  detailError.value = '';
}

function formatRatio(value) {
  return `${(Number(value ?? 0) * 100).toFixed(1)}%`;
}

function formatTime(value) {
  if (value == null || value === '') {
    return '—';
  }
  const millis = Number(value);
  return Number.isFinite(millis) ? new Date(millis).toLocaleString() : '—';
}

function emptyGraph() {
  return { nodes: [], edges: [], stats: {}, capabilities: {} };
}
</script>

<style scoped>
.trajectory-layout {
  display: grid;
  grid-template-columns: 230px minmax(0, 1fr);
  min-height: 0;
  height: 100%;
  background: var(--bg);
}

.trajectory-layout :deep(.detail-panel) {
  position: static;
  align-self: stretch;
  height: 100%;
  max-height: none;
  border-right: 0;
  overflow: auto;
}
.trajectory-summary {
  padding: 20px 16px;
  border-right: 1px solid var(--border);
  background: var(--surface);
  overflow-y: auto;
}

.trajectory-summary h2 {
  margin: 0 0 6px;
  font-size: 16px;
}

.summary-note,
.capability-warning {
  color: var(--muted);
  font-size: 12px;
  line-height: 1.5;
}

.capability-warning {
  color: var(--warning, #b7791f);
}

.trajectory-summary dl {
  display: grid;
  grid-template-columns: 1fr auto;
  gap: 9px 12px;
  margin: 20px 0;
  font-size: 12px;
}

.trajectory-summary dt {
  color: var(--muted);
}

.trajectory-summary dd {
  margin: 0;
  font-variant-numeric: tabular-nums;
  font-weight: 650;
}

.trajectory-legend {
  display: grid;
  gap: 10px;
  padding: 14px 0;
  border-top: 1px solid var(--border);
  font-size: 12px;
}

.trajectory-key {
  display: grid;
  gap: 8px;
  padding: 0 0 14px;
}

.trajectory-key h3 {
  margin: 0 0 2px;
  color: var(--muted);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.trajectory-key-row {
  display: grid;
  grid-template-columns: 9px 24px minmax(0, 1fr);
  align-items: center;
  gap: 6px;
  min-width: 0;
  font-size: 11px;
}

.trajectory-key-row i {
  width: 9px;
  height: 9px;
  border-radius: 50%;
}

.trajectory-key-row strong {
  font-variant-numeric: tabular-nums;
}

.trajectory-key-row span {
  overflow: hidden;
  color: var(--muted);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.trajectory-legend span {
  display: flex;
  align-items: center;
  gap: 8px;
}

.legend-line {
  width: 28px;
  border-top: 2px solid currentColor;
}

.legend-line.dashed {
  border-top-style: dashed;
  opacity: 0.45;
}

.trajectory-viewport {
  position: relative;
  min-width: 0;
  overflow: hidden;
  background:
    radial-gradient(circle at 1px 1px, color-mix(in srgb, var(--muted) 18%, transparent) 1px, transparent 0)
    0 0 / 24px 24px;
}

.trajectory-viewport :deep(.fullscreen-surface-content) {
  overflow: auto;
}

.trajectory-canvas {
  display: block;
  min-width: 100%;
}

.trajectory-edge {
  fill: none;
  stroke-width: 2;
  opacity: 0.9;
}

.lane-reuse-marker {
  color: var(--muted);
}

.lane-reuse-marker path {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.5;
}

.lane-reuse-marker text {
  fill: currentColor;
  font-size: 10px;
  font-weight: 600;
  paint-order: stroke;
  stroke: var(--bg);
  stroke-width: 3px;
}

.trajectory-node {
  cursor: pointer;
  outline: none;
}

.node-dot {
  stroke: var(--bg);
  stroke-width: 3;
  transition: r 120ms ease, stroke-width 120ms ease;
}

.trajectory-node:hover .node-dot,
.trajectory-node.selected .node-dot,
.trajectory-node:focus-visible .node-dot {
  r: 14px;
  stroke: var(--text);
  stroke-width: 2;
}

.node-label {
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 13px;
  font-weight: 600;
  paint-order: stroke;
  stroke: var(--bg);
  stroke-width: 4px;
  stroke-linejoin: round;
}

.node-label-metadata {
  fill: var(--muted);
  font-size: 11px;
  font-weight: 500;
}

.time-label {
  fill: var(--muted);
  font-size: 11px;
  text-transform: uppercase;
}

.compaction-ring {
  fill: none;
  stroke: #ef4444;
  stroke-width: 2;
  stroke-dasharray: 3 3;
}

.trajectory-empty {
  display: grid;
  place-items: center;
  min-height: 320px;
  color: var(--muted);
}

@media (max-width: 1000px) {
  .trajectory-layout {
    grid-template-columns: 190px minmax(600px, 1fr);
  }

  .trajectory-layout :deep(.fullscreen-surface-aside .detail-panel) {
    width: 100%;
  }
}
</style>
