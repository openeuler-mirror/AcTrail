<template>
  <section class="plugin-config-panel">
    <button class="plugin-config-toggle" type="button" :disabled="loading" @click="toggle">
      <span class="plugin-config-toggle-title">
        <Settings2 :size="17" aria-hidden="true" />
        <span>
          <strong>Configuration</strong>
          <small>Schema-driven runtime settings</small>
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
          {{ document.editable ? 'Editable' : 'Read only' }}
        </span>
        <ChevronDown :size="17" :class="{ rotated: opened }" aria-hidden="true" />
      </span>
    </button>

    <div v-if="opened" class="plugin-config-body">
      <div class="plugin-config-heading">
        <div>
          <span>Instance</span>
          <code>{{ instanceId }}</code>
        </div>
        <p v-if="document?.editable">
          Locked fields are marked explicitly and enforced by the plugin schema.
        </p>
        <p v-else-if="document">This plugin exposes configuration for inspection only.</p>
      </div>

      <p v-if="loading" class="plugin-config-note">Loading configuration…</p>
      <p v-else-if="error" class="plugin-config-error">{{ error }}</p>
      <template v-else-if="document">
        <section v-if="pendingDocument" class="plugin-config-conflict" role="alert">
          <div>
            <strong>Runtime configuration changed</strong>
            <span>
              A plugin command changed the runtime configuration. Your unsaved edits are preserved but cannot be
              submitted until you reload the current plugin configuration.
            </span>
          </div>
          <button type="button" @click="reloadPendingDocument">Reload runtime configuration</button>
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
            {{ testing ? 'Testing…' : 'Test configuration' }}
          </button>
          <button
            class="primary"
            type="button"
            :disabled="!canUpdate || updating"
            @click="updateConfiguration"
          >
            {{ updating ? 'Updating…' : 'Update configuration' }}
          </button>
        </div>

        <ul v-if="validation && !validation.valid" class="plugin-config-errors">
          <li v-for="message in validation.errors" :key="message">{{ message }}</li>
        </ul>
        <p v-if="updated" class="plugin-config-valid">Runtime configuration updated.</p>
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

const props = defineProps({
  instanceId: { type: String, required: true },
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
    return { label: 'Read-only configuration', className: 'readonly', valid: false };
  }
  if (validation.value && !validation.value.valid) {
    return { label: 'Configuration has errors', className: 'invalid', valid: false };
  }
  if (validation.value?.valid && validatedSnapshot.value === draftSnapshot.value) {
    return {
      label: dirty.value ? 'Test passed — ready to update' : 'Current configuration is valid',
      className: 'valid',
      valid: true,
    };
  }
  return {
    label: dirty.value ? 'Changes must be tested before update' : 'No uncommitted changes',
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
  margin: 0 var(--stats-space-2xl) var(--stats-space-lg) calc(var(--stats-space-2xl) + var(--stats-space-lg));
  overflow: visible;
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface-soft);
  color: var(--stats-text);
  font-size: var(--stats-font-md);
}

.plugin-config-toggle {
  width: 100%;
  min-height: calc(var(--stats-control-height-lg) + var(--stats-space-md));
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-md) var(--stats-space-lg);
  border: 0;
  background: transparent;
  color: var(--stats-text);
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.plugin-config-toggle:hover {
  background: var(--stats-surface-bar);
}

.plugin-config-toggle:focus-visible,
.plugin-config-conflict button:focus-visible,
.plugin-config-actions button:focus-visible {
  outline: 2px solid var(--stats-accent);
  outline-offset: calc(-1 * var(--stats-space-xs));
}

.plugin-config-toggle-title,
.plugin-config-toggle-state,
.plugin-config-toggle-title > span {
  display: flex;
  align-items: center;
}

.plugin-config-toggle-title {
  gap: var(--stats-space-md);
}

.plugin-config-toggle-title > span {
  align-items: flex-start;
  flex-direction: column;
  gap: var(--stats-space-2xs);
}

.plugin-config-toggle-title strong {
  font-size: var(--stats-font-ui);
  font-weight: var(--stats-weight-medium);
}

.plugin-config-toggle-title small {
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}

.plugin-config-toggle-state {
  gap: var(--stats-space-md);
}

