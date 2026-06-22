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

          <ul v-if="block.items?.length" class="insight-item-list">
            <li v-for="item in block.items" :key="item.id">
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
  border: 1px solid #bdd7d2;
  border-radius: 8px;
  background: #f6fbfa;
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
  border: 1px solid #c8d7d5;
  border-left: 4px solid var(--teal);
  border-radius: 8px;
  background: #fbfcfc;
  box-shadow: 0 7px 18px rgba(15, 23, 42, 0.06);
}

.insight-block.tone-tools {
  border-left-color: var(--amber);
  background: #fffaf0;
}

.insight-block.tone-reasoning {
  border-left-color: #3158a3;
  background: #f5f7fc;
}

.insight-block.tone-response {
  border-left-color: #16a34a;
  background: #f4fbf6;
}

.insight-block.tone-context {
  border-left-color: #64748b;
  background: #f8fafc;
}

.insight-block.tone-http {
  border-left-color: #0f766e;
  background: #f3fbf9;
}

.insight-block.tone-status {
  border-left-color: #b45309;
  background: #fff8ed;
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
  border-color: rgba(180, 83, 9, 0.55);
  background: rgba(251, 191, 36, 0.22);
  transform: translateX(1px);
}

.insight-block-toggle:focus-visible {
  outline: 2px solid rgba(15, 118, 110, 0.35);
  outline-offset: 4px;
}

.insight-block-chevron {
  box-sizing: content-box;
  padding: 2px;
  border: 1px solid rgba(180, 83, 9, 0.28);
  border-radius: 5px;
  color: #92400e;
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
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 8px;
  background: #101819;
  color: #e6f2ef;
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
  border: 1px solid rgba(15, 23, 42, 0.08);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.7);
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
