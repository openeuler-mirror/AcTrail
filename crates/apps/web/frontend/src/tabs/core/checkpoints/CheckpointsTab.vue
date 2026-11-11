<template>
  <section class="tab-detail-layout checkpoint-layout" :class="{ 'detail-open': Boolean(selectedRow) }">
    <section class="tab-detail-main checkpoint-main">
      <p v-if="error" class="cp-message error">{{ error }}</p>

      <div v-if="listing && !listing.enabled" class="cp-empty">
        <strong>{{ t('disabledTitle') }}</strong>
        <span>{{ t('disabled') }}</span>
      </div>
      <div v-else-if="listing && !listing.trace" class="cp-empty">
        <strong>{{ t('noHistoryTitle') }}</strong>
        <span>{{ t('noHistory') }}</span>
      </div>

      <template v-else-if="listing">
        <header class="cp-header">
          <div>
            <h2>{{ t('title') }}</h2>
            <div class="cp-summary">
              <span class="trace-state" :class="ended ? 'trace-state-completed' : 'trace-state-running'">
                {{ ended ? t('traceEnded') : t('traceRunning') }}
              </span>
              <span>{{ t('restorableCount', { count: restorable }) }}</span>
            </div>
          </div>
          <div class="cp-toolbar">
            <button type="button" class="cp-button" :title="t('saveHint')" :disabled="busy || ended || listing.trace.disconnected_at != null" @click="command('save')">
              <Plus :size="14" :stroke-width="1.8" aria-hidden="true" />
              {{ t('save') }}
            </button>
          </div>
        </header>

        <p v-if="listing.trace && listing.runtime == null" class="cp-message" role="status">
          {{ t('pauseUnknown') }}
        </p>

        <section v-if="pending" class="cp-notice" role="status">
          <Loader2 :size="17" class="cp-spin" />
          <div>
            <strong>{{ t(pending.phase === 'waiting_boundary' ? 'pendingWaiting' : 'pendingRunning', { id: pending.target_checkpoint }) }}</strong>
            <small>{{ t('pendingHint') }}</small>
          </div>
          <button
            v-if="pending.phase === 'waiting_boundary'"
            type="button"
            class="cp-button"
            :disabled="busy"
            @click="command('force', { operation: pending.operation_id })"
          >
            <Zap :size="14" />
            {{ t('force') }}
          </button>
        </section>
        <section v-else-if="stopped" class="cp-notice warning" role="alert">
          <AlertTriangle :size="17" />
          <div>
            <strong>{{ t('heldTitle') }}</strong>
            <small>{{ t('heldHint') }}</small>
          </div>
          <button type="button" class="cp-button" :disabled="busy || listing.runtime?.busy" @click="command('resume')">
            <Play :size="14" />
            {{ t('resume') }}
          </button>
        </section>

        <p v-if="message" class="cp-message" :class="{ error: message.error }" role="status">{{ message.text }}</p>

        <CheckpointHistory
          ref="history"
          :graph="graph"
          :selected-key="selectedKey"
          :busy="busy"
          :restore-pending="Boolean(pending)"
          @select="(key) => { selectedKey = key; }"
          @restore="restore"
          @open-llm-call="openLlmCall"
        />
      </template>
    </section>

    <CheckpointDetail
      v-if="selectedRow"
      :row="selectedRow"
      :trace="listing.trace"
      :busy="busy"
      :restore-pending="Boolean(pending)"
      :pending-target="pending?.target_checkpoint"
      @close="selectedKey = null"
      @restore="restore"
      @delete="remove"
      @force="(operation) => command('force', { operation: operation.operation_id })"
      @open-llm-call="openLlmCall"
      @select-checkpoint="selectCheckpoint"
    />
  </section>
</template>

<script setup>
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue';
import { AlertTriangle, Loader2, Play, Plus, Zap } from '@lucide/vue';

import { sendCheckpointCommand } from '../../../api';
import { useModuleLocale } from '../../../locale';
import CheckpointDetail from './CheckpointDetail.vue';
import CheckpointHistory from './CheckpointHistory.vue';
import { historyGraph, initialRow } from './graph';
import { pendingRestore, restorableCount } from './model';
import { llmCallPath } from './navigation';
import { useCheckpointHistory } from './useCheckpointHistory';
import strings from './locale';
import './checkpoints.css';

