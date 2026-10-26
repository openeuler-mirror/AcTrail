<template>
  <section class="plugin-config-panel">
    <button class="plugin-config-toggle" type="button" :disabled="loading" @click="toggle">
      <span class="plugin-config-toggle-title">
        <Settings2 :size="17" aria-hidden="true" />
        <span>
          <strong>{{ panelTitle }}</strong>
          <small>{{ panelSubtitle }}</small>
        </span>
      </span>
      <span class="plugin-config-toggle-state">
        <span
          v-if="document"
          class="plugin-config-access"
          :class="document.editable ? 'editable' : 'readonly'"
        >
          <Pencil v-if="document.editable" :size="13" aria-hidden="true" />
          <LockKeyhole v-else :size="13" aria-hidden="true" />
          {{ document.editable ? t('configPanel.editable') : t('configPanel.readOnly') }}
        </span>
        <ChevronDown :size="17" :class="{ rotated: opened }" aria-hidden="true" />
      </span>
    </button>

    <div v-if="opened" class="plugin-config-body">
      <div class="plugin-config-heading">
        <div>
          <span>{{ t('configPanel.instance') }}</span>
          <code>{{ instanceId }}</code>
        </div>
        <p v-if="document?.editable">
          {{ t('configPanel.lockedNote') }}
        </p>
        <p v-else-if="document">{{ t('configPanel.inspectionOnly') }}</p>
      </div>

      <p v-if="loading" class="plugin-config-note">{{ t('configPanel.loading') }}</p>
      <p v-else-if="error" class="plugin-config-error">{{ error }}</p>
      <template v-else-if="document">
        <section v-if="pendingDocument" class="plugin-config-conflict" role="alert">
          <div>
            <strong>{{ t('configPanel.conflictTitle') }}</strong>
            <span>{{ t('configPanel.conflictBody') }}</span>
          </div>
          <button type="button" @click="reloadPendingDocument">{{ t('configPanel.reload') }}</button>
        </section>
        <PluginConfigItem
          name="configuration"
          :schema="document.schema ?? {}"
          :model-value="draft"
          :editable="document.editable"
          @update:model-value="changeDraft"
        />

        <div class="plugin-config-actions">
          <span class="plugin-config-validation-state" :class="validationState.className">
            <CheckCircle2 v-if="validationState.valid" :size="14" aria-hidden="true" />
            {{ validationState.label }}
          </span>
          <button
            type="button"
            :disabled="!document.editable || testing || updating"
            @click="testConfiguration"
          >
            {{ testing ? t('configPanel.testing') : t('configPanel.test') }}
          </button>
          <button
            class="primary"
            type="button"
            :disabled="!canUpdate || updating"
            @click="updateConfiguration"
          >
            {{ updating ? t('configPanel.updating') : t('configPanel.update') }}
          </button>
        </div>

        <ul v-if="validation && !validation.valid" class="plugin-config-errors">
          <li v-for="message in validation.errors" :key="message">{{ message }}</li>
        </ul>
        <p v-if="updated" class="plugin-config-valid">
          {{ t('configPanel.updated') }}
        </p>
      </template>
    </div>
  </section>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { CheckCircle2, ChevronDown, LockKeyhole, Pencil, Settings2 } from '@lucide/vue';

import {
  readRuntimePluginConfig,
  updateRuntimePluginConfig,
  validateRuntimePluginConfig,
} from '../../api';
import PluginConfigItem from './PluginConfigItem.vue';
import { useModuleLocale } from '../../locale';
import strings from './locale';

const { t } = useModuleLocale(strings);

const props = defineProps({
  instanceId: { type: String, required: true },
  pluginId: { type: String, default: '' },
  refreshNonce: { type: Number, default: 0 },
});

const emit = defineEmits(['updated']);
const opened = ref(false);
const loading = ref(false);
const testing = ref(false);
const updating = ref(false);
const document = ref(null);
const pendingDocument = ref(null);
const draft = ref(null);
const originalSnapshot = ref('');
const validatedSnapshot = ref('');
const validation = ref(null);
const error = ref('');
const updated = ref(false);
let activeConfigLoad = null;

const isLlmTurnAnomaly = computed(() => props.pluginId === 'actrail.llm-turn-anomaly');
const panelTitle = computed(() => t(isLlmTurnAnomaly.value ? 'configPanel.titleLlmAlert' : 'configPanel.titleDefault'));
const panelSubtitle = computed(() => isLlmTurnAnomaly.value
  ? t('configPanel.subtitleLlmAlert')
  : t('configPanel.subtitleDefault'));

