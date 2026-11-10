<template>
  <details class="cp-rollback-history" @click.stop @keydown.enter.stop @keydown.space.stop>
    <summary class="cp-rollback" :title="t('rollbackHistory')">
      <RotateCcw :size="12" aria-hidden="true" />
      {{ t('restoredHere', { time: formatClock(restore.operation.finished_at) }) }}
      <ChevronRight class="cp-rollback-chevron" :size="12" aria-hidden="true" />
    </summary>
    <section class="cp-rollback-content">
      <h3>{{ t('rollbackHistory') }}</h3>
      <ol v-if="restore.discarded.length" class="cp-rollback-list">
        <li v-for="row in restore.discarded" :key="row.key">
          <button
            type="button"
            class="cp-rollback-entry"
            :class="{ selected: row.key === selectedKey }"
            :data-key="row.key"
            @click="$emit('select', row.key)"
          >
            <span class="cp-rollback-node" aria-hidden="true">
              <template v-if="row.type === 'checkpoint'">{{ row.checkpoint.checkpoint_id }}</template>
              <RotateCcw v-else-if="row.type === 'restore'" :size="12" />
              <Save v-else :size="12" />
            </span>
            <span>
              <strong>{{ title(row) }}</strong>
              <small>{{ formatClock(row.operation.finished_at ?? row.operation.requested_at) }}</small>
            </span>
          </button>
          <CheckpointRollback
            v-for="nested in row.restores"
            :key="nested.key"
            :restore="nested"
            :selected-key="selectedKey"
            @select="(key) => $emit('select', key)"
          />
        </li>
      </ol>
      <p v-else>{{ t('rollbackEmpty') }}</p>
      <button type="button" class="cp-rollback-result" @click="$emit('select', restore.key)">
        {{ t('restoreResult') }}
      </button>
    </section>
  </details>
</template>

<script setup>
import { ChevronRight, RotateCcw, Save } from '@lucide/vue';

import { useModuleLocale } from '../../../locale';
import { formatClock, operationStatus } from './model';
import strings from './locale';

const props = defineProps({
  restore: { type: Object, required: true },
  selectedKey: { type: String, default: null },
});
defineEmits(['select']);
const { t } = useModuleLocale(strings);

function title(row) {
  if (row.type === 'checkpoint') return t('pointName', { id: row.checkpoint.checkpoint_id });
  if (row.type === 'restore') return t('restoreTo', { id: row.operation.target_checkpoint });
  return t(operationStatus(row.operation) === 'running' ? 'saveRunning' : 'saveFailed');
}
</script>

<style scoped>
.cp-rollback-history { margin-top: 5px; }
.cp-rollback {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 5px;
  border-radius: var(--ui-radius-sm);
  background: var(--trace-interactive-bg);
  color: var(--trace-interactive-text);
  font-size: var(--ui-font-xs);
  list-style: none;
  cursor: pointer;
}
.cp-rollback::-webkit-details-marker { display: none; }
.cp-rollback-history[open] .cp-rollback-chevron { transform: rotate(90deg); }
.cp-rollback-content {
  margin-top: 8px;
  padding: 10px;
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-soft);
  cursor: default;
}
.cp-rollback-content h3, .cp-rollback-content p {
  margin: 0 0 8px;
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-medium);
}
.cp-rollback-list { margin: 0; padding: 0; list-style: none; }
.cp-rollback-list li + li { margin-top: 4px; }
.cp-rollback-list .cp-rollback-history { margin-left: 38px; }
.cp-rollback-entry {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 6px;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-sm);
  background: transparent;
  color: var(--ui-text);
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.cp-rollback-entry.selected { border-color: var(--ui-border); background: var(--ui-surface-raised); }
.cp-rollback-entry strong, .cp-rollback-entry small { display: block; font-size: var(--ui-font-xs); }
.cp-rollback-entry small { margin-top: 2px; color: var(--ui-muted); }
.cp-rollback-node {
  display: flex;
  align-items: center;
  justify-content: center;
  flex: 0 0 24px;
  height: 24px;
  border: 1px solid var(--ui-border);
  border-radius: 50%;
  color: var(--ui-muted);
  background: var(--ui-surface-raised);
  font: 600 var(--ui-font-xs) var(--ui-value-font);
}
.cp-rollback-result {
  margin-top: 8px;
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--trace-interactive-text);
  font: inherit;
  font-size: var(--ui-font-xs);
  text-decoration: underline;
  text-underline-offset: 2px;
  cursor: pointer;
}
.cp-rollback:hover, .cp-rollback-entry:hover { background: var(--ui-accent-muted); }
.cp-rollback:focus-visible, .cp-rollback-entry:focus-visible, .cp-rollback-result:focus-visible {
  outline: 2px solid var(--ui-focus-ring-color);
  outline-offset: 2px;
}
</style>
