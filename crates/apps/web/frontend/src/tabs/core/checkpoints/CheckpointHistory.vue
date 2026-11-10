<template>
  <ol class="cp-history cp-rows">
    <li
      v-for="(row, index) in graph.rows"
      :key="row.key"
      class="cp-row"
      :class="{
        selected: row.key === selectedKey || row.restores?.some((restore) => restore.key === selectedKey), muted: isMuted(row),
        connected: index > 0,
        pending: ['waiting', 'running'].includes(operationStatus(row.operation)),
        failed: ['failed', 'unknown'].includes(operationStatus(row.operation)),
      }"
      :data-key="row.key"
      tabindex="0"
      @click="$emit('select', row.key)"
      @keydown.enter.self.prevent="$emit('select', row.key)"
      @keydown.space.self.prevent="$emit('select', row.key)"
    >
      <span class="cp-node" aria-hidden="true">
        <template v-if="row.type === 'checkpoint'">{{ row.checkpoint.checkpoint_id }}</template>
        <RotateCcw v-else-if="row.type === 'restore'" :size="14" />
        <Save v-else :size="14" />
      </span>
      <div class="cp-row-text">
        <div class="cp-title">
          <strong>{{ title(row) }}</strong>
          <span v-for="chip in chips(row)" :key="chip.label" class="cp-chip" :class="chip.tone">{{ chip.label }}</span>
        </div>
        <div class="cp-meta">{{ meta(row) }}</div>
        <CheckpointRollback
          v-for="restore in row.restores"
          :key="restore.key"
          :restore="restore"
          :selected-key="selectedKey"
          @select="(key) => $emit('select', key)"
        />
      </div>
      <div class="cp-row-actions">
        <button
          v-if="row.operation.llm_call_action_id"
          type="button"
          class="cp-button quiet cp-icon-button"
          :title="t('llmCallHint')"
          :aria-label="t('llmCall')"
          @click.stop="$emit('open-llm-call', row.operation)"
        >
          <MessagesSquare :size="15" />
        </button>
        <button
          v-if="row.type === 'checkpoint' && checkpointStatus(row.checkpoint) === 'restorable' && !restorePending"
          type="button"
          class="cp-button"
          :disabled="busy"
          @click.stop="$emit('restore', row.checkpoint)"
        >
          <RotateCcw :size="13" />
          {{ t('restore') }}
        </button>
      </div>
    </li>
  </ol>
</template>

<script setup>
import { MessagesSquare, RotateCcw, Save } from '@lucide/vue';

import { useModuleLocale } from '../../../locale';
import CheckpointRollback from './CheckpointRollback.vue';
import { checkpointStatus, formatClock, operationStatus } from './model';
import strings from './locale';

const props = defineProps({
  graph: { type: Object, required: true },
  selectedKey: { type: String, default: null },
  busy: Boolean,
  restorePending: Boolean,
});
defineEmits(['select', 'restore', 'open-llm-call']);
const { t } = useModuleLocale(strings);

function isMuted(row) {
  return row.type === 'checkpoint' && checkpointStatus(row.checkpoint) !== 'restorable';
}

function title(row) {
  if (row.type === 'restore') return t('restoreTo', { id: row.operation.target_checkpoint });
  if (row.type === 'checkpoint') return t(`triggers.${row.operation.trigger}`);
  return operationStatus(row.operation) === 'running' ? t('saveRunning') : t('saveFailed');
}

function meta(row) {
  const time = formatClock(row.operation.finished_at ?? row.operation.requested_at);
  const id = row.type === 'checkpoint' ? t('pointName', { id: row.checkpoint.checkpoint_id }) : `operation-${row.operation.operation_id}`;
  return `${id} · ${time}`;
}

function chips(row) {
  if (row.type === 'checkpoint') {
    const status = checkpointStatus(row.checkpoint);
    return status === 'restorable' ? [] : [{ label: t(`status.${status}`), tone: '' }];
  }
  const status = operationStatus(row.operation);
  if (row.type === 'save' && status === 'running') return [];
  return status === 'succeeded' ? [] : [{ label: t(`status.${status}`), tone: status }];
}
</script>
