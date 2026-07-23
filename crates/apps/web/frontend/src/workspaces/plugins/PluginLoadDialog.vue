<template>
  <Teleport to=".app-shell">
    <div v-if="open" class="plugin-load-backdrop" @mousedown.self="close">
      <section
        class="plugin-load-dialog"
        role="dialog"
        aria-modal="true"
        :aria-labelledby="titleId"
      >
        <header class="plugin-load-header">
          <div>
            <span>{{ needsFilePolicy ? 'Configure file access' : 'Load plugin' }}</span>
            <h2 :id="titleId">{{ plugin.plugin_id }}</h2>
          </div>
          <button type="button" aria-label="Close load dialog" :disabled="busy" @click="close">
            <X :size="18" aria-hidden="true" />
          </button>
        </header>

        <form class="plugin-load-form" @submit.prevent="submit">
          <label class="plugin-load-field">
            <span>Runtime instance name</span>
            <input v-model="instanceId" type="text" autocomplete="off" :disabled="busy" />
            <small>This name identifies the loaded plugin in commands and status views.</small>
          </label>

          <details class="plugin-load-permissions">
            <summary>
              <span>Built-in access</span>
              <small>{{ plugin.automatic_host_grants?.length ?? 0 }} read-only permissions</small>
            </summary>
            <div class="plugin-load-chips">
              <code v-for="grant in plugin.automatic_host_grants" :key="grant">{{ grant }}</code>
              <span v-if="!plugin.automatic_host_grants?.length">None</span>
            </div>
          </details>

          <section v-if="needsFilePolicy" class="plugin-load-section editable">
            <div class="plugin-load-section-heading">
              <div>
                <span>Files this plugin can manage</span>
                <small>The plugin can create only the selected rule types inside these paths.</small>
              </div>
              <strong>Required</strong>
            </div>

            <div class="plugin-load-scope-list">
              <div v-for="(scope, index) in filePolicyScopes" :key="scope.key" class="plugin-load-scope">
                <label>
                  <span>Path</span>
                  <input
                    v-model="scope.path_scope"
                    type="text"
                    placeholder="/workspace/project/**"
                    autocomplete="off"
                    :disabled="busy"
                    @blur="showValidation = true"
                  />
                  <small>Use an absolute file path or a directory ending in <code>/**</code>.</small>
                </label>
                <fieldset>
                  <legend>Rule types</legend>
                  <label v-for="decision in decisionOptions" :key="decision.value">
                    <input v-model="scope.decisions" type="checkbox" :value="decision.value" :disabled="busy" />
                    <span>{{ decision.label }}</span>
                  </label>
                </fieldset>
                <button
                  v-if="filePolicyScopes.length > 1"
                  class="plugin-load-remove"
                  type="button"
                  :disabled="busy"
                  @click="removeScope(index)"
                >
                  <Trash2 :size="15" aria-hidden="true" />
                  Remove scope
                </button>
              </div>
            </div>
            <button class="plugin-load-add" type="button" :disabled="busy" @click="addScope">
              <Plus :size="15" aria-hidden="true" />
              Add another path
            </button>
          </section>

          <section v-if="needsEnvRead" class="plugin-load-section editable">
            <div class="plugin-load-section-heading">
              <div>
                <span>Readable environment variables</span>
                <small>Only the listed variable names are exposed to the plugin.</small>
              </div>
              <strong>Required</strong>
            </div>
            <label class="plugin-load-field">
              <span>Variable names</span>
              <textarea
                v-model="envReadText"
                rows="3"
                placeholder="API_TOKEN&#10;REGION"
                :disabled="busy"
              ></textarea>
              <small>Enter one variable name per line.</small>
            </label>
          </section>

          <p v-if="showValidation && validationError" class="plugin-load-error">{{ validationError }}</p>

          <footer class="plugin-load-actions">
            <button type="button" :disabled="busy" @click="close">Cancel</button>
            <button class="primary" type="submit" :disabled="busy || !valid">
              {{ busy ? 'Loading…' : 'Load plugin' }}
            </button>
          </footer>
        </form>
      </section>
    </div>
  </Teleport>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { Plus, Trash2, X } from '@lucide/vue';

const props = defineProps({
  open: { type: Boolean, required: true },
  plugin: { type: Object, required: true },
  busy: { type: Boolean, default: false },
});

const emit = defineEmits(['close', 'submit']);
const decisionOptions = [
  { value: 'allow', label: 'Allow' },
  { value: 'deny', label: 'Deny' },
  { value: 'gray', label: 'Ask plugin' },
];
const decisions = decisionOptions.map((decision) => decision.value);
const instanceId = ref('');
const filePolicyScopes = ref([]);
const envReadText = ref('');
const showValidation = ref(false);
let nextScopeKey = 0;

const titleId = computed(() => `plugin-load-title-${props.plugin.package_key}`);
const needsFilePolicy = computed(() => props.plugin.parameterized_host_grants
  ?.includes('file-policy.rules.apply'));
const needsEnvRead = computed(() => props.plugin.parameterized_host_grants?.includes('env-read'));
const envRead = computed(() => envReadText.value
  .split('\n')
  .map((name) => name.trim())
  .filter(Boolean));
const validationError = computed(() => {
  if (!instanceId.value || instanceId.value.trim() !== instanceId.value) {
    return 'Instance ID is required and cannot have surrounding whitespace.';
  }
  if (needsFilePolicy.value) {
    for (const scope of filePolicyScopes.value) {
      if (!scope.path_scope.startsWith('/')) {
        return 'Every file-policy scope must be an absolute path.';
      }
      if (scope.decisions.length === 0) {
        return 'Select at least one rule decision for every file-policy scope.';
      }
    }
  }
  if (needsEnvRead.value) {
    if (envRead.value.length === 0) {
      return 'Enter at least one environment variable name.';
    }
    if (envRead.value.some((name) => !/^[A-Za-z_][A-Za-z0-9_]*$/.test(name))) {
      return 'Environment variable names may contain letters, digits, and underscores.';
    }
  }
  return '';
});
const valid = computed(() => !validationError.value);

watch(
  () => [props.open, props.plugin.package_key],
  ([open]) => {
    if (open) reset();
  },
  { immediate: true },
);

onMounted(() => window.addEventListener('keydown', onKeydown));
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown));

function reset() {
  instanceId.value = props.plugin.plugin_id ?? '';
  filePolicyScopes.value = [newScope()];
  envReadText.value = '';
  showValidation.value = false;
}

function newScope() {
  nextScopeKey += 1;
  return { key: nextScopeKey, path_scope: '', decisions: [...decisions] };
}

function addScope() {
  filePolicyScopes.value.push(newScope());
}

function removeScope(index) {
  filePolicyScopes.value.splice(index, 1);
}

function close() {
  if (!props.busy) emit('close');
}

function onKeydown(event) {
  if (event.key === 'Escape' && props.open) close();
}

function submit() {
  if (!valid.value || props.busy) return;
  emit('submit', {
    instance_id: instanceId.value,
    grants: {
      file_policy_rules_apply: needsFilePolicy.value
        ? filePolicyScopes.value.flatMap((scope) => scope.decisions.map((decision) => ({
          decision,
          path_scope: scope.path_scope,
        })))
        : [],
      env_read: needsEnvRead.value ? envRead.value : [],
    },
  });
}
</script>

<style scoped>
.plugin-load-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: grid;
  place-items: center;
  padding: var(--stats-space-xl);
  background: rgb(4 9 18 / 72%);
  backdrop-filter: blur(0.25rem);
}

.plugin-load-dialog {
  min-width: 0;
  width: min(45rem, 100%);
  max-height: min(52.5rem, calc(100vh - 2 * var(--stats-space-xl)));
  overflow: auto;
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-lg);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  box-shadow: 0 1.5rem 5rem rgb(0 0 0 / 42%);
}

.plugin-load-header,
.plugin-load-actions,
.plugin-load-section-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
}

.plugin-load-header {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: var(--stats-space-xl) var(--stats-space-2xl);
  border-bottom: 1px solid var(--stats-border);
  background: var(--stats-surface-strong);
}

.plugin-load-header span,
.plugin-load-section-heading small,
.plugin-load-field small,
.plugin-load-scope small,
.plugin-load-permissions small {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.plugin-load-scope small code {
  color: inherit;
  font-size: inherit;
}

.plugin-load-header h2 {
  margin: var(--stats-space-2xs) 0 0;
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-medium);
}

.plugin-load-header button,
.plugin-load-actions button,
.plugin-load-add,
.plugin-load-remove {
  min-height: var(--stats-control-height-md);
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-soft);
  color: var(--stats-text);
  cursor: pointer;
  font: inherit;
}

.plugin-load-header button {
  width: var(--stats-control-height-md);
  display: grid;
  place-items: center;
  padding: 0;
}

.plugin-load-form {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-2xl);
}

.plugin-load-field,
.plugin-load-section-heading > div,
.plugin-load-scope > label {
  display: grid;
  gap: var(--stats-space-xs);
}

.plugin-load-field > span,
.plugin-load-scope > label > span,
.plugin-load-section-heading span,
.plugin-load-scope legend {
  color: var(--stats-text);
  font-size: var(--stats-font-md);
  font-weight: var(--stats-weight-medium);
}

.plugin-load-field input,
.plugin-load-field textarea,
.plugin-load-scope input[type="text"] {
  width: 100%;
  padding: var(--stats-space-md);
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
  color: var(--stats-text);
  font: inherit;
}

.plugin-load-section {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-lg);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
}

.plugin-load-section.readonly {
  background: var(--stats-surface-soft);
}

.plugin-load-section.editable {
  border-color: var(--stats-accent-soft);
  background: var(--stats-accent-faint);
}

.plugin-load-permissions {
  min-width: 0;
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface-soft);
}

.plugin-load-permissions summary {
  min-height: var(--stats-control-height-lg);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
  padding: 0 var(--stats-space-lg);
  color: var(--stats-text);
  cursor: pointer;
  font-size: var(--stats-font-md);
  font-weight: var(--stats-weight-medium);
}

.plugin-load-permissions summary::marker {
  color: var(--stats-muted);
}

.plugin-load-permissions .plugin-load-chips {
  padding: 0 var(--stats-space-lg) var(--stats-space-lg);
}

.plugin-load-section-heading strong {
  padding: var(--stats-space-xs) var(--stats-space-sm);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}

.plugin-load-section.editable .plugin-load-section-heading strong {
  color: var(--stats-accent);
}

.plugin-load-chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--stats-space-sm);
}

.plugin-load-chips code {
  padding: var(--stats-space-xs) var(--stats-space-sm);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}

.plugin-load-scope-list {
  display: grid;
  gap: var(--stats-space-md);
}

.plugin-load-scope {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--stats-space-md);
  padding: var(--stats-space-md);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
}

.plugin-load-scope fieldset {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--stats-space-md);
  margin: 0;
  padding: 0;
  border: 0;
}

.plugin-load-scope fieldset label,
.plugin-load-add,
.plugin-load-remove {
  display: inline-flex;
  align-items: center;
  gap: var(--stats-space-xs);
}

.plugin-load-scope fieldset label {
  min-height: var(--stats-control-height-sm);
  padding: 0 var(--stats-space-sm);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-soft);
  color: var(--stats-text);
  font-size: var(--stats-font-sm);
}

.plugin-load-scope legend {
  margin-bottom: var(--stats-space-xs);
}

.plugin-load-remove {
  grid-column: 1 / -1;
  justify-self: end;
  min-height: var(--stats-control-height-sm);
  padding: 0 var(--stats-space-md);
  color: var(--stats-danger);
}

.plugin-load-add {
  justify-self: start;
  padding: 0 var(--stats-space-md);
}

.plugin-load-error {
  margin: 0;
  color: var(--stats-danger);
  font-size: var(--stats-font-sm);
}

.plugin-load-actions {
  padding-top: var(--stats-space-lg);
  border-top: 1px solid var(--stats-border);
  justify-content: flex-end;
}

.plugin-load-actions button {
  padding: 0 var(--stats-space-lg);
}

.plugin-load-actions button.primary {
  border-color: var(--stats-accent-soft);
  background: var(--stats-accent-muted);
  color: var(--stats-accent);
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

@media (max-width: 47.5rem) {
  .plugin-load-scope {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (max-width: 42.5rem) {
  .plugin-load-backdrop {
    padding: 0;
  }

  .plugin-load-dialog {
    max-height: 100vh;
    border-radius: 0;
  }
}
</style>
