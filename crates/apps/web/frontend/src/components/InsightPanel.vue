<template>
  <section v-if="insight" class="detail-section action-insight">
    <h3>{{ insight.heading }}</h3>

    <div v-if="insight.chips.length" class="insight-chip-row">
      <span v-for="chip in insight.chips" :key="chip.label" class="insight-chip">
        <small>{{ chip.label }}</small>
        <strong>{{ chip.value }}</strong>
      </span>
    </div>

    <p v-if="loadingMessage" class="detail-muted">{{ loadingMessage }}</p>
    <p v-if="error" class="detail-error">{{ error }}</p>

    <div v-if="insight.blocks.length" class="insight-block-stack">
      <article
        v-for="block in insight.blocks"
        :key="block.id"
        class="insight-block"
        :class="`tone-${block.tone}`"
      >
        <header>
          <button
            v-if="block.collapsible"
            class="insight-block-toggle"
            type="button"
            :aria-expanded="!blockCollapsed(block)"
            @click="toggleBlock(block)"
          >
            <ChevronRight
              class="insight-block-chevron"
              :class="{ expanded: !blockCollapsed(block) }"
              :size="15"
              aria-hidden="true"
            />
            <span>
              <small>{{ block.label }}</small>
              <strong>{{ block.title }}</strong>
            </span>
          </button>
          <template v-else>
            <span>{{ block.label }}</span>
            <strong>{{ block.title }}</strong>
          </template>
        </header>

        <div v-if="!blockCollapsed(block)" class="insight-block-body">
          <dl v-if="block.rows?.length" class="insight-block-rows">
            <template v-for="[key, value] in block.rows" :key="key">
              <dt>{{ key }}</dt>
              <dd>{{ value }}</dd>
            </template>
          </dl>

          <pre v-if="block.text">{{ block.text }}</pre>

          <div v-if="itemLimitControlVisible(block)" class="insight-item-controls">
            <span>{{ itemLimitSummary(block) }}</span>
            <label>
              Show
              <input
                type="number"
                :min="1"
                :max="block.items.length"
                :value="itemLimit(block)"
                @input="updateItemLimit(block, $event)"
              />
            </label>
          </div>

          <ul v-if="block.items?.length" class="insight-item-list">
            <li v-for="(item, index) in visibleItems(block)" :key="itemKey(block, item, index)">
              <div>
                <strong>{{ item.title }}</strong>
                <span v-if="item.subtitle">{{ item.subtitle }}</span>
              </div>
              <pre v-if="item.text">{{ item.text }}</pre>
            </li>
          </ul>
        </div>
      </article>
    </div>
  </section>
</template>

<script setup>
import { ref, watch } from 'vue';
import { ChevronRight } from '@lucide/vue';

const props = defineProps({
  insight: {
    type: Object,
    default: null,
  },
  loadingMessage: {
    type: String,
    default: '',
  },
  error: {
    type: String,
    default: '',
  },
});

const collapsedBlocks = ref(new Map());
const touchedBlocks = ref(new Set());
const itemLimits = ref(new Map());
const touchedItemLimits = ref(new Set());

watch(
  () => props.insight,
  (insight) => {
    const next = new Map(collapsedBlocks.value);
    for (const block of insight?.blocks ?? []) {
      const key = blockKey(insight, block);
      if (touchedBlocks.value.has(key)) {
        continue;
      }
      if (block.collapsible && block.defaultCollapsed) {
        next.set(key, true);
      } else {
        next.delete(key);
      }
    }
    collapsedBlocks.value = next;
    const nextLimits = new Map(itemLimits.value);
    for (const block of insight?.blocks ?? []) {
      if (!block.items?.length || !block.itemLimit) {
        continue;
      }
      const key = blockKey(insight, block);
      if (touchedItemLimits.value.has(key)) {
        nextLimits.set(key, clampItemLimit(nextLimits.get(key), block));
      } else {
        nextLimits.set(key, defaultItemLimit(block));
      }
    }
    itemLimits.value = nextLimits;
  },
  { immediate: true },
);

