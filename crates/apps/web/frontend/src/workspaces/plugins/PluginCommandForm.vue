<template>
  <section class="plugin-command-panel">
    <button
      class="plugin-command-toggle"
      :class="{ unsupported: !commandSupported }"
      type="button"
      :disabled="!commandSupported"
      @click="opened = !opened"
    >
      <span class="plugin-command-toggle-title">
        <SquareTerminal :size="17" aria-hidden="true" />
        <span>
          <strong>{{ t('command.title') }}</strong>
          <small>{{ commandSupported ? t('command.subtitleSupported') : t('command.subtitleUnsupported') }}</small>
        </span>
      </span>
      <ChevronDown v-if="commandSupported" :size="17" :class="{ rotated: opened }" aria-hidden="true" />
      <LockKeyhole v-else :size="16" aria-hidden="true" />
    </button>

    <p v-if="!commandSupported" class="plugin-command-unsupported">
      {{ t('command.unsupported') }}
    </p>

    <form v-else-if="opened" class="plugin-command" @submit.prevent="sendCommand">
      <div class="plugin-command-heading">
        <label :for="inputId">{{ t('command.arguments') }}</label>
        <code>{{ instanceId }}</code>
      </div>
      <p>{{ t('command.argumentsHint') }} <code>help</code> {{ t('command.argumentsHintTail') }}</p>
      <textarea
        :id="inputId"
        v-model="commandText"
        rows="4"
        :placeholder="'help'"
        :disabled="sending"
      ></textarea>
      <div class="plugin-command-actions">
        <button type="submit" :disabled="sending || argv.length === 0">
          {{ sending ? t('command.sending') : t('command.send') }}
        </button>
      </div>
      <p v-if="error" class="plugin-command-error">{{ error }}</p>
      <section v-if="result" class="plugin-command-result" aria-live="polite">
        <strong>{{ t('command.exitCode', { code: result.exit_code }) }}</strong>
        <div v-if="result.stdout">
          <span>stdout</span>
          <pre>{{ result.stdout }}</pre>
        </div>
        <div v-if="result.stderr">
          <span>stderr</span>
          <pre>{{ result.stderr }}</pre>
        </div>
        <p v-if="!result.stdout && !result.stderr">{{ t('command.noOutput') }}</p>
      </section>
    </form>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue';
import { ChevronDown, LockKeyhole, SquareTerminal } from '@lucide/vue';

import { sendRuntimePluginCommand } from '../../api';
import { useModuleLocale } from '../../locale';
import strings from './locale';

const { t } = useModuleLocale(strings);

const props = defineProps({
  instanceId: {
    type: String,
    required: true,
  },
  purpose: {
    type: String,
    required: true,
  },
});
const emit = defineEmits(['completed']);

const commandText = ref('help');
const opened = ref(false);
const sending = ref(false);
const result = ref(null);
const error = ref('');
const inputId = computed(() => `plugin-command-${props.instanceId}`);
const commandSupported = computed(() => props.purpose === 'control-decider');
const argv = computed(() => commandText.value
  .split('\n')
  .map((argument) => argument.trim())
  .filter((argument) => argument.length > 0));

async function sendCommand() {
  sending.value = true;
  result.value = null;
  error.value = '';
  try {
    const response = await sendRuntimePluginCommand(props.instanceId, argv.value);
    result.value = response.command;
    if (response.command.exit_code === 0) {
      emit('completed');
    }
  } catch (err) {
    error.value = String(err.message ?? err);
  } finally {
    sending.value = false;
  }
}
</script>

<style scoped>
.plugin-command-panel {
  display: grid;
  margin: 0 var(--ui-space-2xl) var(--ui-space-2xl) calc(var(--ui-space-2xl) + var(--ui-space-lg));
  overflow: hidden;
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface-soft);
  color: var(--ui-text);
  font-size: var(--ui-font-md);
}

.plugin-command-toggle {
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

.plugin-command-toggle:hover {
  background: var(--ui-surface-bar);
}

.plugin-command-toggle:focus-visible,
.plugin-command textarea:focus-visible,
.plugin-command-actions button:focus-visible {
  outline: 2px solid var(--ui-accent);
  outline-offset: calc(-1 * var(--ui-space-xs));
}

.plugin-command-toggle.unsupported {
  cursor: not-allowed;
}

.plugin-command-toggle-title,
.plugin-command-toggle-title > span {
  display: flex;
  align-items: center;
}

.plugin-command-toggle-title {
  gap: var(--ui-space-md);
}

.plugin-command-toggle-title > span {
  align-items: flex-start;
  flex-direction: column;
  gap: var(--ui-space-2xs);
}

.plugin-command-toggle strong,
.plugin-command label,
.plugin-command-result > strong {
  font-weight: var(--ui-weight-medium);
}

.plugin-command-toggle small {
  color: var(--ui-muted);
  font-size: var(--ui-font-xs);
}

.plugin-command-toggle > svg {
  color: var(--ui-muted);
  transition: transform 120ms ease;
}

.plugin-command-toggle > svg.rotated {
  transform: rotate(180deg);
}

.plugin-command-unsupported {
  margin: 0;
  padding: 0 var(--ui-space-lg) var(--ui-space-lg) calc(1.0625rem + var(--ui-space-lg) + var(--ui-space-md));
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  line-height: 1.5;
}

.plugin-command {
  display: grid;
  gap: var(--ui-space-md);
  padding: var(--ui-space-2xl);
  border: 0;
  border-top: 1px solid var(--ui-border);
  border-radius: 0;
  background: var(--ui-surface);
}

.plugin-command-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-lg);
}

.plugin-command-heading code {
  color: var(--ui-muted);
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: var(--ui-font-xs);
}

.plugin-command > p,
.plugin-command-result span {
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.plugin-command > p code {
  color: var(--ui-text);
  font-size: inherit;
}

.plugin-command textarea {
  width: 100%;
  resize: vertical;
  padding: var(--ui-space-md);
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
  color: var(--ui-text);
  font: inherit;
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
}

.plugin-command-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--ui-space-lg);
}

.plugin-command-actions button {
  min-height: var(--ui-control-height-md);
  padding: 0 var(--ui-space-lg);
  border: 1px solid var(--ui-accent-soft);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-accent-muted);
  color: var(--ui-accent);
  cursor: pointer;
  font: inherit;
  font-weight: var(--ui-weight-medium);
}

.plugin-command-actions button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.plugin-command-error {
  color: var(--ui-danger) !important;
}

.plugin-command-result {
  display: grid;
  gap: var(--ui-space-sm);
  padding: var(--ui-space-md);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-soft);
}

.plugin-command-result pre {
  max-height: 15rem;
  margin: var(--ui-space-xs) 0 0;
  overflow: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
}

@media (max-width: 47.5rem) {
  .plugin-command-panel {
    margin-right: var(--ui-space-xl);
    margin-left: var(--ui-space-xl);
  }

  .plugin-command-heading {
    align-items: flex-start;
    flex-direction: column;
  }
}
</style>