const draftSnapshot = computed(() => JSON.stringify(draft.value));
const canUpdate = computed(() => Boolean(
  document.value?.editable
    && validation.value?.valid
    && !pendingDocument.value
    && validatedSnapshot.value === draftSnapshot.value
    && originalSnapshot.value !== draftSnapshot.value,
));
const dirty = computed(() => originalSnapshot.value !== draftSnapshot.value);
const validationState = computed(() => {
  if (!document.value?.editable) {
    return { label: t('configPanel.stateReadOnly'), className: 'readonly', valid: false };
  }
  if (validation.value && !validation.value.valid) {
    return { label: t('configPanel.stateHasErrors'), className: 'invalid', valid: false };
  }
  if (validation.value?.valid && validatedSnapshot.value === draftSnapshot.value) {
    return {
      label: dirty.value ? t('configPanel.stateTestPassed') : t('configPanel.stateCurrentValid'),
      className: 'valid',
      valid: true,
    };
  }
  return {
    label: dirty.value ? t('configPanel.stateMustTest') : t('configPanel.stateNoChanges'),
    className: dirty.value ? 'pending' : 'idle',
    valid: false,
  };
});

watch(() => props.refreshNonce, () => {
  if (document.value) refreshConfiguration();
});

async function toggle() {
  opened.value = !opened.value;
  if (opened.value && !document.value) {
    await loadConfiguration();
  }
}

async function loadConfiguration() {
  const loadToken = Symbol('plugin-config-load');
  activeConfigLoad = loadToken;
  loading.value = true;
  error.value = '';
  try {
    const nextDocument = await readRuntimePluginConfig(props.instanceId);
    if (activeConfigLoad === loadToken) applyDocument(nextDocument);
  } catch (err) {
    if (activeConfigLoad === loadToken) error.value = String(err.message ?? err);
  } finally {
    if (activeConfigLoad === loadToken) loading.value = false;
  }
}

async function refreshConfiguration() {
  const loadToken = Symbol('plugin-config-refresh');
  activeConfigLoad = loadToken;
  loading.value = true;
  error.value = '';
  try {
    const nextDocument = await readRuntimePluginConfig(props.instanceId);
    if (activeConfigLoad !== loadToken) return;
    const nextSnapshot = JSON.stringify(nextDocument.config);
    if (dirty.value && nextSnapshot !== originalSnapshot.value) {
      pendingDocument.value = nextDocument;
      validation.value = null;
      validatedSnapshot.value = '';
      return;
    }
    if (!dirty.value) {
      applyDocument(nextDocument);
    }
  } catch (err) {
    if (activeConfigLoad === loadToken) error.value = String(err.message ?? err);
  } finally {
    if (activeConfigLoad === loadToken) loading.value = false;
  }
}

function applyDocument(nextDocument) {
  document.value = nextDocument;
  pendingDocument.value = null;
  updated.value = false;
  draft.value = cloneJson(nextDocument.config);
  originalSnapshot.value = JSON.stringify(nextDocument.config);
  validatedSnapshot.value = '';
  validation.value = null;
}

function reloadPendingDocument() {
  if (pendingDocument.value) applyDocument(pendingDocument.value);
}

function changeDraft(value) {
  draft.value = value;
  validatedSnapshot.value = '';
  validation.value = null;
  updated.value = false;
}

async function testConfiguration() {
  testing.value = true;
  error.value = '';
  updated.value = false;
  try {
    const result = await validateRuntimePluginConfig(props.instanceId, draft.value);
    validation.value = result;
    validatedSnapshot.value = result.valid ? draftSnapshot.value : '';
  } catch (err) {
    validation.value = null;
    validatedSnapshot.value = '';
    error.value = String(err.message ?? err);
  } finally {
    testing.value = false;
  }
}

async function updateConfiguration() {
  updating.value = true;
  error.value = '';
  updated.value = false;
  try {
    applyDocument(await updateRuntimePluginConfig(props.instanceId, draft.value));
    updated.value = true;
    emit('updated');
  } catch (err) {
    error.value = String(err.message ?? err);
  } finally {
    updating.value = false;
  }
}

function cloneJson(value) {
  return value == null ? value : JSON.parse(JSON.stringify(value));
}
</script>

<style scoped>
.plugin-config-panel {
  display: grid;
  margin: 0 var(--ui-space-2xl) var(--ui-space-lg) calc(var(--ui-space-2xl) + var(--ui-space-lg));
  overflow: visible;
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface-soft);
  color: var(--ui-text);
  font-size: var(--ui-font-md);
}

.plugin-config-toggle {
  width: 100%;
  min-height: calc(var(--ui-control-height-lg) + var(--ui-space-md));
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-lg);
  padding: var(--ui-space-md) var(--ui-space-lg);
  border: 0;
  background: transparent;
  color: var(--ui-text);
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.plugin-config-toggle:hover {
  background: var(--ui-surface-bar);
}

.plugin-config-toggle:focus-visible,
.plugin-config-conflict button:focus-visible,
.plugin-config-actions button:focus-visible {
  outline: 2px solid var(--ui-accent);
  outline-offset: calc(-1 * var(--ui-space-xs));
}

