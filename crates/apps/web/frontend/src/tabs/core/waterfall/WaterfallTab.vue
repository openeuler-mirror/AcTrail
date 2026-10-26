<!-- Show action rows and details on the shared continuous timeline. -->
<template>
  <section class="time-waterfall-page">
    <IdleControls :pager="idle" />
    <section v-if="attribution" ref="attributionSection" class="time-waterfall-attribution">
      <TimeAttributionTab
        :attribution="attribution"
        :query="query"
        :initial-detail="focusInterval?.dimension ?? ''"
        :initial-key="focusInterval?.key ?? ''"
        @open-waterfall="$emit('open-waterfall', $event)"
      />
    </section>

    <button v-if="!attribution" :disabled="attributionLoading" type="button" class="tree-action" @click="$emit('load-attribution')">{{ attributionLoading ? t('loadingAttribution') : t('loadAttribution') }}</button>

    <section
      ref="waterfallSection"
      class="time-waterfall-section"
      aria-labelledby="waterfall-section-title"
    >
      <header class="time-waterfall-heading">
        <span>{{ t('kicker') }}</span>
        <h2 id="waterfall-section-title">{{ t('title') }}</h2>
      </header>
  <section class="tab-detail-layout waterfall-detail-layout" :class="{ 'detail-open': selectedDetail }">
    <section class="waterfall-panel tab-detail-main">
    <div class="waterfall-toolbar">
      <span class="wf-count">
        {{ t('countActions', { count: totalActions }) }}
        <template v-if="windowText"> · {{ windowText }}</template>
      </span>
      <div class="wf-actions">
        <button
          type="button"
          class="tree-action"
          :disabled="!hasTree || queryActive"
          @click="expandAll"
        >
          <ChevronsUpDown :size="15" aria-hidden="true" />
          {{ t('expandAll') }}
        </button>
        <button
          type="button"
          class="tree-action"
          :disabled="!hasTree || queryActive"
          @click="collapseAll"
        >
          <ChevronsDownUp :size="15" aria-hidden="true" />
          {{ t('collapseAll') }}
        </button>
      </div>
    </div>

    <div
      v-if="zoomLabel || focusWindow"
      class="waterfall-breadcrumb"
      :class="{ 'attribution-focus': focusWindow && !zoomLabel }"
    >
      <Search :size="14" aria-hidden="true" />
      <span class="wf-location-copy">
        <strong class="wf-zoom-label">
          {{ zoomLabel ? `Zoomed: ${zoomLabel}` : focusTitle }}
        </strong>
        <small v-if="focusWindow && !zoomLabel">{{ focusDescription }}</small>
      </span>
      <span
        v-if="focusWindow && !zoomLabel"
        class="wf-focus-status"
        :class="{ matched: focusMatchCount > 0 }"
      >
        {{ focusMatchLabel }}
      </span>
      <span
        v-if="focusOccurrences.length > 1 && !zoomLabel"
        class="wf-occurrence-nav"
      >
        <button
          type="button"
          :title="t('previousOccurrence')"
          :disabled="focusOccurrenceIndex <= 0"
          @click="navigateOccurrence(-1)"
        >
          <ChevronLeft :size="13" aria-hidden="true" />
        </button>
        <small>
          {{ focusOccurrenceIndex + 1 }} / {{ focusOccurrences.length }}
        </small>
        <button
          type="button"
          :title="t('nextOccurrence')"
          :disabled="focusOccurrenceIndex < 0 || focusOccurrenceIndex + 1 >= focusOccurrences.length"
          @click="navigateOccurrence(1)"
        >
          <ChevronRight :size="13" aria-hidden="true" />
        </button>
      </span>
      <button
        v-if="focusWindow && !zoomLabel"
        type="button"
        class="wf-zoom-reset"
        @click="showAttribution"
      >
        {{ t('backToAttribution') }}
      </button>
      <button type="button" class="wf-zoom-reset" @click="resetView">
        {{ focusWindow && !zoomLabel ? t('showFullTrace') : t('resetView') }}
      </button>
    </div>

    <div v-if="groups.length || idleRows.length" class="waterfall-legend">
      <button
        v-for="group in groups"
        :key="group.group"
        type="button"
        class="wf-chip"
        :class="[`wf-group-${group.group}`, { inactive: !isGroupActive(group.group) }]"
        @click="toggleGroup(group.group)"
      >
        <span class="wf-chip-dot"></span>
        {{ group.group }}
        <small>{{ group.count }}</small>
      </button>
      <span
        v-if="idleRows.length"
        class="wf-chip wf-chip-idle"
        :title="t('idleIntervals', { count: idleRows.length })"
      >
        <span class="wf-chip-dot"></span>
        {{ t('userIdle') }}
        <small>{{ idleRows.length }}</small>
      </span>
      <div v-if="isGroupActive('llm')" class="wf-phase-legend" aria-hidden="true">
        <span class="wf-phase-key wf-bar-request">{{ t('phaseRequest') }}</span>
        <span class="wf-phase-key wf-bar-ttft">{{ t('phaseTtft') }}</span>
        <span class="wf-phase-key wf-bar-response">{{ t('phaseResponse') }}</span>
      </div>
      <small class="wf-navigation-hint">{{ t('navigationHint') }}</small>
    </div>

    <section v-if="bottleneckGroups.length" class="waterfall-bottlenecks">
      <header class="wf-bottleneck-header">
        <div class="wf-bottleneck-heading">
          <Gauge :size="16" aria-hidden="true" />
          <span>
            <strong>{{ t('bottlenecks') }}</strong>
            <small>Top {{ attribution?.bottlenecks?.default_display_limit ?? 5 }} per type by default; concurrent intervals may overlap.</small>
          </span>
        </div>
        <button
          type="button"
          class="wf-bottleneck-toggle"
          :aria-expanded="bottlenecksExpanded"
          @click="bottlenecksExpanded = !bottlenecksExpanded"
        >
          {{ bottlenecksExpanded ? 'Collapse' : 'Expand' }}
          <ChevronDown v-if="bottlenecksExpanded" :size="14" aria-hidden="true" />
          <ChevronRight v-else :size="14" aria-hidden="true" />
        </button>
      </header>
      <div v-if="bottlenecksExpanded" class="wf-bottleneck-grid">
        <article
          v-for="group in bottleneckGroups"
          :key="group.id"
          class="wf-bottleneck-group"
          :class="`wf-bottleneck-${group.id}`"
        >
          <header>
            <span class="wf-bottleneck-dot"></span>
            <strong>{{ group.label }}</strong>
            <small>{{ group.countLabel }}</small>
          </header>
          <div
            v-if="group.items.length"
            class="wf-bottleneck-list"
            :class="{ expanded: group.isExpanded }"
          >
            <button
              v-for="(item, index) in group.items"
              :key="item.id"
              type="button"
              class="wf-bottleneck-item"
              :class="{ active: isFocusedBottleneck(item) }"
              :title="t('locateInWaterfall', { label: item.label })"
              @click="focusBottleneck(group, item, index)"
            >
              <span class="wf-bottleneck-rank">{{ index + 1 }}</span>
              <span class="wf-bottleneck-copy">
                <span class="wf-bottleneck-name">
                  <strong>{{ item.label }}</strong>
                  <em v-if="item.status && item.status !== 'complete'">{{ bottleneckStatusLabel(item.status) }}</em>
                </span>
                <small>{{ item.description }} · {{ item.offsetLabel }}</small>
                <span class="wf-bottleneck-meter" aria-hidden="true">
                  <span :style="{ width: `${item.relativeWidth}%` }"></span>
                </span>
              </span>
              <span class="wf-bottleneck-duration">
                <strong>{{ formatAttributionDuration(item.duration_nanos) }}</strong>
                <small>{{ item.traceShare }}</small>
              </span>
            </button>
          </div>
          <div v-else class="wf-bottleneck-empty">{{ group.emptyLabel }}</div>
          <button
            v-if="group.canExpand"
            type="button"
            class="wf-bottleneck-more"
            :aria-expanded="group.isExpanded"
            @click="toggleBottleneckGroup(group.id)"
          >
            {{ group.isExpanded ? `Show top ${group.displayLimit}` : `Show all ${group.observedCount}` }}
            <ChevronUp v-if="group.isExpanded" :size="13" aria-hidden="true" />
            <ChevronDown v-else :size="13" aria-hidden="true" />
          </button>
        </article>
      </div>
    </section>

    <div
      v-if="timelineWindow || rows.length || idleLaneSegments.length"
      ref="waterfallScroll"
      class="waterfall-scroll"
      :class="{ 'is-panning': timelinePanning }"
      :aria-label="t('timelineAria')"
      @wheel="handleTimelineWheel"
      @pointerdown="startTimelinePan"
    >
      <div class="waterfall-axis">
        <div class="wf-gutter">{{ t('gutterAction') }}</div>
        <div ref="axisTrack" class="wf-axis-track wf-time-track">
          <span
            v-if="focusBandStyle"
            class="wf-axis-focus"
            :style="focusBandStyle"
            :title="focusDescription"
          ></span>
          <span v-for="tick in ticks" :key="tick.pct" class="wf-tick" :style="{ left: `${tick.pct}%` }">
            {{ tick.label }}
          </span>
        </div>
      </div>

      <div v-if="attributionLanes.length" class="waterfall-attribution-lanes">
        <div
          v-for="lane in attributionLanes"
          :key="lane.id"
          class="wf-attribution-lane"
        >
          <div class="wf-gutter">
            {{ lane.label }}
            <small>{{ lane.segments.length }}</small>
          </div>
          <div class="wf-attribution-track wf-time-track">
            <button
              v-for="segment in lane.segments"
              :key="segment.id"
              type="button"
              class="wf-attribution-segment"
              :class="segment.className"
              :style="segment.style"
              :title="segment.title"
              @click="focusLaneSegment(segment)"
            >
              <span v-if="segment.showLabel">{{ segment.label }}</span>
            </button>
          </div>
        </div>
      </div>

      <div v-if="idleLaneSegments.length" class="waterfall-idle-lanes">
        <div v-for="lane in agentIdleLanes" :key="lane.id" class="wf-idle-lane">
          <div class="wf-gutter" :title="`Session: ${lane.sessionId}`">
            {{ lane.label }}
            <small>{{ lane.segments.length }}</small>
          </div>
          <div class="wf-idle-track wf-time-track">
            <span
              v-for="segment in lane.segments"
              :key="segment.id"
              class="wf-idle-segment"
              :class="{ live: segment.live, waiting: segment.kind === 'hang' }"
              :style="segment.style"
              :title="segment.title"
            ></span>
          </div>
        </div>
      </div>

      <div class="waterfall-rows" :class="{ 'is-focusing': focusActive }">
        <div
          v-for="row in rows"
          :key="row.id"
          v-memo="[row.id, row.expanded, row.barTitle, selectedDetailId, axisWindowKey, focusActionKey]"
          class="wf-row"
          :class="[row.kindClass, {
            selected: row.id === selectedDetailId,
            'attribution-match': isFocusAction(row.id),
          }]"
          :data-action-id="row.id"
          @click="select(row)"
          @dblclick="zoomTo(row)"
        >
          <div class="wf-label" :style="{ paddingLeft: `${row.depth * 16 + 10}px` }">
            <button
              v-if="row.hasChildren"
              type="button"
              class="wf-toggle"
              @click.stop="toggleRow(row)"
            >
              <ChevronDown v-if="row.expanded" :size="14" aria-hidden="true" />
              <ChevronRight v-else :size="14" aria-hidden="true" />
            </button>
            <span v-else class="wf-toggle-spacer"></span>
            <span class="wf-kind" aria-hidden="true"></span>
            <div class="wf-label-main">
              <div class="wf-label-line">
                <span class="wf-label-text" :title="row.label">{{ row.label }}</span>
                <span v-if="row.llmScope" class="wf-llm-scope" :title="row.llmScope">{{ row.llmScope }}</span>
                <span v-if="row.target" class="wf-label-target" :title="row.target">{{ row.target }}</span>
              </div>
              <div v-if="contextChip(row)" class="wf-agent-context" :title="row.agentContext">
                <span aria-hidden="true">↳</span>{{ contextChip(row) }}
              </div>
              <div class="wf-label-meta">
                <span class="wf-meta-start" :title="`start +${formatOffset(row.startOffsetMs)}`">
                  {{ row.startClock || row.startOffsetLabel }}
                </span>
                <DurationBadge :live="row.live">{{ row.durationText }}</DurationBadge>
              </div>
              <div
                v-if="row.llmRequestPreview || row.llmResponsePreview"
                class="wf-llm-messages"
              >
                <div
                  v-if="row.llmRequestPreview"
                  class="wf-llm-message wf-llm-message-request"
                  :title="row.llmMessages?.requestFull || row.llmRequestPreview"
                >
                  <span class="wf-llm-message-label">{{ t('roleUser') }}</span>
                  <span class="wf-llm-message-text">{{ row.llmRequestPreview }}</span>
                </div>
                <div
                  v-if="row.llmResponsePreview"
                  class="wf-llm-message wf-llm-message-response"
                  :title="row.llmMessages?.responseFull || row.llmResponsePreview"
                >
                  <span class="wf-llm-message-label">{{ t('roleAssistant') }}</span>
                  <span class="wf-llm-message-text">{{ row.llmResponsePreview }}</span>
                </div>
              </div>
            </div>
            <button
              v-if="row.hasChildren"
              type="button"
              class="wf-zoom"
              :title="t('zoomToSubtree')"
              @click.stop="zoomTo(row)"
            >
              <ZoomIn :size="13" aria-hidden="true" />
            </button>
          </div>
          <div class="wf-track wf-time-track">
            <span
              v-if="focusBandStyle"
              class="wf-focus-band"
              :class="{ linked: isFocusAction(row.id) }"
              :style="focusBandStyle"
              :title="isFocusAction(row.id) ? focusDescription : undefined"
            ></span>
            <template v-if="row.barSegments?.length">
              <div
                v-for="(segment, index) in row.barSegments"
                :key="`${row.id}-${segment.kind}-${index}`"
                class="wf-bar wf-bar-phase"
                :class="[
                  `wf-bar-${segment.kind}`,
                  `wf-status-${row.status}`,
                  { live: row.live && segment.kind !== 'ttft' },
                  { instant: segment.instant },
                ]"
                :style="segment.style"
                :title="row.barTitle"
              />
            </template>
            <div
              v-else-if="row.barStyle"
              class="wf-bar"
              :class="[
                row.barClass,
                `wf-status-${row.status}`,
                { live: row.live, instant: row.barInstant },
              ]"
              :style="row.barStyle"
              :title="row.barTitle"
            />
          </div>
        </div>

        <div v-if="hasMoreRows" class="wf-load-more-row">
          <button type="button" class="wf-load-more" @click="loadMore">
            Load {{ nextBatchSize }} more ({{ remainingRows }} hidden)
          </button>
          <button type="button" class="wf-load-all" @click="loadAll">{{ t('loadAll') }}</button>
        </div>
      </div>
    </div>

    <div v-else-if="modelBuilding && hasWaterfallData" class="waterfall-empty">{{ t('building') }}</div>
    <div v-else class="waterfall-empty">{{ t('noActions') }}</div>
    </section>
    <DetailPanel
      :detail="selectedDetail"
      :error="detailError"
      :trace-id="traceKey"
      hide-when-empty
      @clear="clearDetail"
    />
  </section>
    </section>
  </section>
