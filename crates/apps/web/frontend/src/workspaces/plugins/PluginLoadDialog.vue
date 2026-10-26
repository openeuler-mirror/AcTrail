<template>
  <Teleport to=".app-shell">
    <Transition name="dialog">
    <div v-if="open" class="plugin-load-backdrop" @mousedown.self="close">
      <section
        ref="dialogRef"
        class="plugin-load-dialog"
        role="dialog"
        aria-modal="true"
        :aria-labelledby="titleId"
      >
        <header class="plugin-load-header">
          <div>
            <span>{{ loadSubtitle }}</span>
            <h2 :id="titleId">{{ plugin.plugin_id }}</h2>
          </div>
          <button type="button" :aria-label="t('load.close')" :disabled="busy" @click="close">
            <X :size="18" aria-hidden="true" />
          </button>
        </header>

        <form class="plugin-load-form" @submit.prevent="submit">
          <label class="plugin-load-field">
            <span>{{ t('load.instanceName') }}</span>
            <input v-model="instanceId" type="text" autocomplete="off" :disabled="busy" />
            <small>{{ t('load.instanceNameHint') }}</small>
          </label>

          <details class="plugin-load-permissions">
            <summary>
              <span>{{ t('load.builtInAccess') }}</span>
              <small>{{ t('load.automaticPermissions', { count: plugin.automatic_host_grants?.length ?? 0 }) }}</small>
            </summary>
            <div class="plugin-load-chips">
              <code v-for="grant in plugin.automatic_host_grants" :key="grant">{{ grant }}</code>
              <span v-if="!plugin.automatic_host_grants?.length">{{ t('load.none') }}</span>
            </div>
          </details>

          <PolicyScopeEditor
            v-if="needsFilePolicy"
            v-model="filePolicyScopes"
            :title="t('load.fileTitle')"
            :description="t('load.fileDescription')"
            :placeholder="t('load.filePlaceholder')"
            :path-hint="t('load.fileHint')"
            :busy="busy"
            @blur="showValidation = true"
          />

          <PolicyScopeEditor
            v-if="needsCommandPolicy"
            v-model="commandPolicyScopes"
            :title="t('load.commandTitle')"
            :description="t('load.commandDescription')"
            :path-label="t('load.commandPathLabel')"
            :placeholder="t('load.commandPlaceholder')"
            :path-hint="t('load.commandHint')"
            :busy="busy"
            @blur="showValidation = true"
          />

          <PolicyScopeEditor
            v-if="needsNetworkPolicy"
            v-model="networkPolicyScopes"
            :title="t('load.networkTitle')"
            :description="t('load.networkDescription')"
            :path-label="t('load.networkPathLabel')"
            :placeholder="t('load.networkPlaceholder')"
            :path-hint="t('load.networkHint')"
            :add-label="t('load.addEndpoint')"
            :busy="busy"
            @blur="showValidation = true"
          />

          <section v-if="needsEnvRead" class="plugin-load-section editable">
            <div class="plugin-load-section-heading">
              <div>
                <span>{{ t('load.envTitle') }}</span>
                <small>{{ t('load.envHint') }}</small>
              </div>
              <strong>{{ t('load.required') }}</strong>
            </div>
            <label class="plugin-load-field">
              <span>{{ t('load.variableNames') }}</span>
              <textarea
                v-model="envReadText"
                rows="3"
                :placeholder="t('load.variablePlaceholder')"
                :disabled="busy"
              ></textarea>
              <small>{{ t('load.variableHint') }}</small>
            </label>
          </section>

          <p v-if="showValidation && validationError" class="plugin-load-error">{{ validationError }}</p>

          <footer class="plugin-load-actions">
            <button type="button" :disabled="busy" @click="close">{{ t('load.cancel') }}</button>
            <button class="primary" type="submit" :disabled="busy || !valid">
              {{ busy ? t('load.loading') : t('load.submit') }}
            </button>
          </footer>
        </form>
      </section>
    </div>
    </Transition>
  </Teleport>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { X } from '@lucide/vue';
import { useDialogBehavior } from '../../composables/dialog';
import { useModuleLocale } from '../../locale';
import strings from './locale';
import PolicyScopeEditor from './PolicyScopeEditor.vue';

const { t } = useModuleLocale(strings);

const props = defineProps({
  open: { type: Boolean, required: true },
  plugin: { type: Object, required: true },
  busy: { type: Boolean, default: false },
});

const emit = defineEmits(['close', 'submit']);
const instanceId = ref('');
const filePolicyScopes = ref([]);
const commandPolicyScopes = ref([]);
const networkPolicyScopes = ref([]);
const envReadText = ref('');
const showValidation = ref(false);