.plugin-config-toggle-title,
.plugin-config-toggle-state,
.plugin-config-toggle-title > span {
  display: flex;
  align-items: center;
}

.plugin-config-toggle-title {
  gap: var(--ui-space-md);
}

.plugin-config-toggle-title > span {
  align-items: flex-start;
  flex-direction: column;
  gap: var(--ui-space-2xs);
}

.plugin-config-toggle-title strong {
  font-size: var(--ui-font-ui);
  font-weight: var(--ui-weight-medium);
}

.plugin-config-toggle-title small {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.plugin-config-toggle-state {
  gap: var(--ui-space-md);
}

.plugin-config-toggle-state > svg {
  color: var(--ui-muted);
  transition: transform 120ms ease;
}

.plugin-config-toggle-state > svg.rotated {
  transform: rotate(180deg);
}

.plugin-config-access {
  min-height: var(--ui-control-height-sm);
  display: inline-flex;
  align-items: center;
  gap: var(--ui-space-xs);
  padding: 0 var(--ui-space-sm);
  border: 1px solid var(--ui-border-strong);
  border-radius: 100vmax;
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-medium);
}

.plugin-config-access.editable {
  border-color: var(--ui-accent-soft);
  background: var(--ui-accent-muted);
  color: var(--ui-accent);
}

.plugin-config-access.readonly {
  background: var(--ui-surface-soft);
  color: var(--ui-muted);
}

.plugin-config-body {
  container: plugin-config / inline-size;
  min-width: 0;
  display: grid;
  gap: var(--ui-space-2xl);
  padding: var(--ui-space-2xl);
  border: 0;
  border-top: 1px solid var(--ui-border);
  border-radius: 0;
  background: var(--ui-surface);
  box-shadow: none;
}

.plugin-config-heading {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 2fr);
  align-items: center;
  gap: var(--ui-space-xl);
  padding-bottom: var(--ui-space-lg);
  border-bottom: 1px solid var(--ui-border);
}

.plugin-config-heading > div {
  display: grid;
  gap: var(--ui-space-2xs);
}

.plugin-config-heading span {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
  text-transform: uppercase;
}

.plugin-config-heading code {
  min-width: 0;
  overflow-wrap: anywhere;
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: var(--ui-font-sm);
}

.plugin-config-heading p {
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  text-align: right;
}

.plugin-config-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ui-space-md);
  justify-content: space-between;
  padding-top: var(--ui-space-lg);
  border-top: 1px solid var(--ui-border);
}

.plugin-config-actions button {
  min-height: var(--ui-control-height-md);
  padding: 0 var(--ui-space-lg);
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-soft);
  color: var(--ui-text);
  cursor: pointer;
  font: inherit;
  font-weight: var(--ui-weight-medium);
}

.plugin-config-actions button.primary {
  border-color: var(--ui-accent-soft);
  background: var(--ui-accent-muted);
  color: var(--ui-accent);
}

.plugin-config-toggle:disabled,
.plugin-config-actions button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

.plugin-config-note,
.plugin-config-error,
.plugin-config-valid {
  margin: 0;
}

.plugin-config-conflict {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-xl);
  padding: var(--ui-space-lg);
  border: 1px solid color-mix(in srgb, var(--ui-danger) 38%, var(--ui-border));
  border-radius: var(--ui-radius-md);
  background: color-mix(in srgb, var(--ui-danger) 8%, var(--ui-surface));
}

.plugin-config-conflict > div {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-xs);
}

.plugin-config-conflict strong {
  color: var(--ui-danger);
  font-size: var(--ui-font-ui);
}

.plugin-config-conflict span {
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.plugin-config-conflict button {
  min-height: var(--ui-control-height-md);
  flex: 0 0 auto;
  padding: 0 var(--ui-space-lg);
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  cursor: pointer;
  font: inherit;
  font-weight: var(--ui-weight-medium);
}

.plugin-config-validation-state {
  display: inline-flex;
  align-items: center;
  gap: var(--ui-space-xs);
  margin-right: auto;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.plugin-config-validation-state.valid {
  color: var(--ui-accent);
}

.plugin-config-validation-state.invalid {
  color: var(--ui-danger);
}

@container plugin-config (max-width: 42rem) {
  .plugin-config-heading {
    grid-template-columns: minmax(0, 1fr);
  }

  .plugin-config-heading p {
    text-align: left;
  }

  .plugin-config-conflict {
    align-items: stretch;
    flex-direction: column;
  }
}

@media (max-width: 47.5rem) {
  .plugin-config-panel {
    margin-right: var(--ui-space-xl);
    margin-left: var(--ui-space-xl);
  }
}

.plugin-config-error,
.plugin-config-errors {
  color: var(--ui-danger);
}

.plugin-config-errors {
  margin: 0;
  padding-left: var(--ui-space-xl);
}

.plugin-config-valid {
  color: var(--ui-accent);
  font-size: var(--ui-font-sm);
}
</style>
