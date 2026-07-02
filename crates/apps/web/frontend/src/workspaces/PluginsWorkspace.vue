<template>
  <main class="runtime-workspace">
    <div class="runtime-content">
      <section class="runtime-hero">
        <div>
          <span>Startup Plugins</span>
          <h2>Plugin enablement</h2>
        </div>
        <div class="runtime-source">{{ sourceLabel }}</div>
      </section>

      <section class="runtime-metrics">
        <div v-for="metric in metrics" :key="metric.label" class="runtime-metric">
          <span>{{ metric.label }}</span>
          <strong>{{ metric.value }}</strong>
        </div>
      </section>

      <div v-if="loading && !plugins" class="runtime-panel loading-panel">
        <span class="loading-spinner" aria-hidden="true"></span>
        <p>Loading plugin status...</p>
      </div>

      <section v-else-if="!plugins?.available" class="runtime-panel runtime-empty">
        <h2>Plugin status unavailable</h2>
        <p>{{ plugins?.reason ?? error }}</p>
      </section>

      <section v-else class="plugins-layout">
        <aside class="runtime-panel runtime-side">
          <div class="runtime-side-heading">Source</div>
          <dl class="runtime-rows">
            <dt>Mode</dt>
            <dd>{{ plugins.source?.mode ?? 'unknown' }}</dd>
            <dt>Path</dt>
            <dd>{{ plugins.source?.path ?? 'n/a' }}</dd>
          </dl>

          <div class="runtime-side-heading">Startup</div>
          <dl class="runtime-rows">
            <dt>Global</dt>
            <dd>{{ plugins.global_enabled ? 'Enabled' : 'Disabled' }}</dd>
            <dt>Policy</dt>
            <dd>{{ plugins.global_failure_policy }}</dd>
            <dt>Configured</dt>
            <dd>{{ plugins.configured_count }}</dd>
            <dt>Effective</dt>
            <dd>{{ plugins.enabled_count }}</dd>
          </dl>

          <details class="startup-plugin-details">
            <summary>
              <span>Startup load list</span>
              <strong>{{ filteredPlugins.length }}</strong>
            </summary>
            <div v-if="filteredPlugins.length === 0" class="startup-plugin-empty">None</div>
            <ul v-else class="startup-plugin-list">
              <li v-for="plugin in filteredPlugins" :key="plugin.instance_id">
                <div>
                  <strong>{{ plugin.instance_id }}</strong>
                  <span>{{ plugin.effective_enabled ? 'Enabled' : 'Disabled' }}</span>
                </div>
                <code>{{ plugin.manifest_path }}</code>
              </li>
            </ul>
          </details>
        </aside>

        <div class="plugins-main">
          <section class="runtime-panel plugins-panel">
            <header class="runtime-panel-header">
              <div>
                <span>Runtime instances</span>
                <strong>{{ runtimeSummary }}</strong>
              </div>
              <button
                class="runtime-icon-button"
                type="button"
                :disabled="loading"
                title="Refresh plugin status"
                aria-label="Refresh plugin status"
                @click="loadPlugins"
              >
                <RefreshCw :size="16" aria-hidden="true" />
              </button>
            </header>
            <div v-if="!runtimeStatus?.available" class="runtime-inline-empty">
              <strong>Runtime status unavailable</strong>
              <span>{{ runtimeStatus?.reason ?? 'No runtime status loaded' }}</span>
            </div>
            <div v-else-if="filteredRuntimePlugins.length === 0" class="runtime-compact-empty">
              <strong>No runtime plugins</strong>
              <span>The daemon currently reports no loaded plugin instances.</span>
            </div>
            <div v-else class="plugin-runtime-list">
              <article
                v-for="plugin in filteredRuntimePlugins"
                :key="plugin.instance_id"
                class="plugin-runtime-item"
              >
                <details class="plugin-runtime-disclosure">
                  <summary class="plugin-runtime-summary">
                    <span class="plugin-runtime-main">
                      <strong>{{ plugin.instance_id }}</strong>
                      <span>{{ plugin.plugin_id }}</span>
                    </span>
                    <span class="plugin-runtime-badges">
                      <span
                        class="state-switch"
                        :class="{ active: plugin.state === 'active' }"
                        role="switch"
                        :aria-checked="plugin.state === 'active'"
                        aria-disabled="true"
                      >
                        <span class="state-switch-track" aria-hidden="true">
                          <span class="state-switch-thumb"></span>
                        </span>
                        <span>{{ plugin.state }}</span>
                      </span>
                      <span class="plugin-runtime-chip primary">{{ purposeLabel(plugin.purpose) }}</span>
                      <span class="plugin-runtime-chip">{{ plugin.runtime }}</span>
                    </span>
                  </summary>
                  <dl class="plugin-runtime-details">
                    <dt>Purpose</dt>
                    <dd>{{ plugin.purpose }}</dd>
                    <dt>Records</dt>
                    <dd>{{ recordsText(plugin) }}</dd>
                    <dt>Queue</dt>
                    <dd>{{ queueText(plugin) }}</dd>
                    <dt>Host grants</dt>
                    <dd>{{ hostGrantText(plugin.host_grants) }}</dd>
                    <dt>Payload reads</dt>
                    <dd>{{ payloadReadText(plugin) }}</dd>
                    <dt>Last error</dt>
                    <dd>{{ plugin.last_error ?? 'none' }}</dd>
                    <dt>Warnings</dt>
                    <dd>{{ warningText(plugin.warnings) }}</dd>
                  </dl>
                </details>
                <button
                  class="runtime-danger-icon-button"
                  type="button"
                  :disabled="loading || unloadingInstances[plugin.instance_id]"
                  title="Unload plugin"
                  aria-label="Unload plugin"
                  @click="unloadPlugin(plugin.instance_id)"
                >
                  <Trash2 :size="16" aria-hidden="true" />
                </button>
              </article>
            </div>
          </section>
        </div>
      </section>
    </div>

    <div v-if="error" class="error-bar">{{ error }}</div>
  </main>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { RefreshCw, Trash2 } from '@lucide/vue';

