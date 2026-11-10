<template>
  <section class="config-editor" aria-labelledby="config-editor-heading" :aria-busy="loading || saving">
    <header class="config-editor-header">
      <div class="config-editor-title">
        <h3 id="config-editor-heading">{{ t('config.editableTitle') }}</h3>
        <span>{{ t('config.editableCount', { count: fieldCount }) }}</span>
      </div>
      <p class="config-editor-hint">{{ t('config.editableHint') }}</p>
      <div v-if="source" class="config-editor-source">
        <FileText :size="15" aria-hidden="true" />
        <code>{{ source }}</code>
      </div>
    </header>

    <div v-if="fields" class="config-editor-layout">
      <nav class="config-section-nav" :aria-label="t('config.sections')">
        <span>{{ t('config.sections') }}</span>
        <a v-for="(group, index) in editableGroups" :key="group.id" :href="`#config-section-${group.id}`">
          <small aria-hidden="true">0{{ index + 1 }}</small>{{ t(`config.${group.id}Group`) }}
        </a>
      </nav>
      <form novalidate @submit.prevent="save" @input="clearFeedback" @change="clearFeedback">
      <fieldset :disabled="saving || loading" class="config-editor-fields">
        <legend class="config-sr-only">{{ t('config.editableTitle') }}</legend>
        <section
          v-for="(group, index) in editableGroups"
          :key="group.id"
          :id="`config-section-${group.id}`"
          v-reveal="index"
          class="config-field-group"
          :aria-labelledby="`config-group-${group.id}`"
        >
          <div class="config-group-heading">
            <span class="config-group-number" aria-hidden="true">0{{ index + 1 }}</span>
            <h4 :id="`config-group-${group.id}`">{{ t(`config.${group.id}Group`) }}</h4>
            <p>{{ t(`config.${group.id}Hint`) }}</p>
          </div>
          <div class="config-group-grid">
            <ConfigField
              v-for="field in group.fields"
              :key="field.key"
              v-model="fields[field.key]"
              :field="field"
              :modified="!ConfigValues.equal(fields[field.key], baseline[field.key])"
              @update:model-value="clearFeedback"
            />
          </div>
        </section>
      </fieldset>

      <footer class="config-editor-actions">
        <div class="config-editor-feedback" aria-live="polite" aria-atomic="true">
          <p
            :class="{ 'config-editor-error': error, 'config-editor-success': saved && !error }"
            :role="error ? 'alert' : 'status'"
          >
            <Transition name="config-feedback" mode="out-in">
              <span :key="error ? 'error' : saved ? 'saved' : 'pending'" class="config-feedback-icon" aria-hidden="true">
                <CircleAlert v-if="error" :size="17" />
                <CircleCheck v-else-if="saved" :size="17" />
                <span v-else class="config-status-dot" :class="{ 'is-pending': changedCount > 0 }"></span>
              </span>
            </Transition>
            <span>{{ error || (saved ? t('config.saved') : changedCount ? t('config.unsavedCount', { count: changedCount }) : t('config.noChanges')) }}</span>
          </p>
          <small v-if="!saved">{{ t('config.applyHint') }}</small>
        </div>
        <div class="config-editor-buttons">
          <button class="config-reset" type="button" :disabled="!changedCount || saving || loading" @click="reset">
            <RotateCcw :size="15" aria-hidden="true" />{{ t('config.reset') }}
          </button>
          <button class="config-save" type="submit" :disabled="!changedCount || saving || loading">
            <LoaderCircle v-if="saving" class="config-saving-icon" :size="16" aria-hidden="true" />
            <Save v-else :size="16" aria-hidden="true" />
            {{ saving ? t('config.saving') : t('config.save') }}
          </button>
        </div>
      </footer>
      </form>
    </div>

    <div v-else class="config-editor-empty" role="status">
      <template v-if="loading">
        <LoaderCircle class="config-saving-icon" :size="20" aria-hidden="true" />
        <p>{{ t('config.loadingFile') }}</p>
      </template>
      <template v-else>
        <CircleAlert :size="20" aria-hidden="true" />
        <p v-if="error" class="config-editor-error" role="alert">{{ error }}</p>
        <p v-else>{{ unavailable }}</p>
        <button v-if="error || unavailable" class="config-reset" type="button" @click="load()">{{ t('config.retry') }}</button>
      </template>
    </div>
  </section>
</template>

<script setup>
import { CircleAlert, CircleCheck, FileText, LoaderCircle, RotateCcw, Save } from '@lucide/vue';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import { readEditableConfig, saveEditableConfig } from '../api';
import { useModuleLocale } from '../locale';
import strings from './locale';
import ConfigField from './config/ConfigField.vue';
import { ConfigFieldCatalog } from './config/fields';
import { ConfigValues } from './config/ConfigValues';