.plugin-config-toggle-state > svg {
  color: var(--stats-muted);
  transition: transform 120ms ease;
}

.plugin-config-toggle-state > svg.rotated {
  transform: rotate(180deg);
}

.plugin-config-access {
  min-height: var(--stats-control-height-sm);
  display: inline-flex;
  align-items: center;
  gap: var(--stats-space-xs);
  padding: 0 var(--stats-space-sm);
  border: 1px solid var(--stats-border-strong);
  border-radius: 100vmax;
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
}

.plugin-config-access.editable {
  border-color: var(--stats-accent-soft);
  background: var(--stats-accent-muted);
  color: var(--stats-accent);
}

.plugin-config-access.readonly {
  background: var(--stats-surface-soft);
  color: var(--stats-muted);
}

.plugin-config-body {
  container: plugin-config / inline-size;
  min-width: 0;
  display: grid;
  gap: var(--stats-space-2xl);
  padding: var(--stats-space-2xl);
  border: 0;
  border-top: 1px solid var(--stats-border);
  border-radius: 0;
  background: var(--stats-surface);
  box-shadow: none;
}

.plugin-config-heading {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 2fr);
  align-items: center;
  gap: var(--stats-space-xl);
  padding-bottom: var(--stats-space-lg);
  border-bottom: 1px solid var(--stats-border);
}

.plugin-config-heading > div {
  display: grid;
  gap: var(--stats-space-2xs);
}

.plugin-config-heading span {
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
  text-transform: uppercase;
}

.plugin-config-heading code {
  min-width: 0;
  overflow-wrap: anywhere;
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: var(--stats-font-sm);
}

.plugin-config-heading p {
  margin: 0;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  text-align: right;
}

.plugin-config-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--stats-space-md);
  justify-content: space-between;
  padding-top: var(--stats-space-lg);
  border-top: 1px solid var(--stats-border);
}

.plugin-config-actions button {
  min-height: var(--stats-control-height-md);
  padding: 0 var(--stats-space-lg);
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-soft);
  color: var(--stats-text);
  cursor: pointer;
  font: inherit;
  font-weight: var(--stats-weight-medium);
}

.plugin-config-actions button.primary {
  border-color: var(--stats-accent-soft);
  background: var(--stats-accent-muted);
  color: var(--stats-accent);
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

:global(.stats-theme-arc-glass) .plugin-config-panel {
  border-color: rgb(15 15 20 / 10%);
  background: rgb(255 255 255 / 55%);
  box-shadow:
    0 0.8rem 2.2rem rgb(15 15 20 / 9%),
    inset 0 1px 0 rgb(255 255 255 / 76%);
}

:global(.stats-theme-arc-glass) .plugin-config-body {
  border-top-color: rgb(15 15 20 / 10%);
  background: rgb(255 255 255 / 66%);
}

.plugin-config-conflict {
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-xl);
  padding: var(--stats-space-lg);
  border: 1px solid color-mix(in srgb, var(--stats-danger) 38%, var(--stats-border));
  border-radius: var(--stats-radius-md);
  background: color-mix(in srgb, var(--stats-danger) 8%, var(--stats-surface));
}

.plugin-config-conflict > div {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-xs);
}

.plugin-config-conflict strong {
  color: var(--stats-danger);
  font-size: var(--stats-font-ui);
}

.plugin-config-conflict span {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.plugin-config-conflict button {
  min-height: var(--stats-control-height-md);
  flex: 0 0 auto;
  padding: 0 var(--stats-space-lg);
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  cursor: pointer;
  font: inherit;
  font-weight: var(--stats-weight-medium);
}

.plugin-config-validation-state {
  display: inline-flex;
  align-items: center;
  gap: var(--stats-space-xs);
  margin-right: auto;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.plugin-config-validation-state.valid {
  color: var(--stats-accent);
}

.plugin-config-validation-state.invalid {
  color: var(--stats-danger);
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
    margin-right: var(--stats-space-xl);
    margin-left: var(--stats-space-xl);
  }
}

.plugin-config-error,
.plugin-config-errors {
  color: var(--stats-danger);
}

.plugin-config-errors {
  margin: 0;
  padding-left: var(--stats-space-xl);
}

.plugin-config-valid {
  color: var(--stats-accent);
  font-size: var(--stats-font-sm);
}
</style>
