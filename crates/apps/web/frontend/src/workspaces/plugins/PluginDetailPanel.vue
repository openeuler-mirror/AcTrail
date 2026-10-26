<template>
  <Teleport to="body">
  <aside class="plugin-detail" :aria-label="t('detail.aria', { id: row.pluginId })">
    <header class="plugin-detail-header">
      <span class="plugin-detail-tile" aria-hidden="true"><Puzzle :size="18" /></span>
      <div class="plugin-detail-title">
        <span>{{ row.purposeLabel }}</span>
        <h2>{{ row.pluginId }}</h2>
      </div>
      <button class="plugin-detail-close" type="button" :aria-label="t('detail.close')" @click="$emit('close')">
        <X :size="18" aria-hidden="true" />
      </button>
    </header>

    <div class="plugin-detail-body">
      <section class="plugin-detail-section">
        <h3>{{ t('detail.package') }}</h3>
        <dl class="plugin-detail-rows">
          <dt>{{ t('detail.packageKey') }}</dt>
          <dd>{{ row.packageKey }}</dd>
          <dt>{{ t('detail.runtime') }}</dt>
          <dd>{{ row.package.runtime ?? t('detail.unknown') }}</dd>
          <dt>{{ t('detail.manifest') }}</dt>
          <dd class="plugin-detail-path">{{ row.package.manifest_path ?? t('detail.missing') }}</dd>
          <dt>{{ t('detail.configPath') }}</dt>
          <dd class="plugin-detail-path">{{ row.package.plugin_config_path ?? t('detail.none') }}</dd>
          <dt>{{ t('detail.capabilities') }}</dt>
          <dd><PluginGrantList :items="row.package.requested_capabilities" /></dd>
          <dt>{{ t('detail.automaticGrants') }}</dt>
          <dd><PluginGrantList :items="row.package.automatic_host_grants" /></dd>
          <dt>{{ t('detail.loadable') }}</dt>
          <dd>{{ row.package.activation_ready ? t('detail.ready') : row.package.issue ?? t('detail.notLoadable') }}</dd>
        </dl>
      </section>

      <section v-if="instances.length" class="plugin-detail-section">
        <h3>{{ t('detail.instances') }}</h3>
        <article v-for="instance in instances" :key="instance.instance_id" class="plugin-detail-instance">
          <header>
            <strong>{{ instance.instance_id }}</strong>
            <span class="plugin-runtime-state" :class="{ active: instance.state === 'active' }">
              <i aria-hidden="true"></i>{{ instance.state }}
            </span>
          </header>
          <dl class="plugin-detail-rows">
            <dt>{{ t('detail.records') }}</dt>
            <dd>{{ t('detail.recordsValue', { observed: instance.observed_records ?? 0, dropped: instance.dropped_records ?? 0 }) }}</dd>
            <dt>{{ t('detail.queue') }}</dt>
            <dd>{{ instance.queue_depth ?? 0 }} / {{ instance.queue_capacity ?? 0 }}</dd>
            <dt>{{ t('detail.hostGrants') }}</dt>
            <dd><PluginGrantList :items="instance.host_grants" /></dd>
            <dt>{{ t('detail.lastError') }}</dt>
            <dd>{{ instance.last_error ?? t('detail.none') }}</dd>
          </dl>
          <PluginCommandForm
            :instance-id="instance.instance_id"
            :purpose="instance.purpose"
            @completed="$emit('config-changed', instance.instance_id)"
          />
          <PluginConfigPanel
            :instance-id="instance.instance_id"
            :plugin-id="instance.plugin_id"
            :refresh-nonce="configNonce(instance.instance_id)"
            @updated="$emit('config-updated')"
          />
          <div class="plugin-detail-actions">
            <button
              v-if="canUnload(instance)"
              class="plugin-lifecycle-action danger"
              type="button"
              @click="$emit('unload', instance)"
            >
              {{ t('detail.unloadInstance') }}
            </button>
          </div>
        </article>
      </section>

      <p v-else class="plugin-detail-note">
        {{ t('detail.noInstance') }}
      </p>
    </div>
  </aside>
  </Teleport>