function blockCollapsed(block) {
  if (!block?.collapsible) {
    return false;
  }
  return Boolean(collapsedBlocks.value.get(blockKey(props.insight, block)));
}

function toggleBlock(block) {
  if (!block?.collapsible) {
    return;
  }
  const key = blockKey(props.insight, block);
  const next = new Map(collapsedBlocks.value);
  if (next.get(key)) {
    next.delete(key);
  } else {
    next.set(key, true);
  }
  collapsedBlocks.value = next;
  touchedBlocks.value = new Set(touchedBlocks.value).add(key);
}

function blockKey(insight, block) {
  return `${insight?.instanceId ?? insight?.kind ?? insight?.heading ?? 'insight'}:${block?.id ?? 'block'}`;
}

function visibleItems(block) {
  if (!block?.items?.length) {
    return [];
  }
  if (!block.itemLimit) {
    return block.items;
  }
  return block.items.slice(0, itemLimit(block));
}

function itemLimitControlVisible(block) {
  return Boolean(block?.itemLimit && block?.items?.length > block.itemLimit);
}

function itemLimit(block) {
  if (!block?.items?.length) {
    return 0;
  }
  if (!block.itemLimit) {
    return block.items.length;
  }
  const stored = itemLimits.value.get(blockKey(props.insight, block));
  return stored === undefined ? defaultItemLimit(block) : clampItemLimit(stored, block);
}

function updateItemLimit(block, event) {
  const key = blockKey(props.insight, block);
  const next = new Map(itemLimits.value);
  next.set(key, clampItemLimit(event.target.value, block));
  itemLimits.value = next;
  touchedItemLimits.value = new Set(touchedItemLimits.value).add(key);
}

function itemLimitSummary(block) {
  return `Showing ${itemLimit(block)} of ${block.items.length}`;
}

function itemKey(block, item, index) {
  return `${blockKey(props.insight, block)}:${item?.id ?? 'item'}:${index}`;
}

function defaultItemLimit(block) {
  if (!block?.itemLimit) {
    return block?.items?.length ?? 0;
  }
  return clampItemLimit(block.itemLimit, block);
}

function clampItemLimit(raw, block) {
  const total = block?.items?.length ?? 0;
  if (total <= 0) {
    return 0;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) {
    return Math.min(total, 1);
  }
  return Math.min(total, Math.max(1, parsed));
}
</script>

<style scoped>
.action-insight {
  display: grid;
  gap: 10px;
}

.insight-chip-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}

.insight-chip {
  min-width: 0;
  display: inline-grid;
  gap: 1px;
  padding: 6px 8px;
  border: 1px solid var(--trace-insight-chip-border);
  border-radius: 8px;
  background: var(--trace-insight-chip-bg);
}

.insight-chip small {
  color: var(--muted);
  font-size: 10px;
  font-weight: 800;
  text-transform: uppercase;
}

.insight-chip strong {
  min-width: 0;
  color: var(--text);
  font-size: 12px;
  overflow-wrap: anywhere;
}

.insight-block-stack {
  display: grid;
  gap: 10px;
}

.insight-block {
  min-width: 0;
  padding: 11px;
  border: 1px solid var(--trace-insight-block-border);
  border-left: 4px solid var(--teal);
  border-radius: 8px;
  background: var(--trace-insight-block-bg);
  box-shadow: var(--trace-insight-block-shadow);
}

.insight-block.tone-tools {
  border-left-color: var(--trace-insight-tools-border);
  background: var(--trace-insight-tools-bg);
}

.insight-block.tone-reasoning {
  border-left-color: var(--trace-insight-reasoning-border);
  background: var(--trace-insight-reasoning-bg);
}

.insight-block.tone-response {
  border-left-color: var(--trace-insight-response-border);
  background: var(--trace-insight-response-bg);
}

.insight-block.tone-context {
  border-left-color: var(--trace-insight-context-border);
  background: var(--trace-insight-context-bg);
}

