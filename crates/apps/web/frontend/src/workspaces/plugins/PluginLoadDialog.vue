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
            <span>{{ loadSubtitle }}</span>
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

          <PolicyScopeEditor
            v-if="needsFilePolicy"
            v-model="filePolicyScopes"
            title="Files this plugin can manage"
            description="The plugin can create only the selected rule types inside these paths."
            placeholder="/workspace/project/**"
            path-hint="Use an absolute file path or a directory ending in /**."
            :busy="busy"
            @blur="showValidation = true"
          />

          <PolicyScopeEditor
            v-if="needsCommandPolicy"
            v-model="commandPolicyScopes"
            title="Executables this plugin can manage"
            description="The plugin can publish only the selected decisions for these executable scopes."
            path-label="Executable scope"
            placeholder="/usr/bin/**"
            path-hint="Use an exact absolute executable path or a directory ending in /**."
            :busy="busy"
            @blur="showValidation = true"
          />

          <PolicyScopeEditor
            v-if="needsNetworkPolicy"
            v-model="networkPolicyScopes"
            title="Remote endpoints this plugin can manage"
            description="The plugin can publish only the selected decisions for these numeric endpoint scopes."
            path-label="Remote endpoint scope"
            placeholder="203.0.113.10:443, 203.0.113.10:* or *"
            path-hint="Use *, an exact numeric endpoint, or IP:* for every port on one IP; bracket IPv6 addresses."
            add-label="Add another endpoint"
            :busy="busy"
            @blur="showValidation = true"
          />

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
import { X } from '@lucide/vue';
import PolicyScopeEditor from './PolicyScopeEditor.vue';

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
    .filter(Boolean).length > 1) return 'Configure policy access';
  if (needsNetworkPolicy.value) return 'Configure network connections';
  if (needsCommandPolicy.value) return 'Configure command execution';
  if (needsFilePolicy.value) return 'Configure file access';
  return 'Load plugin';
});
const envRead = computed(() => envReadText.value
  .split('\n')
  .map((name) => name.trim())
  .filter(Boolean));
const validationError = computed(() => {
  if (!instanceId.value || instanceId.value.trim() !== instanceId.value) {
    return 'Instance ID is required and cannot have surrounding whitespace.';
  }
  const fileScopeError = needsFilePolicy.value
    ? validateScopes(filePolicyScopes.value, 'file-policy')
    : '';
  if (fileScopeError) return fileScopeError;
  const commandScopeError = needsCommandPolicy.value
    ? validateScopes(commandPolicyScopes.value, 'command-policy')
    : '';
  if (commandScopeError) return commandScopeError;
  const networkScopeError = needsNetworkPolicy.value
    ? validateNetworkScopes(networkPolicyScopes.value)
    : '';
  if (networkScopeError) return networkScopeError;
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
      return `Every ${label} scope must be an absolute path.`;
    }
    if (scope.decisions.length === 0) {
      return `Select at least one rule decision for every ${label} scope.`;
    }
  }
  return '';
}

function validateNetworkScopes(scopes) {
  for (const scope of scopes) {
    if (!scope.path_scope || (scope.path_scope !== '*' && !looksLikeNumericRemoteScope(scope.path_scope))) {
      return 'Every network-policy scope must be *, a numeric IP endpoint, or an IP:* any-port selector.';
    }
    if (scope.decisions.length === 0) {
      return 'Select at least one rule decision for every network-policy scope.';
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
.plugin-load-permissions small {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.plugin-load-header h2 {
  margin: var(--stats-space-2xs) 0 0;
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-medium);
}

.plugin-load-header button,
.plugin-load-actions button {
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
.plugin-load-section-heading > div {
  display: grid;
  gap: var(--stats-space-xs);
}

.plugin-load-field > span,
.plugin-load-section-heading span {
  color: var(--stats-text);
  font-size: var(--stats-font-md);
  font-weight: var(--stats-weight-medium);
}

.plugin-load-field input,
.plugin-load-field textarea {
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
