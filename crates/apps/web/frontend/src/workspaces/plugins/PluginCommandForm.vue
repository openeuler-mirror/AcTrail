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
          <strong>Plugin command</strong>
          <small>{{ commandSupported ? 'Send a supported management command' : 'Not supported by this plugin type' }}</small>
        </span>
      </span>
      <ChevronDown v-if="commandSupported" :size="17" :class="{ rotated: opened }" aria-hidden="true" />
      <LockKeyhole v-else :size="16" aria-hidden="true" />
    </button>

    <p v-if="!commandSupported" class="plugin-command-unsupported">
      Observation plugins consume trace data and do not expose management commands. Configure this instance through the Configuration panel above.
    </p>

    <form v-else-if="opened" class="plugin-command" @submit.prevent="sendCommand">
      <div class="plugin-command-heading">
        <label :for="inputId">Command arguments</label>
        <code>{{ instanceId }}</code>
      </div>
      <p>Enter one argument per line. Send <code>help</code> to list supported operations.</p>
      <textarea
        :id="inputId"
        v-model="commandText"
        rows="4"
        :placeholder="'help'"
        :disabled="sending"
      ></textarea>
      <div class="plugin-command-actions">
        <button type="submit" :disabled="sending || argv.length === 0">
          {{ sending ? 'Sending…' : 'Send command' }}
        </button>
      </div>
      <p v-if="error" class="plugin-command-error">{{ error }}</p>
      <section v-if="result" class="plugin-command-result" aria-live="polite">
        <strong>Exit code {{ result.exit_code }}</strong>
        <div v-if="result.stdout">
          <span>stdout</span>
          <pre>{{ result.stdout }}</pre>
        </div>
        <div v-if="result.stderr">
          <span>stderr</span>
          <pre>{{ result.stderr }}</pre>
        </div>
        <p v-if="!result.stdout && !result.stderr">The plugin returned no output.</p>
      </section>
    </form>
  </section>
</template>

<script setup>
import { computed, ref } from 'vue';
import { ChevronDown, LockKeyhole, SquareTerminal } from '@lucide/vue';

import { sendRuntimePluginCommand } from '../../api';

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
  margin: 0 var(--stats-space-2xl) var(--stats-space-2xl) calc(var(--stats-space-2xl) + var(--stats-space-lg));
  overflow: hidden;
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface-soft);
  color: var(--stats-text);
  font-size: var(--stats-font-md);
}

.plugin-command-toggle {
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

.plugin-command-toggle:hover {
  background: var(--stats-surface-bar);
}

.plugin-command-toggle:focus-visible,
.plugin-command textarea:focus-visible,
.plugin-command-actions button:focus-visible {
  outline: 2px solid var(--stats-accent);
  outline-offset: calc(-1 * var(--stats-space-xs));
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
  gap: var(--stats-space-md);
}

.plugin-command-toggle-title > span {
  align-items: flex-start;
  flex-direction: column;
  gap: var(--stats-space-2xs);
}

.plugin-command-toggle strong,
.plugin-command label,
.plugin-command-result > strong {
  font-weight: var(--stats-weight-medium);
}

.plugin-command-toggle small {
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}

.plugin-command-toggle > svg {
  color: var(--stats-muted);
  transition: transform 120ms ease;
}

.plugin-command-toggle > svg.rotated {
  transform: rotate(180deg);
}

.plugin-command-unsupported {
  margin: 0;
  padding: 0 var(--stats-space-lg) var(--stats-space-lg) calc(1.0625rem + var(--stats-space-lg) + var(--stats-space-md));
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  line-height: 1.5;
}

.plugin-command {
  display: grid;
  gap: var(--stats-space-md);
  padding: var(--stats-space-2xl);
  border: 0;
  border-top: 1px solid var(--stats-border);
  border-radius: 0;
  background: var(--stats-surface);
}

.plugin-command-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-lg);
}

.plugin-command-heading code {
  color: var(--stats-muted);
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: var(--stats-font-xs);
}

.plugin-command > p,
.plugin-command-result span {
  margin: 0;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.plugin-command > p code {
  color: var(--stats-text);
  font-size: inherit;
}

.plugin-command textarea {
  width: 100%;
  resize: vertical;
  padding: var(--stats-space-md);
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
  color: var(--stats-text);
  font: inherit;
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
}

.plugin-command-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--stats-space-lg);
}

.plugin-command-actions button {
  min-height: var(--stats-control-height-md);
  padding: 0 var(--stats-space-lg);
  border: 1px solid var(--stats-accent-soft);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-accent-muted);
  color: var(--stats-accent);
  cursor: pointer;
  font: inherit;
  font-weight: var(--stats-weight-medium);
}

.plugin-command-actions button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.plugin-command-error {
  color: var(--stats-danger) !important;
}

.plugin-command-result {
  display: grid;
  gap: var(--stats-space-sm);
  padding: var(--stats-space-md);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-soft);
}

.plugin-command-result pre {
  max-height: 15rem;
  margin: var(--stats-space-xs) 0 0;
  overflow: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
}

@media (max-width: 47.5rem) {
  .plugin-command-panel {
    margin-right: var(--stats-space-xl);
    margin-left: var(--stats-space-xl);
  }

  .plugin-command-heading {
    align-items: flex-start;
    flex-direction: column;
  }
}
</style>
