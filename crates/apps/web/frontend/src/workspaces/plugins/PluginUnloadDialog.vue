<template>
  <Teleport to=".app-shell">
    <div v-if="open" class="plugin-unload-backdrop" @mousedown.self="close">
      <section
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
            <span>{{ t('pluginUnload.kicker') }}</span>
            <h2 :id="titleId">{{ plugin.instance_id }}</h2>
          </div>
        </header>

        <div class="plugin-unload-body">
          <p :id="descriptionId">
            {{ t('pluginUnload.description') }}
          </p>
          <dl>
            <dt>{{ t('pluginUnload.instanceId') }}</dt>
            <dd>{{ plugin.instance_id }}</dd>
            <dt>{{ t('pluginUnload.pluginId') }}</dt>
            <dd>{{ plugin.plugin_id }}</dd>
            <dt>{{ t('pluginUnload.purpose') }}</dt>
            <dd>{{ plugin.purpose }}</dd>
          </dl>
        </div>

        <footer>
          <button type="button" :disabled="busy" @click="close">{{ t('pluginUnload.keepLoaded') }}</button>
          <button ref="confirmButton" class="danger" type="button" :disabled="busy" @click="$emit('confirm')">
            {{ busy ? t('pluginUnload.unloading') : t('pluginUnload.unload') }}
          </button>
        </footer>
      </section>
    </div>
  </Teleport>
</template>

<script setup>
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { TriangleAlert } from '@lucide/vue';

import { useLocale } from '../../locale';

const props = defineProps({
  open: { type: Boolean, required: true },
  plugin: { type: Object, required: true },
  busy: { type: Boolean, default: false },
});

const emit = defineEmits(['close', 'confirm']);
const { t } = useLocale();
const confirmButton = ref(null);
const titleId = 'plugin-unload-title';
const descriptionId = 'plugin-unload-description';

watch(
  () => props.open,
  async (open) => {
    if (open) {
      await nextTick();
      confirmButton.value?.focus();
    }
  },
  { immediate: true },
);

onMounted(() => window.addEventListener('keydown', onKeydown));
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown));

function close() {
  if (!props.busy) emit('close');
}

function onKeydown(event) {
  if (event.key === 'Escape' && props.open) close();
}
</script>

<style scoped>
.plugin-unload-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: grid;
  place-items: center;
  padding: var(--stats-space-xl);
  background: rgb(4 9 18 / 72%);
  backdrop-filter: blur(0.25rem);
}

.plugin-unload-dialog {
  min-width: 0;
  width: min(34rem, 100%);
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--stats-danger) 45%, var(--stats-border-strong));
  border-radius: var(--stats-radius-lg);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  box-shadow: 0 1.5rem 5rem rgb(0 0 0 / 42%);
}

.plugin-unload-dialog header,
.plugin-unload-dialog footer {
  display: flex;
  align-items: center;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-xl) var(--stats-space-2xl);
}

.plugin-unload-dialog header {
  border-bottom: 1px solid var(--stats-border);
}

.plugin-unload-icon {
  width: var(--stats-control-height-lg);
  height: var(--stats-control-height-lg);
  flex: 0 0 auto;
  display: grid;
  place-items: center;
  border-radius: 50%;
  background: color-mix(in srgb, var(--stats-danger) 12%, transparent);
  color: var(--stats-danger);
}

.plugin-unload-dialog header > div {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-2xs);
}

.plugin-unload-dialog header span,
.plugin-unload-body p,
.plugin-unload-body dt {
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
}

.plugin-unload-dialog h2 {
  margin: 0;
  overflow-wrap: anywhere;
  font-size: var(--stats-font-display-sm);
  font-weight: var(--stats-weight-medium);
}

.plugin-unload-body {
  display: grid;
  gap: var(--stats-space-xl);
  padding: var(--stats-space-2xl);
}

.plugin-unload-body p,
.plugin-unload-body dl {
  margin: 0;
}

.plugin-unload-body dl {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: var(--stats-space-md) var(--stats-space-xl);
}

.plugin-unload-body dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}

.plugin-unload-dialog footer {
  justify-content: flex-end;
  border-top: 1px solid var(--stats-border);
  background: var(--stats-surface-soft);
}

.plugin-unload-dialog footer button {
  min-height: var(--stats-control-height-md);
  padding: 0 var(--stats-space-lg);
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
  color: var(--stats-text);
  cursor: pointer;
  font: inherit;
  font-weight: var(--stats-weight-medium);
}

.plugin-unload-dialog footer button.danger {
  border-color: color-mix(in srgb, var(--stats-danger) 45%, transparent);
  background: color-mix(in srgb, var(--stats-danger) 12%, transparent);
  color: var(--stats-danger);
}

.plugin-unload-dialog footer button:focus-visible {
  outline: 2px solid var(--stats-accent);
  outline-offset: var(--stats-space-xs);
}

.plugin-unload-dialog footer button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
</style>
