// Connect waterfall navigation, selection and attribution to the shared timeline.
import { WaterfallDetail } from './detail.js';
import { WaterfallModel } from './model.js';
import { IdlePages } from '../idle/pages.js';
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { formatAttributionDuration } from '../../../../components/time-attribution/model';
import { TABLE_RENDER_LIMITS } from '../../../tableConfig';
import { normalizeTableQuery } from '../../../tableModel';
import { useTimelineNavigation } from '../../time-navigation/useTimelineNavigation.js';
import {
  actionDetail,
  collectDefaultExpandedIds,
  collectParentIds,
  decorateWaterfallRows,
  defaultActiveGroups,
  findWaterfallNode,
  findWaterfallPath,
  flattenMatchingWaterfall,
  flattenVisibleWaterfall,
  formatOffset,
  idleIntervalRows,
  panTimeViewport,
  projectTimeInterval,
  subtreeWindow,
  windowLabel,
} from '../model';

import { rowOverlapsWindow, contextualFocusWindow, attributionOccurrences, decorateBottleneckItems, bottleneckStatusLabel, bottleneckStatusDescription, parseNanos, ratioPercent, projectAttributionLane, laneTargetContext, nanosOffsetMs, compareNanos, contextChip, nanosClock } from './utils.js';

export class WaterfallView {
  constructor(props, emit, t) {
    const idle = new IdlePages();
    watch(() => [props.traceKey, props.waterfall?.actions, props.idleRange?.from, props.idleRange?.to],
      ([traceId, actions, from, to]) => idle.reset(traceId, actions, { from, to }), { immediate: true });
    const expandedIds = ref(new Set());
    const activeGroups = ref(new Set());
    const zoomId = ref(null);
    const focusEnabled = ref(true);
    const bottlenecksExpanded = ref(false);
    const expandedBottleneckGroups = ref(new Set());
    const visibleLimit = ref(TABLE_RENDER_LIMITS.initialRows);
    const detailSelection = new WaterfallDetail();
    const selectedDetailId = detailSelection.id;
    const selectedDetail = detailSelection.data;
    const detailError = detailSelection.error;
    const attributionSection = ref(null);
    const waterfallSection = ref(null);
    const waterfallScroll = ref(null);
    const axisTrack = ref(null);
    const manualTimeViewport = computed({
      get: () => props.timelineViewport,
      set: value => emit('update:viewport', value),
    });
    const builder = new WaterfallModel(props, idle);
    const model = builder.data;
    const modelBuilding = builder.building;
    function showAttribution() {
      attributionSection.value?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    }
    const hasWaterfallData = computed(
      () =>
        Boolean(props.timelineWindow) || (props.waterfall?.actions?.length ?? 0) > 0
        || (props.waterfall?.links?.length ?? 0) > 0
        || idle.data.value.intervals.length > 0,
    );
    onBeforeUnmount(() => { idle.cancel(); clearDetail(); });
    const roots = computed(() => model.value.roots);
    const groups = computed(() => model.value.groups);
    const window = computed(() => model.value.window);
    const idleRows = computed(() =>
      idleIntervalRows(model.value.idleIntervals ?? [], window.value),
    );
    const totalActions = computed(() => model.value.totalActions);
    const windowText = computed(() => windowLabel(window.value));
    const parentIds = computed(() => collectParentIds(roots.value));
    const hasTree = computed(() => parentIds.value.length > 0);
    const normalizedQuery = computed(() => normalizeTableQuery(props.query));
    const queryActive = computed(() => normalizedQuery.value.length > 0);
    const zoomNode = computed(() =>
      zoomId.value ? findWaterfallNode(roots.value, zoomId.value) : null,
    );
    const zoomLabel = computed(() => {
      const node = zoomNode.value;
      if (!node) {
        return '';
      }
      return [node.label, node.target].filter(Boolean).join(' ');
    });
    const displayRoots = computed(() => (zoomNode.value ? [zoomNode.value] : roots.value));
    const focusActionIds = computed(() =>
      Array.from(
        new Set(
          (props.focusInterval?.actionIds ?? [])
            .map((actionId) => String(actionId))
            .filter(Boolean),
        ),
      ),
    );
    const focusActionIdSet = computed(() => new Set(focusActionIds.value));
    const focusActionKey = computed(() => focusActionIds.value.join('|'));
    const focusPaths = computed(() =>
      focusActionIds.value
        .map((actionId) => findWaterfallPath(roots.value, actionId))
        .filter((path) => path.length),
    );
    const primaryFocusPath = computed(() => focusPaths.value[0] ?? []);
    const focusMatchCount = computed(() => focusPaths.value.length);
    const focusWindow = computed(() => {
      if (!focusEnabled.value || !props.focusInterval || !window.value?.startNanos) {
        return null;
      }
      try {
        const startNanos = BigInt(props.focusInterval.startNanos);
        const endNanos = BigInt(props.focusInterval.endNanos);
        if (endNanos <= startNanos) {
          return null;
        }
        return {
          startMs: Number(startNanos - window.value.startNanos) / 1_000_000,
          spanMs: Math.max(Number(endNanos - startNanos) / 1_000_000, 0.001),
        };
      } catch {
        return null;
      }
    });
    const focusTitle = computed(() => {
      const source = props.focusInterval?.source ?? t('source.attribution');
      const dimension = {
        category: t('dimensions.category'),
        round: t('dimensions.round'),
        model: t('dimensions.model'),
        model_request: t('dimensions.modelRequest'),
        tool: t('dimensions.tool'),
        command: t('dimensions.command'),
        command_occurrence: t('dimensions.commandOccurrence'),
        unattributed_gap: t('dimensions.unattributedGap'),
      }[props.focusInterval?.dimension];
      const context = [dimension, props.focusInterval?.label].filter(Boolean).join(' · ');
      return context ? `${source} · ${context}` : `${source} interval`;
    });
    const focusDescription = computed(() => {
      if (!focusWindow.value) {
        return '';
      }
      const start = focusWindow.value.startMs;
      const end = start + focusWindow.value.spanMs;
      const range = `+${formatOffset(start)} → +${formatOffset(end)}`;
      const duration = formatOffset(focusWindow.value.spanMs);
      return [props.focusInterval?.description, `${duration} · ${range}`]
        .filter(Boolean)
        .join(' · ');
    });
    const focusMatchLabel = computed(() => {
      if (!focusActionIds.value.length) {
        return t('timeRange');
      }
      if (!focusMatchCount.value) {
        return t('linkedUnavailable');
      }
      return t('linkedActions', {
        matched: focusMatchCount.value,
        total: focusActionIds.value.length,
        noun: focusActionIds.value.length === 1 ? t('linkedAction') : t('linkedActionPlural'),
      });
    });
    const focusOccurrences = computed(() =>
      attributionOccurrences(
        props.attribution,
        props.focusInterval?.dimension,
        props.focusInterval?.key,
      ),
    );
    const focusOccurrenceIndex = computed(() => {
      const start = String(props.focusInterval?.startNanos ?? '');
      const end = String(props.focusInterval?.endNanos ?? '');
      return focusOccurrences.value.findIndex(
        (occurrence) => occurrence.startNanos === start && occurrence.endNanos === end,
      );
    });
    const focusAxisWindow = computed(() =>
      focusWindow.value
        ? contextualFocusWindow(focusWindow.value, window.value.spanMs)
        : null,
    );
    const baseAxisWindow = computed(() =>
      props.timelineBounds ?? (zoomNode.value
        ? subtreeWindow(zoomNode.value, window.value.spanMs)
        : focusAxisWindow.value ?? { startMs: 0, spanMs: window.value.spanMs }),
    );
    const axisWindow = computed(() => manualTimeViewport.value ?? baseAxisWindow.value);
    const {
      panning: timelinePanning,
      zoomBy: zoomTimeline,
      resetViewport: resetTimeline,
      handleWheel: handleTimelineWheel,
      startPan: startTimelinePan,
    } = useTimelineNavigation({
      viewport: manualTimeViewport,
      activeViewport: axisWindow,
      bounds: baseAxisWindow,
      surfaceRef: waterfallScroll,
      trackRef: axisTrack,
      trackSelector: '.wf-time-track',
      reset: () => {
        manualTimeViewport.value = baseAxisWindow.value;
      },
    });
    const focusBandStyle = computed(() => {
      if (!focusWindow.value || zoomNode.value) {
        return null;
      }
      const projected = projectTimeInterval(
        focusWindow.value.startMs,
        focusWindow.value.startMs + focusWindow.value.spanMs,
        axisWindow.value,
      );
      if (!projected) {
        return null;
      }
      return {
        left: `${projected.leftPct}%`,
        width: `${projected.widthPct}%`,
      };
    });
    const attributionLanes = computed(() => {
      if (!props.attribution || !window.value?.startNanos) {
        return [];
      }
      return [
        {
          id: 'categories',
          label: t('lanes.agentModel'),
          segments: projectAttributionLane(
            props.attribution.segments,
            'category',
            axisWindow.value,
            window.value.startNanos,
            t,
          ),
        },
        {
          id: 'commands',
          label: t('lanes.commands'),
          segments: projectAttributionLane(
            props.attribution.command_segments,
            'command',
            axisWindow.value,
            window.value.startNanos,
            t,
          ),
        },
      ].filter((lane) => lane.segments.length);
    });
    const bottleneckGroups = computed(() => {
      const bottlenecks = props.attribution?.bottlenecks;
      if (!bottlenecks) {
        return [];
      }
      const displayLimit = Math.max(Number(bottlenecks.default_display_limit ?? 5), 1);
      const definitions = [
        {
          id: 'models',
          label: t('bottleneckGroup.models'),
          countNoun: t('countNoun.requests'),
          dimension: 'model_request',
          collection: bottlenecks.model_requests,
          emptyLabel: t('empty.models'),
        },
        {
          id: 'commands',
          label: t('bottleneckGroup.commands'),
          countNoun: t('countNoun.occurrences'),
          dimension: 'command_occurrence',
          collection: bottlenecks.commands,
          emptyLabel: t('empty.commands'),
        },
        {
          id: 'unattributed',
          label: t('bottleneckGroup.unattributed'),
          countNoun: t('countNoun.gaps'),
          dimension: 'unattributed_gap',
          collection: bottlenecks.unattributed_gaps,
          emptyLabel: t('empty.unattributed'),
        },
      ];
      return definitions.map((definition) => {
        const observedCount = Number(definition.collection?.observed_count ?? 0);
        const allItems = decorateBottleneckItems(
          definition.collection?.items,
          props.attribution?.scope?.duration_nanos,
          window.value?.startNanos,
        );
        const isExpanded = expandedBottleneckGroups.value.has(definition.id);
        const items = isExpanded ? allItems : allItems.slice(0, displayLimit);
        return {
          ...definition,
          observedCount,
          displayLimit,
          isExpanded,
          canExpand: allItems.length > displayLimit,
          countLabel: observedCount
            ? t('showing', { shown: items.length, total: observedCount, noun: definition.countNoun })
            : t('zeroCount', { noun: definition.countNoun }),
          items,
        };
      });
    });
    const axisWindowKey = computed(() => {
      const { startMs, spanMs } = axisWindow.value;
      return `${startMs}:${spanMs}`;
    });
    const ticks = computed(() => {
      const { startMs, spanMs } = axisWindow.value;
      return Array.from({ length: 5 }, (_, index) => {
        const fraction = index / 4;
        return { pct: fraction * 100, label: formatOffset(startMs + spanMs * fraction) };
      });
    });
    const allRows = computed(() => {
      const flat = queryActive.value && !focusWindow.value
        ? flattenMatchingWaterfall(displayRoots.value, normalizedQuery.value, activeGroups.value)
        : flattenVisibleWaterfall(displayRoots.value, expandedIds.value, activeGroups.value);
      return focusWindow.value && !zoomNode.value
        ? flat.filter((row) => rowOverlapsWindow(row, focusWindow.value))
        : flat;
    });
    const totalRows = computed(() => allRows.value.length);
    const rows = computed(() => decorateWaterfallRows(
      allRows.value.slice(0, visibleLimit.value),
      axisWindow.value,
    ));
    const idleLaneSegments = computed(() => {
      const segments = [];
      for (const interval of idleRows.value) {
        const style = idleSegmentStyle(interval);
        if (!style) {
          continue;
        }
        segments.push({ id: interval.id, style, title: idleTitle(interval), live: interval.live,
          kind: interval.kind, sessionId: interval.sessionId });
      }
      return segments;
    });
    const agentIdleLanes = computed(() => {
      const lanes = new Map();
      for (const segment of idleLaneSegments.value) {
        const id = JSON.stringify([segment.sessionId, segment.kind]);
        if (!lanes.has(id)) {
          lanes.set(id, { id, sessionId: segment.sessionId,
            label: t('agentStalled'),
            segments: [] });
        }
        lanes.get(id).segments.push(segment);
      }
      return [...lanes.values()];
    });
    const remainingRows = computed(() => Math.max(totalRows.value - rows.value.length, 0));
    const nextBatchSize = computed(() => Math.min(TABLE_RENDER_LIMITS.rowBatchSize, remainingRows.value));
    const hasMoreRows = computed(() => remainingRows.value > 0 && nextBatchSize.value > 0);
    watch(
      model,
      (nextModel, previousModel) => {
        const previousParents = new Set(collectParentIds(previousModel.roots));
        const nextParents = new Set(collectParentIds(nextModel.roots));
        expandedIds.value = new Set([...expandedIds.value].filter(id => nextParents.has(id)));
        for (const id of collectDefaultExpandedIds(nextModel.roots)) if (!previousParents.has(id)) expandedIds.value.add(id);
        const previousGroups = new Set(previousModel.groups.map(group => group.group));
        for (const group of defaultActiveGroups(nextModel.groups)) if (!previousGroups.has(group)) activeGroups.value.add(group);
      },
    );
    watch(() => props.traceKey, () => {
      clearDetail();
      expandedIds.value = new Set();
      activeGroups.value = new Set();
      zoomId.value = null;
    });
    watch(
      () => props.focusInterval?.nonce,
      (nonce) => {
        focusEnabled.value = true;
        zoomId.value = null;
        queueFocusApplication();
        if (nonce) {
          nextTick(() => {
            waterfallSection.value?.scrollIntoView({ behavior: 'smooth', block: 'start' });
          });
        }
      },
      { immediate: true },
    );
    watch(
      () => props.attribution?.trace?.id,
      () => {
        bottlenecksExpanded.value = false;
        expandedBottleneckGroups.value = new Set();
      },
    );
    watch([displayRoots, normalizedQuery, activeGroups], () => {
      visibleLimit.value = TABLE_RENDER_LIMITS.initialRows;
    });
    function select(row) {
      return detailSelection.select(props.traceKey, row.action, (action) => {
        const detail = actionDetail(action, {
          ...row.llmMessages,
          scope: row.llmScope,
          parent: row.agentContext,
          ttft: row.llmPhases?.gap?.durMs ? formatOffset(row.llmPhases.gap.durMs) : null,
        });
        if (focusWindow.value && isFocusAction(row.id)) {
          detail.rows = {
            attribution: focusTitle.value,
            attributed_duration: formatOffset(focusWindow.value.spanMs),
            attributed_interval: focusDescription.value,
            ...detail.rows,
          };
        }
        return detail;
      });
    }
    function clearDetail() {
      detailSelection.clear();
    }
    function isFocusAction(actionId) {
      return focusEnabled.value && focusActionIdSet.value.has(String(actionId));
    }
    const focusActive = computed(() => focusEnabled.value && focusActionIdSet.value.size > 0);
    /**
     * Nested rows repeat the enclosing command line on every row, which reads as
     * noise. Only surface the context when it differs from the row's own target.
     */
    function idleSegmentStyle(interval) {
      const axis = axisWindow.value;
      // Open intervals end at the server-provided axis end. The surrounding
      // workspace refreshes the timeline page to advance this boundary.
      const endMs = interval.endOffsetMs ?? axis.startMs + axis.spanMs;
      const projected = projectTimeInterval(interval.startOffsetMs, endMs, axis);
      if (!projected) {
        return null;
      }
      return {
        left: `${projected.leftPct}%`,
        width: `${projected.widthPct}%`,
      };
    }
    function idleTitle(interval) {
      const lines = [t('agentStalled')];
      lines.push(t('idle.session', { id: interval.sessionId }));
      if (interval.taskId) {
        lines.push(t('idle.task', { id: interval.taskId }));
      }
      lines.push(t('idle.start', { clock: nanosClock(interval.startNanos) }));
      lines.push(t('idle.end', { clock: interval.live ? t('idle.running') : nanosClock(interval.endNanos) }));
      lines.push(t('idle.duration', {
        value: interval.live ? t('idle.running') : formatOffset(interval.durMs),
      }));
      return lines.join('\n');
    }
    function queueFocusApplication() {
      nextTick(applyAttributionFocus);
    }
    async function applyAttributionFocus() {
      if (!focusEnabled.value || !focusWindow.value || !primaryFocusPath.value.length) {
        return;
      }
      const nextExpanded = new Set(expandedIds.value);
      const nextGroups = new Set(activeGroups.value);
      for (const path of focusPaths.value) {
        for (const ancestor of path.slice(0, -1)) {
          if (ancestor.hasChildren) {
            nextExpanded.add(ancestor.id);
          }
        }
        for (const node of path) {
          nextGroups.add(node.kindGroup);
        }
      }
      expandedIds.value = nextExpanded;
      activeGroups.value = nextGroups;
      await nextTick();
      const targetRow = focusActionIds.value
        .map((actionId) => allRows.value.find((row) => row.id === actionId))
        .find(Boolean);
      if (!targetRow) {
        return;
      }
      const rowIndex = allRows.value.findIndex((row) => row.id === targetRow.id);
      if (rowIndex >= visibleLimit.value) {
        visibleLimit.value = rowIndex + 1;
        await nextTick();
      }
      select(targetRow);
      await nextTick();
      const element = Array.from(
        waterfallScroll.value?.querySelectorAll('.wf-row') ?? [],
      ).find((row) => row.dataset.actionId === targetRow.id);
      element?.scrollIntoView({ block: 'center', inline: 'nearest' });
    }
    function navigateOccurrence(delta) {
      const index = focusOccurrenceIndex.value + delta;
      const occurrence = focusOccurrences.value[index];
      if (!occurrence) {
        return;
      }
      emit('open-waterfall', {
        ...props.focusInterval,
        ...occurrence,
      });
    }
    function focusLaneSegment(segment) {
      emit('open-waterfall', segment.target);
    }
    function focusBottleneck(group, item, index) {
      emit('open-waterfall', {
        startNanos: item.start_unix_nanos,
        endNanos: item.end_unix_nanos,
        actionIds: Array.isArray(item.action_ids) ? item.action_ids : [],
        source: t('source.bottlenecks'),
        dimension: group.dimension,
        key: item.key,
        label: item.label,
        description: [
          `#${index + 1} longest ${group.label.toLowerCase()}`,
          item.description,
          bottleneckStatusDescription(item.status),
        ].filter(Boolean).join(' · '),
      });
    }
    function toggleBottleneckGroup(groupId) {
      const next = new Set(expandedBottleneckGroups.value);
      if (next.has(groupId)) {
        next.delete(groupId);
      } else {
        next.add(groupId);
      }
      expandedBottleneckGroups.value = next;
    }
    function isFocusedBottleneck(item) {
      return focusEnabled.value
        && String(item.start_unix_nanos) === String(props.focusInterval?.startNanos ?? '')
        && String(item.end_unix_nanos) === String(props.focusInterval?.endNanos ?? '');
    }
    function isGroupActive(group) {
      return activeGroups.value.has(group);
    }
    function toggleGroup(group) {
      const next = new Set(activeGroups.value);
      if (next.has(group)) {
        next.delete(group);
      } else {
        next.add(group);
      }
      activeGroups.value = next;
    }
    function toggleRow(row) {
      const next = new Set(expandedIds.value);
      if (next.has(row.id)) {
        next.delete(row.id);
      } else {
        next.add(row.id);
      }
      expandedIds.value = next;
    }
    function expandAll() {
      expandedIds.value = new Set(parentIds.value);
    }
    function collapseAll() {
      expandedIds.value = new Set();
    }
    function loadMore() {
      visibleLimit.value += TABLE_RENDER_LIMITS.rowBatchSize;
    }
    function loadAll() {
      visibleLimit.value = totalRows.value;
    }
    function zoomTo(row) {
      if (!row.hasChildren) {
        return;
      }
      zoomId.value = row.id;
      manualTimeViewport.value = subtreeWindow(row, window.value.spanMs);
      const next = new Set(expandedIds.value);
      next.add(row.id);
      expandedIds.value = next;
    }
    function resetView() {
      resetTimeline();
      if (zoomId.value) {
        zoomId.value = null;
        queueFocusApplication();
        return;
      }
      focusEnabled.value = false;
      clearDetail();
      expandedIds.value = new Set(collectDefaultExpandedIds(roots.value));
      activeGroups.value = defaultActiveGroups(groups.value);
      visibleLimit.value = TABLE_RENDER_LIMITS.initialRows;
    }
    Object.assign(this, {
      idle,
      detailError, expandedIds, activeGroups, zoomId, focusEnabled, bottlenecksExpanded,
      expandedBottleneckGroups, visibleLimit, selectedDetailId, selectedDetail, attributionSection,
      waterfallSection, waterfallScroll, axisTrack, manualTimeViewport, model,
      modelBuilding, hasWaterfallData, roots, groups, window,
      idleRows, totalActions, windowText, parentIds, hasTree,
      normalizedQuery, queryActive, zoomNode, zoomLabel, displayRoots,
      focusActionIds, focusActionIdSet, focusActionKey, focusPaths, primaryFocusPath,
      focusMatchCount, focusWindow, focusTitle, focusDescription, focusMatchLabel,
      focusOccurrences, focusOccurrenceIndex, focusAxisWindow, baseAxisWindow, axisWindow,
      focusBandStyle, attributionLanes, bottleneckGroups, axisWindowKey, ticks,
      allRows, totalRows, rows, idleLaneSegments, agentIdleLanes,
      remainingRows, nextBatchSize, hasMoreRows, focusActive, showAttribution,
      select, clearDetail, isFocusAction, idleSegmentStyle,
      idleTitle, queueFocusApplication, applyAttributionFocus, navigateOccurrence, focusLaneSegment,
      focusBottleneck, toggleBottleneckGroup, isFocusedBottleneck, isGroupActive, toggleGroup,
      toggleRow, expandAll, collapseAll, loadMore, loadAll,
      zoomTo, resetView, timelinePanning, zoomTimeline, resetTimeline,
      handleTimelineWheel, startTimelinePan, formatOffset, formatAttributionDuration, rowOverlapsWindow,
      contextualFocusWindow, attributionOccurrences, decorateBottleneckItems, bottleneckStatusLabel, bottleneckStatusDescription,
      parseNanos, ratioPercent, projectAttributionLane, laneTargetContext, nanosOffsetMs,
      compareNanos, contextChip, nanosClock
    });
  }
}