</template>

<script setup>
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  ChevronsDownUp,
  ChevronsUpDown,
  Gauge,
  Search,
  ZoomIn,
} from '@lucide/vue';

import DetailPanel from '../../../components/DetailPanel.vue';
import IdleControls from './idle/IdleControls.vue';
import DurationBadge from '../../../components/DurationBadge.vue';
import { useModuleLocale } from '../../../locale';
import TimeAttributionTab from '../time-attribution/TimeAttributionTab.vue';
import strings from './locale';

const { t } = useModuleLocale(strings);

import { WaterfallView } from './view/main.js';

const props = defineProps({
  timelineWindow: { type: Object, default: null },
  timelineBounds: { type: Object, default: null },
  timelineViewport: { type: Object, default: null },
  idleRange: { type: Object, default: () => ({}) },
  traceKey: { type: [String, Number], default: null },
  traceDetail: { type: Object, default: null },
  actionTree: { type: Object, default: null },
  waterfall: { type: Object, required: true },
  query: { type: String, default: '' },
  focusInterval: { type: Object, default: null },
  attributionLoading: { type: Boolean, default: false },
  attribution: { type: Object, default: null },
});

const emit = defineEmits(['open-waterfall', 'load-attribution', 'update:viewport']);

const {
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
} = new WaterfallView(props, emit, t);
</script>
<style src="./waterfall.css" scoped></style>
