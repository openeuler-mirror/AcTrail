<template>
  <Teleport to=".app-shell">
    <Transition name="dialog">
    <div v-if="open" class="plugin-unload-backdrop" @mousedown.self="close">
      <section
        ref="dialogRef"
        class="plugin-unload-dialog"
        role="alertdialog"
        aria-modal="true"
        :aria-labelledby="titleId"
        :aria-describedby="descriptionId"
      >
        <header>
          <span class="plugin-unload-icon" aria-hidden="true">
            <TriangleAlert :size="20" />
          </span>
          <div>
            <span>{{ t('unload.kicker') }}</span>
            <h2 :id="titleId">{{ plugin.instance_id }}</h2>
          </div>
        </header>

        <div class="plugin-unload-body">
          <p :id="descriptionId">
            {{ t('unload.description') }}
          </p>
          <dl>
            <dt>{{ t('unload.instanceId') }}</dt>
            <dd>{{ plugin.instance_id }}</dd>
            <dt>{{ t('unload.pluginId') }}</dt>
            <dd>{{ plugin.plugin_id }}</dd>
            <dt>{{ t('unload.purpose') }}</dt>
            <dd>{{ plugin.purpose }}</dd>
          </dl>
        </div>

        <footer>
          <button type="button" :disabled="busy" @click="close">{{ t('unload.keepLoaded') }}</button>
          <button ref="confirmButton" class="danger" type="button" :disabled="busy" @click="$emit('confirm')">
            {{ busy ? t('unload.unloading') : t('unload.unload') }}
          </button>
        </footer>
      </section>
    </div>
    </Transition>
  </Teleport>
</template>

<script setup>
import { ref } from 'vue';
import { TriangleAlert } from '@lucide/vue';

import { useDialogBehavior } from '../../composables/dialog';
import { useModuleLocale } from '../../locale';
import strings from './locale';

const props = defineProps({
  open: { type: Boolean, required: true },
  plugin: { type: Object, required: true },
  busy: { type: Boolean, default: false },
});

const emit = defineEmits(['close', 'confirm']);
const { t } = useModuleLocale(strings);
const confirmButton = ref(null);
const dialogRef = ref(null);
const titleId = 'plugin-unload-title';
const descriptionId = 'plugin-unload-description';

useDialogBehavior({
  isOpen: () => props.open,
  getContainer: () => dialogRef.value,
  onClose: close,
  initialFocus: confirmButton,
});

function close() {
  if (!props.busy) emit('close');
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

.dialog-enter-active .plugin-unload-dialog {
  animation: dialog-pop 300ms cubic-bezier(0.34, 1.56, 0.64, 1);
}

.dialog-leave-active .plugin-unload-dialog {
  animation: dialog-fold 200ms cubic-bezier(0.32, 0, 0.67, 0) forwards;
}

@keyframes dialog-pop {
  from {
    opacity: 0;
    transform: scale(0.9) translateY(14px);
  }

  to {
    opacity: 1;
    transform: none;
  }
}

@keyframes dialog-fold {
  to {
    opacity: 0;
    transform: scale(0.94) translateY(8px);
  }
}

.plugin-unload-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: grid;
  place-items: center;
  padding: var(--ui-space-xl);
  background: rgb(4 9 18 / 72%);
  backdrop-filter: blur(0.25rem);
}

.plugin-unload-dialog {
  min-width: 0;
  width: min(34rem, 100%);
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--ui-danger) 45%, var(--ui-border-strong));
  border-radius: var(--ui-radius-lg);
  background: var(--ui-surface-strong);
  color: var(--ui-text);
  box-shadow: 0 1.5rem 5rem rgb(0 0 0 / 42%);
}

.plugin-unload-dialog header,
.plugin-unload-dialog footer {
  display: flex;
  align-items: center;
  gap: var(--ui-space-lg);
  padding: var(--ui-space-xl) var(--ui-space-2xl);
}

.plugin-unload-dialog header {
  border-bottom: 1px solid var(--ui-border);
}

.plugin-unload-icon {
  width: var(--ui-control-height-lg);
  height: var(--ui-control-height-lg);
  flex: 0 0 auto;
  display: grid;
  place-items: center;
  border-radius: 50%;
  background: color-mix(in srgb, var(--ui-danger) 12%, transparent);
  color: var(--ui-danger);
}

.plugin-unload-dialog header > div {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-2xs);
}

.plugin-unload-dialog header span,
.plugin-unload-body p,
.plugin-unload-body dt {
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.plugin-unload-dialog h2 {
  margin: 0;
  overflow-wrap: anywhere;
  font-size: var(--ui-font-display-sm);
  font-weight: var(--ui-weight-medium);
}

.plugin-unload-body {
  display: grid;
  gap: var(--ui-space-xl);
  padding: var(--ui-space-2xl);
}

.plugin-unload-body p,
.plugin-unload-body dl {
  margin: 0;
}

.plugin-unload-body dl {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: var(--ui-space-md) var(--ui-space-xl);
}

.plugin-unload-body dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}

.plugin-unload-dialog footer {
  justify-content: flex-end;
  border-top: 1px solid var(--ui-border);
  background: var(--ui-surface-soft);
}

.plugin-unload-dialog footer button {
  min-height: var(--ui-control-height-md);
  padding: 0 var(--ui-space-lg);
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
  color: var(--ui-text);
  cursor: pointer;
  font: inherit;
  font-weight: var(--ui-weight-medium);
}

.plugin-unload-dialog footer button.danger {
  border-color: color-mix(in srgb, var(--ui-danger) 45%, transparent);
  background: color-mix(in srgb, var(--ui-danger) 12%, transparent);
  color: var(--ui-danger);
}

.plugin-unload-dialog footer button:focus-visible {
  outline: 2px solid var(--ui-accent);
  outline-offset: var(--ui-space-xs);
}

.plugin-unload-dialog footer button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
</style>