const titleId = computed(() => `plugin-load-title-${props.plugin.package_key}`);
const needsFilePolicy = computed(() => props.plugin.parameterized_host_grants
  ?.includes('file-policy.rules.apply'));
const needsCommandPolicy = computed(() => props.plugin.parameterized_host_grants
  ?.includes('command-policy.rules.apply'));
const needsNetworkPolicy = computed(() => props.plugin.parameterized_host_grants
  ?.includes('network-policy.rules.apply'));
const needsEnvRead = computed(() => props.plugin.parameterized_host_grants?.includes('env-read'));
const loadSubtitle = computed(() => {
  if ([needsFilePolicy.value, needsCommandPolicy.value, needsNetworkPolicy.value]
    .filter(Boolean).length > 1) return t('load.subtitlePolicy');
  if (needsNetworkPolicy.value) return t('load.subtitleNetwork');
  if (needsCommandPolicy.value) return t('load.subtitleCommand');
  if (needsFilePolicy.value) return t('load.subtitleFile');
  return t('load.submit');
});
const envRead = computed(() => envReadText.value
  .split('\n')
  .map((name) => name.trim())
  .filter(Boolean));
const validationError = computed(() => {
  if (!instanceId.value || instanceId.value.trim() !== instanceId.value) {
    return t('load.invalidInstanceId');
  }
  const fileScopeError = needsFilePolicy.value
    ? validateScopes(filePolicyScopes.value, t('load.filePathLabel'))
    : '';
  if (fileScopeError) return fileScopeError;
  const commandScopeError = needsCommandPolicy.value
    ? validateScopes(commandPolicyScopes.value, t('load.commandPathLabel'))
    : '';
  if (commandScopeError) return commandScopeError;
  const networkScopeError = needsNetworkPolicy.value
    ? validateNetworkScopes(networkPolicyScopes.value)
    : '';
  if (networkScopeError) return networkScopeError;
  if (needsEnvRead.value) {
    if (envRead.value.length === 0) {
      return t('load.envRequired');
    }
    if (envRead.value.some((name) => !/^[A-Za-z_][A-Za-z0-9_]*$/.test(name))) {
      return t('load.envInvalid');
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

const dialogRef = ref(null);

useDialogBehavior({
  isOpen: () => props.open,
  getContainer: () => dialogRef.value,
  onClose: close,
});

function reset() {
  instanceId.value = props.plugin.plugin_id ?? '';
  filePolicyScopes.value = [newScope('file')];
  commandPolicyScopes.value = [newScope('command')];
  networkPolicyScopes.value = [newScope('network')];
  envReadText.value = '';
  showValidation.value = false;
}

function newScope(kind) {
  return {
    key: `${kind}-policy-scope-initial`,
    path_scope: '',
    decisions: ['allow', 'deny', 'gray'],
  };
}

function validateScopes(scopes, label) {
  for (const scope of scopes) {
    if (!scope.path_scope.startsWith('/')) {
      return t('load.absolutePath', { label });
    }
    if (scope.decisions.length === 0) {
      return t('load.decisionsRequired', { label });
    }
  }
  return '';
}

function validateNetworkScopes(scopes) {
  for (const scope of scopes) {
    if (!scope.path_scope || (scope.path_scope !== '*' && !looksLikeNumericRemoteScope(scope.path_scope))) {
      return t('load.endpointInvalid');
    }
    if (scope.decisions.length === 0) {
      return t('load.endpointDecisionsRequired');
    }
  }
  return '';
}

function looksLikeNumericRemoteScope(value) {
  const ipv4 = value.match(/^([0-9]{1,3}(?:\.[0-9]{1,3}){3}):(\*|[0-9]{1,5})$/);
  if (ipv4) {
    return ipv4[1].split('.').every((part) => Number(part) <= 255)
      && (ipv4[2] === '*' || Number(ipv4[2]) <= 65535);
  }
  const ipv6 = value.match(/^\[([0-9A-Fa-f:.]+)\]:(\*|[0-9]{1,5})$/);
  return Boolean(
    ipv6
    && ipv6[1].includes(':')
    && (ipv6[2] === '*' || Number(ipv6[2]) <= 65535),
  );
}

function close() {
  if (!props.busy) emit('close');
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
      command_policy_rules_apply: needsCommandPolicy.value
        ? commandPolicyScopes.value.flatMap((scope) => scope.decisions.map((decision) => ({
          decision,
          path_scope: scope.path_scope,
        })))
        : [],
      network_policy_rules_apply: needsNetworkPolicy.value
        ? networkPolicyScopes.value.flatMap((scope) => scope.decisions.map((decision) => ({
          decision,
          remote_scope: scope.path_scope,
        })))
        : [],
      env_read: needsEnvRead.value ? envRead.value : [],
    },
  });
}
</script>

<style scoped>
.dialog-enter-active {
  transition: opacity 160ms var(--ui-ease-out);
}

.dialog-leave-active {
  transition: opacity 120ms var(--ui-ease-out);
}

.dialog-enter-from,
.dialog-leave-to {
  opacity: 0;
}

.dialog-enter-active .plugin-load-dialog {
  animation: dialog-pop 300ms cubic-bezier(0.34, 1.56, 0.64, 1);
}

.dialog-leave-active .plugin-load-dialog {
  animation: dialog-fold 200ms cubic-bezier(0.32, 0, 0.67, 0) forwards;
}

@keyframes dialog-pop {
  from {
    opacity: 0;
    transform: scale(0.92) translateY(12px);
  }

  to {
    opacity: 1;
    transform: none;
  }
}

@keyframes dialog-fold {
  to {
    opacity: 0;
    transform: scale(0.95) translateY(6px);
  }
}

.plugin-load-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: grid;
  place-items: center;
  padding: var(--ui-space-xl);
  background: rgb(4 9 18 / 72%);
  backdrop-filter: blur(0.25rem);
}