</template>

<script setup>
import { Puzzle, X } from '@lucide/vue';

import { useModuleLocale } from '../../locale';
import strings from './locale';
import PluginConfigPanel from './PluginConfigPanel.vue';
import PluginCommandForm from './PluginCommandForm.vue';
import PluginGrantList from './PluginGrantList.vue';

const { t } = useModuleLocale(strings);

const props = defineProps({
  row: { type: Object, required: true },
  instances: { type: Array, default: () => [] },
  configNonces: { type: Object, default: () => ({}) },
});

defineEmits(['close', 'unload', 'config-changed', 'config-updated']);

function configNonce(instanceId) {
  return props.configNonces?.[instanceId] ?? 0;
}

function canUnload(instance) {
  return instance.purpose !== 'alert-consumer';
}
</script>

<style scoped>
.plugin-detail {
  position: fixed;
  top: 0;
  right: 0;
  z-index: 1200;
  width: min(560px, 100vw);
  height: 100vh;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  border-left: 1px solid var(--ui-border);
  background: var(--ui-bg-base);
  box-shadow: -8px 0 32px rgba(0, 0, 0, 0.16);
  animation: plugin-detail-in 320ms cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes plugin-detail-in {
  from {
    opacity: 0.4;
    transform: translateX(24px);
  }

  to {
    opacity: 1;
    transform: none;
  }
}

.plugin-detail-header {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--ui-space-md);
  padding: var(--ui-space-lg) var(--ui-space-2xl);
  border-bottom: 1px solid var(--ui-border);
  background: var(--ui-bg-shell);
}

.plugin-detail-tile {
  width: 34px;
  height: 34px;
  display: grid;
  place-items: center;
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-soft);
  color: var(--ui-muted);
}

.plugin-detail-title {
  min-width: 0;
}

.plugin-detail-title span {
  color: var(--ui-muted);
  font-size: 11px;
  font-weight: var(--ui-weight-semibold);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.plugin-detail-title h2 {
  margin: 2px 0 0;
  overflow: hidden;
  font-size: var(--ui-font-title);
  font-weight: var(--ui-weight-semibold);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plugin-detail-close {
  width: 32px;
  height: 32px;
  display: grid;
  place-items: center;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-sm);
  background: transparent;
  color: var(--ui-muted);
  cursor: pointer;
}

.plugin-detail-close:hover {
  border-color: var(--ui-border);
  background: var(--ui-surface-soft);
  color: var(--ui-text);
}

.plugin-detail-body {
  min-height: 0;
  overflow-y: auto;
  padding: var(--ui-space-2xl);
  display: grid;
  align-content: start;
  gap: var(--ui-space-2xl);
}

.plugin-detail-section h3 {
  margin: 0 0 var(--ui-space-md);
  color: var(--ui-muted);
  font-size: 11px;
  font-weight: var(--ui-weight-semibold);
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.plugin-detail-rows {
  display: grid;
  grid-template-columns: minmax(96px, auto) minmax(0, 1fr);
  gap: 8px 14px;
  margin: 0;
  font-size: var(--ui-font-sm);
}

.plugin-detail-rows dt {
  color: var(--ui-muted);
}

.plugin-detail-rows dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}

.plugin-detail-path {
  font-family: var(--ui-mono);
  font-size: var(--ui-font-xs);
}

.plugin-detail-instance {
  display: grid;
  gap: var(--ui-space-md);
  margin-bottom: var(--ui-space-2xl);
  padding: var(--ui-space-lg);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-lg);
  background: var(--ui-surface);
}

.plugin-detail-instance > header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-md);
}

.plugin-detail-note {
  margin: 0;
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
  line-height: 1.5;
}

.plugin-detail-actions {
  display: flex;
  justify-content: flex-end;
}
</style>