const { t } = useModuleLocale(strings);
const props = defineProps({ refreshNonce: { type: Number, default: 0 } });
const fields = ref(null);
const kinds = ref({});
const editableGroups = computed(() => ConfigFieldCatalog.groups(kinds.value));
const fieldCount = computed(() => Object.keys(kinds.value).length);
// Last file response is the baseline for local change indicators and undo only.
const baseline = ref(null);
const source = ref('');
const unavailable = ref('');
const loading = ref(false);
const saving = ref(false);
const saved = ref(false);
const error = ref('');
const changes = computed(() => ConfigValues.changes(fields.value, baseline.value));
const changedCount = computed(() => Object.keys(changes.value).length);
let handledRefreshNonce = props.refreshNonce;
let disposed = false;

onMounted(() => load());
watch(() => props.refreshNonce, () => load());
onBeforeUnmount(() => { disposed = true; });

function clearFeedback() {
  saved.value = false;
  error.value = '';
}

function reset() {
  return load({ preserveDraft: false });
}

function acceptFields(values, fieldKinds) {
  try {
    ConfigValues.assertCatalog(values, fieldKinds, t);
  } catch (err) {
    fields.value = null;
    baseline.value = null;
    kinds.value = {};
    throw err;
  }
  kinds.value = { ...fieldKinds };
  baseline.value = ConfigValues.forEditing(values, fieldKinds);
  fields.value = ConfigValues.clone(baseline.value);
}

async function load({ preserveDraft = true, preserveError = false } = {}) {
  if (disposed || saving.value || loading.value) return;
  const requestedNonce = props.refreshNonce;
  const draft = preserveDraft ? ConfigValues.clone(changes.value) : null;
  loading.value = true;
  if (!preserveError) clearFeedback();
  try {
    const result = await readEditableConfig();
    if (disposed) return;
    if (result.available) {
      acceptFields(result.fields, result.kinds);
      for (const [key, value] of Object.entries(draft ?? {})) {
        if (Object.hasOwn(fields.value, key)) fields.value[key] = value;
      }
    } else {
      fields.value = null;
      baseline.value = null;
      kinds.value = {};
    }
    source.value = result.source?.path ?? '';
    unavailable.value = result.reason ?? '';
  } catch (err) {
    if (!disposed) error.value = String(err.message ?? err);
  } finally {
    handledRefreshNonce = requestedNonce;
    loading.value = false;
    if (!disposed && props.refreshNonce !== handledRefreshNonce) await load({ preserveDraft, preserveError });
  }
}

async function save(event) {
  if (disposed || saving.value || loading.value || !changedCount.value) return;
  clearFeedback();
  const invalid = event.currentTarget.querySelector('input:invalid, select:invalid, textarea:invalid');
  if (invalid) {
    error.value = t(invalid.validity.valueMissing ? 'config.requiredValue'
      : invalid.type === 'number' ? 'config.invalidInteger' : 'config.invalidValue', { field: invalid.name });
    invalid.focus();
    return;
  }
  let succeeded = false;
  saving.value = true;
  try {
    const result = await saveEditableConfig(ConfigValues.serialize(changes.value, kinds.value, t));
    if (disposed) return;
    acceptFields(result.fields, result.kinds);
    saved.value = true;
    succeeded = true;
  } catch (err) {
    if (!disposed) error.value = String(err.message ?? err);
  } finally {
    saving.value = false;
    if (!disposed && props.refreshNonce !== handledRefreshNonce) {
      await load({ preserveDraft: !succeeded, preserveError: !succeeded });
    }
  }
}
</script>