.insight-block.tone-http {
  border-left-color: var(--trace-insight-http-border);
  background: var(--trace-insight-http-bg);
}

.insight-block.tone-status {
  border-left-color: var(--trace-insight-status-border);
  background: var(--trace-insight-status-bg);
}

.insight-block header {
  display: grid;
  gap: 2px;
  margin-bottom: 8px;
}

.insight-block header span {
  color: var(--muted);
  font-size: 10px;
  font-weight: 900;
  text-transform: uppercase;
}

.insight-block header strong {
  min-width: 0;
  font-size: 13px;
  overflow-wrap: anywhere;
}

.insight-block-toggle {
  min-width: 0;
  display: grid;
  grid-template-columns: 18px minmax(0, 1fr);
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.insight-block-toggle span {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.insight-block-toggle small {
  color: var(--muted);
  font-size: 10px;
  font-weight: 900;
  text-transform: uppercase;
}

.insight-block-toggle strong {
  min-width: 0;
  font-size: 13px;
  overflow-wrap: anywhere;
}

.insight-block-toggle:hover .insight-block-chevron,
.insight-block-toggle:focus-visible .insight-block-chevron {
  border-color: var(--trace-insight-chevron-hover-border);
  background: var(--trace-insight-chevron-hover-bg);
  transform: translateX(1px);
}

.insight-block-toggle:focus-visible {
  outline: 2px solid var(--trace-insight-focus-outline);
  outline-offset: 4px;
}

.insight-block-chevron {
  box-sizing: content-box;
  padding: 2px;
  border: 1px solid var(--trace-insight-chevron-border);
  border-radius: 5px;
  background: var(--trace-insight-chevron-bg);
  color: var(--trace-insight-chevron-text);
  transition:
    transform 0.14s ease,
    background-color 0.14s ease,
    border-color 0.14s ease;
}

.insight-block-chevron.expanded {
  transform: rotate(90deg);
}

.insight-block-toggle:hover .insight-block-chevron.expanded,
.insight-block-toggle:focus-visible .insight-block-chevron.expanded {
  transform: rotate(90deg) translateX(1px);
}

.insight-block-body {
  min-width: 0;
}

.insight-block pre,
.insight-item-list pre {
  max-height: 220px;
  margin: 8px 0 0;
  padding: 10px;
  overflow: auto;
  border: 1px solid var(--trace-code-border);
  border-radius: 8px;
  background: var(--trace-code-bg);
  color: var(--trace-code-text);
  font-size: 12px;
  line-height: 1.48;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.insight-block-rows {
  display: grid;
  grid-template-columns: minmax(86px, auto) minmax(0, 1fr);
  gap: 6px 10px;
  margin: 8px 0 0;
  font-size: 12px;
}

.insight-block-rows dt {
  color: var(--muted);
}

.insight-block-rows dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}

.insight-item-controls {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin: 8px 0 0;
  color: var(--muted);
  font-size: 11px;
  font-weight: 700;
}

.insight-item-controls label {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.insight-item-controls input {
  width: 64px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--trace-insight-input-border);
  border-radius: 7px;
  background: var(--trace-insight-input-bg);
  color: var(--text);
  font-size: 12px;
  font-weight: 700;
}

.insight-item-controls input:focus {
  outline: 2px solid var(--trace-insight-focus-outline);
  border-color: var(--teal);
}

.insight-item-list {
  display: grid;
  gap: 8px;
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
}

.insight-item-list li {
  min-width: 0;
  padding: 8px;
  border: 1px solid var(--trace-insight-item-border);
  border-radius: 8px;
  background: var(--trace-insight-item-bg);
}

.insight-item-list li > div {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
}

.insight-item-list strong {
  min-width: 0;
  font-size: 12px;
  overflow-wrap: anywhere;
}

.insight-item-list span {
  flex: 0 0 auto;
  color: var(--muted);
  font-size: 10px;
  font-weight: 800;
  text-transform: uppercase;
}
</style>
