<!-- Expandable res area: host and cgroup resource tracks on the flame graph's time axis. -->
<template>
  <section ref="root" class="flame-layer layer-res">
    <button
      type="button"
      class="flame-layer-header"
      :aria-expanded="expanded"
      @click="toggleExpanded"
    >
      <span class="flame-layer-chevron">
        <ChevronDown v-if="expanded" :size="16" aria-hidden="true" />
        <ChevronRight v-else :size="16" aria-hidden="true" />
      </span>
      <span class="flame-layer-dot"></span>
      <span class="flame-layer-copy">
        <strong>{{ t('resources.title') }}</strong>
      </span>
      <span class="flame-layer-stats">
        <template v-if="expanded && loading"><small>{{ t('resources.loading') }}</small></template>
        <template v-else-if="tracks?.cgroup.available && tracks.cgroup.source">
          <span class="res-source" :class="`source-${tracks.cgroup.source}`">
            {{ t(`resources.source.${tracks.cgroup.source}`) }}
          </span>
        </template>
      </span>
    </button>

    <div v-if="expanded" class="flame-tracks res-tracks">
      <div v-if="error" class="flame-layer-empty res-error">{{ t('resources.error', { message: error }) }}</div>
      <template v-for="group in GROUPS" :key="group">
        <div class="flame-track-row res-group-row">
          <div class="fg-gutter res-group-gutter">
            <span>{{ t(`resources.groups.${group}`) }}</span>
          </div>
          <div class="res-group-info">
            <template v-if="group === 'host'">
              <small v-if="hostSpec">{{ hostSpec }}</small>
            </template>
            <template v-else-if="tracks?.cgroup.source">
              <!-- The source explanation and cgroup path only help debugging, so they live in the tooltip. -->
              <span
                class="res-source"
                :class="`source-${tracks.cgroup.source}`"
                :title="[t(`resources.sourceHints.${tracks.cgroup.source}`), tracks.cgroup.sourcePath].filter(Boolean).join('\n')"
              >
                {{ t(`resources.source.${tracks.cgroup.source}`) }}
              </span>
            </template>
            <small
              v-for="reason in visibleReasons(group)"
              :key="reason"
              class="res-reason"
            >
              {{ t(`resources.reasons.${reason}`) }}
            </small>
          </div>
        </div>
        <template v-if="tracks?.[group].available">
          <div
            v-for="id in GROUP_TRACKS[group]"
            :key="id"
            class="flame-track-row res-track-row"
          >
            <div class="fg-gutter res-gutter">
              <span>{{ t(`resources.tracks.${id}`) }}</span>
              <small class="res-value">{{ gutterValue(group, id) }}</small>
            </div>
            <div class="flame-track fg-time-track res-track">
              <span v-if="isMissing(group, id)" class="res-not-collected">{{ t('resources.reasons.not_collected') }}</span>
              <template v-else>
                <ResourceTrackCanvas
                  :track="tracks[group].tracks[id]"
                  :viewport="canvasViewport"
                  :hover-ms="hoverMs"
                  :label="t(`resources.tracks.${id}`)"
                  @hover="hoverMs = $event"
                />
                <span class="res-scale">{{ formatResourceValue(tracks[group].tracks[id].unit, tracks[group].tracks[id].yMax, t) }}</span>
              </template>
            </div>
          </div>
        </template>
      </template>
    </div>
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue';
import { ChevronDown, ChevronRight } from '@lucide/vue';

import { readResourceTimeline } from '../../../../api.js';
import { useModuleLocale } from '../../../../locale';
import strings from '../locale';
import ResourceTrackCanvas from './ResourceTrackCanvas.vue';
import {
  GROUP_TRACKS,
  buildResourceTracks,
  formatBytes,
  formatResourceValue,
  planCoversViewport,
  planResourceQuery,
  timelineOriginMs,
  valuesAt,
} from './model.js';

const { t } = useModuleLocale(strings);

const props = defineProps({
  traceKey: { type: [String, Number], default: null },
  timelineWindow: { type: Object, default: null },
  bounds: { type: Object, required: true },
  viewport: { type: Object, required: true },
  // Mutated in place by the flame graph on every pan frame; drawing reads it directly.
  canvasViewport: { type: Object, required: true },
});

const GROUPS = ['host', 'cgroup'];
const STORAGE_KEY = 'actrail.flameGraph.resources.expanded';
const FETCH_DELAY_MS = 150;