<style scoped>
.config-editor { min-width: 0; }
.config-editor-header { padding-bottom: 24px; }
.config-editor-title { display: flex; align-items: baseline; flex-wrap: wrap; gap: 12px; }
.config-editor-title h3 { margin: 0; font-size: 17px; font-weight: 600; letter-spacing: -.02em; }
.config-editor-title > span { color: var(--ui-muted); font-size: 12px; }
.config-editor-hint { margin: 8px 0 0; color: var(--ui-muted); font-size: 13px; line-height: 1.6; }
.config-editor-source { display: flex; align-items: flex-start; gap: 8px; margin-top: 16px; color: var(--ui-muted); }
.config-editor-source svg { flex-shrink: 0; margin-top: 2px; }
.config-editor-source code { font: 12px/1.6 var(--ui-mono); overflow-wrap: anywhere; }
.config-editor-fields { position: relative; min-width: 0; margin: 0; padding: 0; border: 0; }
.config-sr-only { position: absolute; width: 1px; height: 1px; padding: 0; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
.config-editor-layout { display: grid; grid-template-columns: 146px minmax(0, 1fr); align-items: start; gap: 32px; }
.config-editor-layout form { min-width: 0; }
.config-section-nav { position: sticky; top: 96px; display: flex; flex-direction: column; gap: 3px; padding: 16px 0; }
.config-section-nav > span { margin-bottom: 12px; padding-left: 12px; color: var(--ui-muted); font-size: 10px; text-transform: uppercase; letter-spacing: .06em; }
.config-section-nav a { display: flex; align-items: baseline; gap: 9px; padding: 10px 8px 10px 12px; border-left: 1px solid var(--ui-border); color: var(--ui-muted); text-decoration: none; font-size: 12px; line-height: 1.5; transition: color var(--ui-duration-fast), background var(--ui-duration-fast); }
.config-section-nav small { font: 9px var(--ui-mono); }
.config-section-nav a:hover { color: var(--ui-text); border-color: var(--ui-text); background: var(--ui-accent-faint); }
.config-section-nav a:focus-visible { outline: 2px solid var(--ui-focus-ring-color); outline-offset: 2px; }
.config-field-group { padding: 28px 0 32px; border-top: 1px solid var(--ui-border); scroll-margin-top: 96px; }
.config-group-heading { display: grid; grid-template-columns: 24px minmax(0, 1fr); align-items: baseline; gap: 5px 8px; }
.config-group-number { color: var(--ui-muted); font: 11px/1 var(--ui-mono); }
.config-group-heading h4 { margin: 0; font-size: 14px; font-weight: 600; }
.config-group-heading p { grid-column: 2; margin: 0; color: var(--ui-muted); font-size: 12px; line-height: 1.7; }
.config-group-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); align-items: start; gap: 24px; margin-top: 24px; }
.config-editor-fields:disabled { opacity: .6; }
.config-editor-actions {
  position: sticky; bottom: 0; z-index: 5; display: flex; align-items: center; justify-content: space-between;
  gap: 20px; padding: 18px 0; border-top: 1px solid var(--ui-border-strong); background: var(--ui-bg-base);
}
.config-editor-feedback { min-width: 0; }
.config-editor-feedback p { display: flex; align-items: flex-start; gap: 8px; margin: 0; font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
.config-feedback-icon { display: inline-flex; flex-shrink: 0; min-width: 17px; margin-top: 1px; }
.config-editor-feedback > small { display: block; margin-top: 4px; color: var(--ui-muted); font-size: 11px; line-height: 1.6; }
.config-status-dot { flex-shrink: 0; width: 6px; height: 6px; margin: 6px 3px; border-radius: 50%; background: var(--ui-border-strong); transition: background var(--ui-duration-base); }
.config-status-dot.is-pending { background: var(--ui-accent-strong); }
.config-editor-buttons { display: flex; flex-shrink: 0; gap: 10px; }
.config-save, .config-reset {
  display: inline-flex; align-items: center; justify-content: center; gap: 8px; min-height: 40px;
  padding: 0 16px; border: 1px solid transparent; border-radius: 6px; font: inherit; font-size: 12px; font-weight: 500;
  cursor: pointer; transition: background var(--ui-duration-fast), opacity var(--ui-duration-fast), transform var(--ui-duration-fast);
}
.config-save { border-color: var(--ui-text); background: var(--ui-text); color: var(--ui-bg-base); }
.config-save:hover:not(:disabled) { background: var(--ui-accent-strong); border-color: var(--ui-accent-strong); }
.config-reset { background: transparent; color: var(--ui-muted); }
.config-reset:hover:not(:disabled) { background: var(--ui-bg-hover); color: var(--ui-text); }
.config-save:active:not(:disabled), .config-reset:active:not(:disabled) { transform: translateY(1px); }
.config-save:focus-visible, .config-reset:focus-visible { outline: 2px solid var(--ui-focus-ring-color); outline-offset: 3px; }
.config-save:disabled, .config-reset:disabled { opacity: .4; cursor: default; }
.config-editor-success { color: var(--ui-text); }
.config-editor-error { color: var(--ui-danger); overflow-wrap: anywhere; }
.config-editor-empty { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 48px 20px; border-top: 1px solid var(--ui-border); color: var(--ui-muted); text-align: center; font-size: 13px; }
.config-editor-empty p { margin: 0; }
.config-saving-icon { animation: config-spin 1s linear infinite; }
.config-feedback-enter-active, .config-feedback-leave-active { transition: opacity var(--ui-duration-fast), transform var(--ui-duration-fast); }
.config-feedback-enter-from { opacity: 0; transform: translateY(3px); }
.config-feedback-leave-to { opacity: 0; transform: translateY(-3px); }
@keyframes config-spin { to { transform: rotate(360deg); } }
@media (max-width: 1100px) {
  .config-editor-layout { grid-template-columns: 124px minmax(0, 1fr); gap: 20px; }
  .config-group-grid { gap: 20px 16px; }
}
@media (max-width: 900px) {
  .config-editor-layout { grid-template-columns: minmax(0, 1fr); }
  .config-section-nav { position: static; flex-direction: row; flex-wrap: wrap; padding: 0; }
  .config-section-nav > span { display: none; }
}
@media (max-width: 600px) {
  .config-group-grid { grid-template-columns: minmax(0, 1fr); gap: 20px; }
  .config-field-group { padding: 24px 0 28px; }
  .config-control input, .config-control select { height: 44px; font-size: 16px; }
  .config-editor-actions { align-items: stretch; flex-direction: column; gap: 12px; padding: 14px 0 max(14px, env(safe-area-inset-bottom)); }
  .config-editor-buttons { justify-content: flex-end; }
  .config-save, .config-reset { min-height: 44px; }
  .config-save { flex: 1; }
}
@media (prefers-reduced-motion: reduce) {
  .config-saving-icon { animation: none; }
  .config-feedback-enter-active, .config-feedback-leave-active { transition: none; }
}
</style>
