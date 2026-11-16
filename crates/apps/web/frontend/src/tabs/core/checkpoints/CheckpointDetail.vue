<template>
  <aside class="detail-panel checkpoint-detail">
    <div class="detail-header">
      <div>
        <span>{{ checkpoint ? t('rows.checkpoint') : `operation-${operation.operation_id}` }}</span>
        <h2>{{ checkpoint ? t('pointName', { id: checkpoint.checkpoint_id }) : title }}</h2>
      </div>
      <button type="button" class="cp-button quiet cp-icon-button" aria-label="Close" @click="$emit('close')">
        <X :size="16" />
      </button>
    </div>

    <div v-if="checkpoint || operation.phase === 'waiting_boundary' || operation.llm_call_action_id" class="cp-detail-actions">
      <template v-if="checkpoint">
        <button
          type="button"
          class="cp-button primary"
          :disabled="busy || restorePending || status !== 'restorable'"
          :title="restorePending ? t('restorePending') : undefined"
          @click="$emit('restore', checkpoint)"
        >
          <RotateCcw :size="14" />
          {{ t('restore') }}
        </button>
        <button
          v-if="operation.llm_call_action_id"
          type="button"
          class="cp-button"
          :title="t('llmCallHint')"
          @click="$emit('open-llm-call', operation)"
        >
          <MessagesSquare :size="14" />
          {{ t('llmCall') }}
        </button>
        <button
          type="button"
          class="cp-button danger"
          :disabled="busy || !canDelete(checkpoint, { target_checkpoint: pendingTarget }, trace)"
          @click="$emit('delete', checkpoint)"
        >
          <Trash2 :size="14" />
          {{ t(checkpoint.release === 'failed' ? 'retryDelete' : 'delete') }}
        </button>
      </template>
      <template v-else>
        <button
          v-if="operation.phase === 'waiting_boundary'"
          type="button"
          class="cp-button primary"
          :disabled="busy"
          @click="$emit('force', operation)"
        >
          <Zap :size="14" />
          {{ t('force') }}
        </button>
        <button
          v-if="operation.llm_call_action_id"
          type="button"
          class="cp-button"
          :title="t('llmCallHint')"
          @click="$emit('open-llm-call', operation)"
        >
          <MessagesSquare :size="14" />
          {{ t('llmCall') }}
        </button>
      </template>
    </div>

    <dl class="detail-rows">
      <template v-if="checkpoint">
        <dt>{{ t('rows.trigger') }}</dt>
        <dd>{{ t(`triggers.${operation.trigger}`) }}</dd>
        <dt>{{ t('rows.status') }}</dt>
        <dd>
          <span class="cp-chip" :class="status === 'restorable' ? 'succeeded' : ''">{{ t(`status.${status}`) }}</span>
          <small v-if="status === 'invalidated'" class="cp-detail-note">{{ t('invalidatedBy', { id: checkpoint.invalidated_by }) }}</small>
          <small v-if="status === 'deleted'" class="cp-detail-note">{{ t(`release.${checkpoint.release}`) }}</small>
        </dd>
        <dt>{{ t('rows.created') }}</dt>
        <dd>{{ formatTime(checkpoint.created_at) }}</dd>
        <dt>{{ t('rows.operation') }}</dt>
        <dd class="cp-mono">operation-{{ operation.operation_id }}</dd>
      </template>
      <template v-else>
        <dt>{{ t('rows.status') }}</dt>
        <dd><span class="cp-chip" :class="opStatus">{{ t(`status.${opStatus}`) }}</span></dd>
        <template v-if="operation.target_checkpoint != null">
          <dt>{{ t('rows.target') }}</dt>
          <dd>
            <button type="button" class="cp-link" @click="$emit('select-checkpoint', operation.target_checkpoint)">
              {{ t('pointName', { id: operation.target_checkpoint }) }}
            </button>
          </dd>
        </template>
        <dt>{{ t('rows.requested') }}</dt>
        <dd>{{ formatTime(operation.requested_at) }}</dd>
        <template v-if="operation.started_at">
          <dt>{{ t('rows.started') }}</dt>
          <dd>{{ formatTime(operation.started_at) }}</dd>
        </template>
        <template v-if="operation.finished_at">
          <dt>{{ t('rows.finished') }}</dt>
          <dd>{{ formatTime(operation.finished_at) }}</dd>
        </template>
      </template>
    </dl>

    <section v-if="checkpoint && scopes.length" class="detail-section">
      <h3>{{ t('scopes') }}</h3>
      <ul class="cp-scope-list">
        <li v-for="scope in scopes" :key="scope.id">
          <strong>{{ scope.id }}</strong>
          <small>{{ scope.backend_instance }}</small>
          <span class="cp-mono cp-root-line">{{ scope.root }}</span>
          <span v-if="scope.excludes.length" class="cp-excludes">{{ t('excludes', { paths: scope.excludes.join(', ') }) }}</span>
        </li>
      </ul>
    </section>

    <p v-if="!checkpoint && operation.failure" class="detail-error">{{ operation.failure.code }}: {{ operation.failure.message }}</p>
  </aside>
</template>

<script setup>
import { computed } from 'vue';
import { MessagesSquare, RotateCcw, Trash2, X, Zap } from '@lucide/vue';

import { useModuleLocale } from '../../../locale';
import { canDelete, checkpointStatus, formatTime, operationStatus } from './model';
import strings from './locale';

const props = defineProps({
  /** A row of `historyRows`: the operation and, for a save or restore, its checkpoint. */
  row: {
    type: Object,
    required: true,
  },
  trace: {
    type: Object,
    required: true,
  },
  busy: Boolean,
  restorePending: Boolean,
  pendingTarget: { type: Number, default: null },
});
defineEmits(['close', 'restore', 'delete', 'force', 'open-llm-call', 'select-checkpoint']);

const { t } = useModuleLocale(strings);
const operation = computed(() => props.row.operation);
// A restore row is about the operation; its target is one click away.
const checkpoint = computed(() => (props.row.type === 'checkpoint' ? props.row.checkpoint : null));
const status = computed(() => checkpoint.value && checkpointStatus(checkpoint.value));
const opStatus = computed(() => operationStatus(operation.value));
const title = computed(() => {
  if (props.row.type === 'restore') return t('restoreTo', { id: operation.value.target_checkpoint });
  return opStatus.value === 'running' ? t('saveRunning') : t('saveFailed');
});
const scopes = computed(() => props.trace.scopes ?? []);
</script>

<style scoped>
.checkpoint-detail {
  font-size: var(--ui-font-md);
}

.cp-detail-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding: 14px 0 2px;
}

.cp-detail-note {
  display: block;
  margin-top: 4px;
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.cp-mono {
  font-family: var(--ui-value-font);
  font-size: var(--ui-font-sm);
  overflow-wrap: anywhere;
}

.cp-link {
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--trace-interactive-text);
  font-family: var(--ui-value-font);
  font-size: var(--ui-font-sm);
  text-decoration: underline;
  text-underline-offset: 2px;
  cursor: pointer;
}

.cp-scope-list {
  display: grid;
  gap: 8px;
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
}

.cp-scope-list li {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 2px 10px;
  padding: 9px 10px;
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-raised);
}

.cp-scope-list li strong {
  font-size: var(--ui-font-sm);
}

.cp-scope-list li .cp-chip {
  justify-self: start;
}

.cp-scope-list li small {
  justify-self: end;
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.cp-root-line,
.cp-excludes {
  grid-column: 1 / -1;
}

.cp-excludes {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.cp-scope-list li .detail-error {
  grid-column: 1 / -1;
}
</style>