const root = ref(null);
const expanded = ref(readExpanded());
const timeline = shallowRef(null);
const plan = shallowRef(null);
const loading = ref(false);
const error = ref('');
const hoverMs = ref(null);
const widthPx = ref(800);
let controller = null;
let timer = null;
let resizeObserver = null;

const originMs = computed(() => timelineOriginMs(props.timelineWindow));
const tracks = computed(() => (timeline.value && originMs.value != null
  ? buildResourceTracks(timeline.value, originMs.value)
  : null));
/** "16 CPUs · 15.4 GiB" for the host group. */
const hostSpec = computed(() => [
  tracks.value?.host.cpuCount ? t('resources.cpuCount', { count: tracks.value.host.cpuCount }) : null,
  tracks.value?.host.memoryTotal ? formatBytes(tracks.value.host.memoryTotal) : null,
].filter(Boolean).join(' · '));

watch(
  () => [expanded.value, props.traceKey, originMs.value, props.viewport, props.bounds, widthPx.value],
  scheduleFetch,
  { immediate: true },
);
watch(() => props.traceKey, () => {
  controller?.abort();
  timeline.value = null;
  plan.value = null;
  error.value = '';
  hoverMs.value = null;
});

onMounted(() => {
  resizeObserver = new ResizeObserver(([entry]) => {
    // The gutter is 156px; the remainder is the time track.
    const width = Math.round((entry?.contentRect.width ?? 956) - 156);
    if (width > 0 && Math.abs(width - widthPx.value) > 16) widthPx.value = width;
  });
  resizeObserver.observe(root.value);
});

onBeforeUnmount(() => {
  clearTimeout(timer);
  controller?.abort();
  resizeObserver?.disconnect();
});

defineExpose({ expanded });

function toggleExpanded() {
  expanded.value = !expanded.value;
  try {
    localStorage.setItem(STORAGE_KEY, expanded.value ? '1' : '0');
  } catch {
    // Storage may be unavailable; the toggle still works for this page.
  }
}

/** Expanded unless this browser collapsed it, like the flame graph's other layers. */
function readExpanded() {
  try {
    return localStorage.getItem(STORAGE_KEY) !== '0';
  } catch {
    return true;
  }
}

function visibleReasons(group) {
  return tracks.value?.[group].reasons ?? [];
}

function isMissing(group, id) {
  return group === 'cgroup' && tracks.value?.cgroup.missing.includes(id);
}

/** The hovered value, or the window maximum when nothing is hovered. */
function gutterValue(group, id) {
  const track = tracks.value?.[group].tracks[id];
  if (!track || isMissing(group, id) || track.empty) return '';
  if (hoverMs.value != null) {
    // Points are as sparse as the sampling interval, which is often wider than a bucket.
    const interval = timeline.value[group === 'host' ? 'host' : 'workload'].sample_interval_ms ?? 0;
    const distance = Math.max(timeline.value.bucket_ms * 1.5, interval * 0.75, 1);
    const [first] = valuesAt(track, hoverMs.value, distance);
    return formatResourceValue(track.unit, first.value, t);
  }
  return '';
}

function scheduleFetch() {
  clearTimeout(timer);
  if (!expanded.value || props.traceKey == null || originMs.value == null) return;
  const bucketMs = timeline.value?.bucket_ms ?? Infinity;
  if (planCoversViewport(plan.value, bucketMs, props.viewport, widthPx.value)
    && plan.value.traceKey === props.traceKey
    && plan.value.boundsEnd >= props.bounds.startMs + props.bounds.spanMs - bucketMs) {
    return;
  }
  timer = setTimeout(fetchWindow, timeline.value ? FETCH_DELAY_MS : 0);
}

async function fetchWindow() {
  controller?.abort();
  const current = new AbortController();
  controller = current;
  const traceKey = props.traceKey;
  const next = planResourceQuery(originMs.value, props.viewport, props.bounds, widthPx.value);
  loading.value = true;
  try {
    const response = await readResourceTimeline(traceKey, {
      fromMs: next.fromMs,
      toMs: next.toMs,
      maxPoints: next.maxPoints,
      signal: current.signal,
    });
    if (traceKey !== props.traceKey) return;
    timeline.value = response;
    plan.value = { ...next, traceKey, boundsEnd: props.bounds.startMs + props.bounds.spanMs };
    error.value = '';
  } catch (failure) {
    if (failure?.name === 'AbortError') return;
    error.value = String(failure?.message ?? failure);
  } finally {
    if (controller === current) loading.value = false;
  }
}
</script>

<style src="../flame-graph.css" scoped></style>
<style src="./resources.css" scoped></style>