.plugin-load-dialog {
  min-width: 0;
  width: min(45rem, 100%);
  max-height: min(52.5rem, calc(100vh - 2 * var(--ui-space-xl)));
  overflow: auto;
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-lg);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  box-shadow: 0 1.5rem 5rem rgb(0 0 0 / 42%);
}

.plugin-load-header,
.plugin-load-actions,
.plugin-load-section-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-lg);
}

.plugin-load-header {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: var(--ui-space-xl) var(--ui-space-2xl);
  border-bottom: 1px solid var(--ui-border);
  background: var(--ui-surface-strong);
}

.plugin-load-header span,
.plugin-load-section-heading small,
.plugin-load-field small,
.plugin-load-permissions small {
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.plugin-load-header h2 {
  margin: var(--ui-space-2xs) 0 0;
  font-size: var(--ui-font-display-sm);
  font-weight: var(--ui-weight-medium);
}

.plugin-load-header button,
.plugin-load-actions button {
  min-height: var(--ui-control-height-md);
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-soft);
  color: var(--ui-text);
  cursor: pointer;
  font: inherit;
}

.plugin-load-header button {
  width: var(--ui-control-height-md);
  display: grid;
  place-items: center;
  padding: 0;
}

.plugin-load-form {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-lg);
  padding: var(--ui-space-2xl);
}

.plugin-load-field,
.plugin-load-section-heading > div {
  display: grid;
  gap: var(--ui-space-xs);
}

.plugin-load-field > span,
.plugin-load-section-heading span {
  color: var(--ui-text);
  font-size: var(--ui-font-md);
  font-weight: var(--ui-weight-medium);
}

.plugin-load-field input,
.plugin-load-field textarea {
  width: 100%;
  padding: var(--ui-space-md);
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
  color: var(--ui-text);
  font: inherit;
}

.plugin-load-section {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-lg);
  padding: var(--ui-space-lg);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
}

.plugin-load-section.readonly {
  background: var(--ui-surface-soft);
}

.plugin-load-section.editable {
  border-color: var(--ui-accent-soft);
  background: var(--ui-accent-faint);
}

.plugin-load-permissions {
  min-width: 0;
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface-soft);
}

.plugin-load-permissions summary {
  min-height: var(--ui-control-height-lg);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-lg);
  padding: 0 var(--ui-space-lg);
  color: var(--ui-text);
  cursor: pointer;
  font-size: var(--ui-font-md);
  font-weight: var(--ui-weight-medium);
}

.plugin-load-permissions summary::marker {
  color: var(--ui-muted);
}

.plugin-load-permissions .plugin-load-chips {
  padding: 0 var(--ui-space-lg) var(--ui-space-lg);
}

.plugin-load-section-heading strong {
  padding: var(--ui-space-xs) var(--ui-space-sm);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-medium);
  text-transform: uppercase;
}

.plugin-load-section.editable .plugin-load-section-heading strong {
  color: var(--ui-accent);
}

.plugin-load-chips {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ui-space-sm);
}

.plugin-load-chips code {
  padding: var(--ui-space-xs) var(--ui-space-sm);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.plugin-load-error {
  margin: 0;
  color: var(--ui-danger);
  font-size: var(--ui-font-sm);
}

.plugin-load-actions {
  padding-top: var(--ui-space-lg);
  border-top: 1px solid var(--ui-border);
  justify-content: flex-end;
}

.plugin-load-actions button {
  padding: 0 var(--ui-space-lg);
}

.plugin-load-actions button.primary {
  border-color: var(--ui-accent-soft);
  background: var(--ui-accent-muted);
  color: var(--ui-accent);
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
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