import { readPluginEnablement, readPluginRuntimeStatus, unloadRuntimePlugin } from '../api';

const props = defineProps({
  query: {
    type: String,
    default: '',
  },
  refreshNonce: {
    type: Number,
    default: 0,
  },
});

const emit = defineEmits(['loading']);

const plugins = ref(null);
const runtimeStatus = ref(null);
const error = ref('');
const loading = ref(false);
const unloadingInstances = ref({});
let activeLoad = null;

const metrics = computed(() => [
  { label: 'Startup', value: plugins.value?.global_enabled ? 'Enabled' : 'Disabled' },
  { label: 'Configured', value: plugins.value?.configured_count ?? 0 },
  { label: 'Effective', value: plugins.value?.enabled_count ?? 0 },
  { label: 'Runtime active', value: runtimeStatus.value?.active_count ?? 0 },
]);

const filteredPlugins = computed(() => {
  const rows = plugins.value?.plugins ?? [];
  const needle = props.query.trim().toLowerCase();
  if (!needle) {
    return rows;
  }
  return rows.filter((plugin) =>
    [
      plugin.instance_id,
      plugin.manifest_path,
      plugin.plugin_config_path,
      plugin.effective_failure_policy,
      hostGrantText(plugin.host_grants),
      plugin.effective_enabled ? 'enabled' : 'disabled',
      plugin.configured_enabled ? 'configured enabled' : 'configured disabled',
    ]
      .filter(Boolean)
      .some((value) => String(value).toLowerCase().includes(needle)),
  );
});
const sourceLabel = computed(() => plugins.value?.source?.path ?? plugins.value?.source?.mode ?? 'Loading');
const filteredRuntimePlugins = computed(() => {
  const rows = runtimeStatus.value?.plugins ?? [];
  const needle = props.query.trim().toLowerCase();
  if (!needle) {
    return rows;
  }
  return rows.filter((plugin) =>
    [
      plugin.instance_id,
      plugin.plugin_id,
      plugin.state,
      plugin.purpose,
      plugin.runtime,
      queueText(plugin),
      recordsText(plugin),
      warningText(plugin.warnings),
    ]
      .filter(Boolean)
      .some((value) => String(value).toLowerCase().includes(needle)),
  );
});
const runtimeSummary = computed(() => {
  if (!runtimeStatus.value?.available) {
    return 'Unavailable';
  }
  return `${filteredRuntimePlugins.value.length}/${runtimeStatus.value.plugin_count} rows`;
});

onMounted(loadPlugins);

watch(
  () => props.refreshNonce,
  () => {
    loadPlugins();
  },
);

watch(
  loading,
  (value) => {
    emit('loading', value);
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  emit('loading', false);
});

async function loadPlugins() {
  const loadToken = Symbol('plugin-load');
  activeLoad = loadToken;
  loading.value = true;
  error.value = '';
  try {
    const [enablement, runtime] = await Promise.all([
      readPluginEnablement(),
      readPluginRuntimeStatus(),
    ]);
    if (activeLoad === loadToken) {
      plugins.value = enablement;
      runtimeStatus.value = runtime;
    }
  } catch (err) {
    if (activeLoad === loadToken) {
      error.value = String(err.message ?? err);
    }
  } finally {
    if (activeLoad === loadToken) {
      loading.value = false;
    }
  }
}

async function unloadPlugin(instanceId) {
  unloadingInstances.value = { ...unloadingInstances.value, [instanceId]: true };
  error.value = '';
  try {
    await unloadRuntimePlugin(instanceId);
    await loadPlugins();
  } catch (err) {
    error.value = String(err.message ?? err);
  } finally {
    const next = { ...unloadingInstances.value };
    delete next[instanceId];
    unloadingInstances.value = next;
  }
}

function hostGrantText(grants) {
  return grants?.length ? grants.join(', ') : 'none';
}

function queueText(plugin) {
  const depth = plugin.queue_depth ?? 'none';
  const capacity = plugin.queue_capacity ?? 'none';
  return `${depth}/${capacity}`;
}

function recordsText(plugin) {
  return `${plugin.observed_records ?? 0} observed, ${plugin.dropped_records ?? 0} dropped`;
}

function warningText(warnings) {
  return warnings?.length ? warnings.join('; ') : 'none';
}

function payloadReadText(plugin) {
  const metrics = plugin.hostcall_metrics?.payload_read ?? {};
  return `${metrics.calls ?? 0} calls, ${metrics.bytes ?? 0} bytes, ${metrics.truncated ?? 0} truncated`;
}

function purposeLabel(purpose) {
  if (purpose === 'observation-consumer') {
    return 'observer';
  }
  if (purpose === 'control-decider') {
    return 'controller';
  }
  return purpose ?? 'unknown';
}
</script>