const props = defineProps({
  traceKey: {
    type: [String, Number],
    default: null,
  },
  /** `{ checkpointId, operationId, nonce }` from an ActionTree marker. */
  checkpointFocus: {
    type: Object,
    default: null,
  },
  /** Bumped by the header's refresh button. */
  refreshNonce: {
    type: Number,
    default: 0,
  },
});
const emit = defineEmits(['open-action-tree']);

const { t } = useModuleLocale(strings);
const { listing, error, refresh, stop } = useCheckpointHistory(() => props.traceKey);
const graph = computed(() => historyGraph(listing.value));
const rows = computed(() => graph.value.rows);
const records = computed(() => graph.value.records);
const history = ref(null);
const ended = computed(() => listing.value?.trace?.ended_at != null);
const restorable = computed(() => restorableCount(listing.value));
const pending = computed(() => {
  const current = listing.value;
  const operation = pendingRestore(current);
  // A completed worker may still have an operation result waiting for a catalog write.
  if (operation?.phase !== 'waiting_boundary' && current?.runtime && !current.runtime.busy) return null;
  return operation;
});
const stopped = computed(() => listing.value?.runtime?.held === true);
const selectedKey = ref(null);
let selectionInitialized = false;
let appliedFocus = null;
const selectedRow = computed(() => records.value.find((row) => row.key === selectedKey.value) ?? null);
const busy = ref(false);
const message = ref(null);
let messageTimer = null;

watch(
  () => props.traceKey,
  () => {
    selectedKey.value = null;
    selectionInitialized = false;
    appliedFocus = null;
    message.value = null;
  },
);

watch(() => props.refreshNonce, refresh);

// Follow a rollback when it removes the selected row; explicit links can open any old record.
watch(rows, async (current, previous = []) => {
  const selected = (row) => row.key === selectedKey.value || row.restores?.some((restore) => restore.key === selectedKey.value);
  if (selectionInitialized && !selectedKey.value) return;
  if (selectedKey.value && current.some(selected)) return;
  if (selectedRow.value && !previous.some(selected)) return;
  selectedKey.value = initialRow(current)?.key ?? null;
  selectionInitialized = selectedKey.value != null;
  await nextTick();
  history.value?.$el.querySelector('.cp-row.selected')?.scrollIntoView({ block: 'nearest' });
});

watch(
  () => [props.checkpointFocus, records.value],
  ([focus]) => {
    if (!focus || focus === appliedFocus) return;
    if (focus.operationId != null && records.value.some((row) => row.operation.operation_id === focus.operationId)) {
      selectedKey.value = `operation-${focus.operationId}`;
    } else if (focus.checkpointId != null) {
      if (!selectCheckpoint(focus.checkpointId)) return;
    } else {
      return;
    }
    appliedFocus = focus;
    selectionInitialized = true;
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  stop();
  clearTimeout(messageTimer);
});

function selectCheckpoint(checkpointId) {
  const row = records.value.find((candidate) => candidate.type === 'checkpoint' && candidate.checkpoint.checkpoint_id === checkpointId);
  if (row) selectedKey.value = row.key;
  return Boolean(row);
}

function restore(checkpoint) {
  const question = t('confirmRestore', { trace: props.traceKey, id: checkpoint.checkpoint_id });
  if (window.confirm(question)) command('restore', { checkpoint: checkpoint.checkpoint_id });
}

function remove(checkpoint) {
  if (listing.value?.trace?.disconnected_at != null) {
    command('disconnect');
  } else if (window.confirm(t('confirmDelete', { id: checkpoint.checkpoint_id }))) {
    command('delete', { checkpoint: checkpoint.checkpoint_id });
  }
}

async function command(name, params = {}) {
  busy.value = true;
  try {
    const reply = await sendCheckpointCommand(props.traceKey, name, params);
    showMessage(reply.operation_id != null ? t('accepted', { id: reply.operation_id }) : t('acceptedPlain'));
    if (reply.operation_id != null) selectedKey.value = `operation-${reply.operation_id}`;
  } catch (err) {
    showMessage(String(err.message ?? err), true);
  } finally {
    busy.value = false;
    await refresh();
  }
}

function showMessage(text, isError = false) {
  clearTimeout(messageTimer);
  message.value = { text, error: isError };
  if (!isError) messageTimer = setTimeout(() => { message.value = null; }, 4000);
}

async function openLlmCall(operation) {
  try {
    const path = await llmCallPath(props.traceKey, operation.operation_id);
    if (path) emit('open-action-tree', { path });
  } catch (err) {
    showMessage(String(err.message ?? err), true);
  }
}
</script>
