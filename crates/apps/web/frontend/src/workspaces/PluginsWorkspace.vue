<template>
  <main class="runtime-workspace">
    <div class="runtime-content plugins-runtime-content">
      <section class="runtime-hero">
        <div>
          <span>Installed Plugins</span>
          <h2>Plugin candidates and loaded instances</h2>
        </div>
        <div class="runtime-source">{{ sourceLabel }}</div>
      </section>

      <section class="runtime-metrics">
        <div v-for="metric in metrics" :key="metric.label" class="runtime-metric">
          <span>{{ metric.label }}</span>
          <strong>{{ metric.value }}</strong>
        </div>
      </section>

      <div v-if="loading && !catalog" class="runtime-panel loading-panel">
        <span class="loading-spinner" aria-hidden="true"></span>
        <p>Scanning the plugin directory...</p>
      </div>

      <section v-else-if="!catalog?.available" class="runtime-panel runtime-empty">
        <h2>Plugin discovery unavailable</h2>
        <p>{{ catalog?.reason ?? error }}</p>
      </section>

      <section v-else class="plugins-layout">
        <aside class="runtime-panel runtime-side plugins-runtime-side">
          <section class="runtime-side-section">
            <div class="runtime-side-heading">Discovery</div>
            <dl class="runtime-rows">
              <dt>Directory</dt>
              <dd>{{ catalog.directory }}</dd>
              <dt>Packages</dt>
              <dd>{{ catalog.package_count }}</dd>
              <dt>Runtime</dt>
              <dd>{{ catalog.runtime_available ? 'Available' : 'Unavailable' }}</dd>
            </dl>
          </section>

          <section class="runtime-side-section">
            <div class="runtime-side-heading">Startup</div>
            <dl class="runtime-rows">
              <dt>Global</dt>
              <dd>{{ startup?.global_enabled ? 'Enabled' : 'Disabled' }}</dd>
              <dt>Configured</dt>
              <dd>{{ startup?.configured_count ?? 0 }}</dd>
              <dt>Effective</dt>
              <dd>{{ startup?.enabled_count ?? 0 }}</dd>
            </dl>
          </section>

          <details class="startup-plugin-details">
            <summary>
              <span>Startup load list</span>
              <strong>{{ filteredStartupPlugins.length }}</strong>
            </summary>
            <div v-if="filteredStartupPlugins.length === 0" class="startup-plugin-empty">None</div>
            <ul v-else class="startup-plugin-list">
              <li v-for="plugin in filteredStartupPlugins" :key="plugin.instance_id">
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
                <span>Loaded plugin instances</span>
                <strong>{{ runtimeSummary }}</strong>
              </div>
            </header>
            <div v-if="!catalog.runtime_available" class="runtime-inline-empty">
              <strong>Runtime status unavailable</strong>
              <span>{{ catalog.runtime_error ?? 'The daemon is unavailable.' }}</span>
            </div>
            <div v-else-if="filteredRuntimePlugins.length === 0" class="runtime-compact-empty">
              <strong>No loaded plugin instances</strong>
              <span>Load a candidate below to create a runtime instance.</span>
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
                      <small>Instance ID</small>
                      <strong>{{ plugin.instance_id }}</strong>
                      <span>Plugin <code>{{ plugin.plugin_id }}</code></span>
                    </span>
                    <span class="plugin-runtime-badges">
                      <span class="plugin-runtime-chip primary">{{ purposeLabel(plugin.purpose) }}</span>
                      <span class="plugin-runtime-chip">{{ plugin.runtime }}</span>
                    </span>
                  </summary>
                  <dl class="plugin-runtime-details">
                    <dt>Instance ID</dt>
                    <dd>{{ plugin.instance_id }}</dd>
                    <dt>Plugin ID</dt>
                    <dd>{{ plugin.plugin_id }}</dd>
                    <dt>Records</dt>
                    <dd>{{ recordsText(plugin) }}</dd>
                    <dt>Queue</dt>
                    <dd>{{ queueText(plugin) }}</dd>
                    <dt>Host grants</dt>
                    <dd><PluginGrantList :items="plugin.host_grants" /></dd>
                    <dt>Payload reads</dt>
                    <dd>{{ payloadReadText(plugin) }}</dd>
                    <dt>Last error</dt>
                    <dd>{{ plugin.last_error ?? 'none' }}</dd>
                    <dt>Warnings</dt>
                    <dd>{{ warningText(plugin.warnings) }}</dd>
                  </dl>
                  <PluginConfigPanel
                    :instance-id="plugin.instance_id"
                    :refresh-nonce="configRefreshNonces[plugin.instance_id] ?? 0"
                    @updated="refreshPlugins"
                  />
                  <PluginCommandForm
                    :instance-id="plugin.instance_id"
                    :purpose="plugin.purpose"
                    @completed="refreshPluginConfig(plugin.instance_id)"
                  />
                </details>
                <div class="plugin-lifecycle-control">
                  <span class="plugin-runtime-state active"><i aria-hidden="true"></i>Active</span>
                  <button
                    class="plugin-lifecycle-action danger"
                    type="button"
                    :disabled="loading || unloadingInstances[plugin.instance_id]"
                    :aria-label="`Unload plugin instance ${plugin.instance_id}`"
                    @click="requestUnload(plugin)"
                  >
                    {{ unloadingInstances[plugin.instance_id] ? 'Unloading…' : 'Unload' }}
                  </button>
                </div>
              </article>
            </div>
          </section>

          <section class="runtime-panel plugins-panel">
            <header class="runtime-panel-header">
              <div>
                <span>Plugin candidates</span>
                <strong>{{ packageSummary }}</strong>
              </div>
              <button
                class="runtime-icon-button runtime-refresh-button"
                type="button"
                :disabled="loading"
                title="Rescan plugin directory"
                aria-label="Rescan plugin directory"
                @click="refreshPlugins"
              >
                <RefreshCw :size="16" aria-hidden="true" />
                <span>Refresh</span>
              </button>
            </header>
            <div v-if="filteredPackages.length === 0" class="runtime-compact-empty">
              <strong>No plugin candidates</strong>
              <span>All discovered packages are loaded, or no packages match the current filter.</span>
            </div>
            <div v-else class="plugin-runtime-list">
              <article
                v-for="plugin in filteredPackages"
                :key="plugin.package_key"
                class="plugin-runtime-item"
              >
                <details class="plugin-runtime-disclosure">
                  <summary class="plugin-runtime-summary">
                    <span class="plugin-runtime-main">
                      <small>Plugin ID</small>
                      <strong>{{ plugin.plugin_id ?? 'unavailable' }}</strong>
                      <span>Default instance <code>{{ plugin.plugin_id ?? 'unavailable' }}</code></span>
                    </span>
                    <span class="plugin-runtime-badges">
                      <span v-if="!canLoad(plugin)" class="plugin-runtime-chip">
                        {{ packageState(plugin) }}
                      </span>
                      <span class="plugin-runtime-chip">{{ plugin.package_key }}</span>
                      <span v-if="plugin.purpose" class="plugin-runtime-chip primary">
                        {{ purposeLabel(plugin.purpose) }}
                      </span>
                      <span v-if="plugin.runtime" class="plugin-runtime-chip">{{ plugin.runtime }}</span>
                    </span>
                  </summary>
                  <dl class="plugin-runtime-details">
                    <dt>Plugin ID</dt>
                    <dd>{{ plugin.plugin_id ?? 'unavailable' }}</dd>
                    <dt>Default instance ID</dt>
                    <dd>{{ plugin.plugin_id ?? 'unavailable' }}</dd>
                    <dt>Package path</dt>
                    <dd>{{ plugin.package_path }}</dd>
                    <dt>Manifest</dt>
                    <dd>{{ plugin.manifest_path ?? 'invalid package' }}</dd>
                    <dt>Config</dt>
                    <dd>{{ plugin.plugin_config_path ?? 'none' }}</dd>
                    <dt>Capabilities</dt>
                    <dd><PluginGrantList :items="plugin.requested_capabilities" /></dd>
                    <dt>Loaded instances</dt>
                    <dd>{{ loadedInstanceText(plugin) }}</dd>
                    <dt>Load availability</dt>
                    <dd>{{ loadAvailabilityText(plugin) }}</dd>
                    <dt>Issue</dt>
                    <dd>{{ plugin.issue ?? 'none' }}</dd>
                    <dt>Warnings</dt>
                    <dd>{{ warningText(plugin.warnings) }}</dd>
                  </dl>
                </details>
                <div class="plugin-lifecycle-control">
                  <span class="plugin-runtime-state inactive"><i aria-hidden="true"></i>Unloaded</span>
                  <button
                    class="plugin-lifecycle-action"
                    type="button"
                    :disabled="loading || !canLoad(plugin) || loadingPackages[plugin.package_key]"
                    :title="loadAvailabilityText(plugin)"
                    :aria-label="`Load ${plugin.plugin_id ?? plugin.package_key}`"
                    @click="openLoadDialog(plugin)"
                  >
                    {{ loadingPackages[plugin.package_key] ? 'Loading…' : loadActionLabel(plugin) }}
                  </button>
                </div>
              </article>
            </div>
          </section>
        </div>
      </section>
    </div>

    <PluginLoadDialog
      v-if="selectedLoadPlugin"
      :open="Boolean(selectedLoadPlugin)"
      :plugin="selectedLoadPlugin"
      :busy="Boolean(loadingPackages[selectedLoadPlugin.package_key])"
      @close="selectedLoadPlugin = null"
      @submit="loadPlugin(selectedLoadPlugin, $event)"
    />
    <PluginUnloadDialog
      v-if="selectedUnloadPlugin"
      :open="Boolean(selectedUnloadPlugin)"
      :plugin="selectedUnloadPlugin"
      :busy="Boolean(unloadingInstances[selectedUnloadPlugin.instance_id])"
      @close="selectedUnloadPlugin = null"
      @confirm="unloadPlugin(selectedUnloadPlugin.instance_id)"
    />
    <div v-if="error" class="error-bar">{{ error }}</div>
  </main>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { RefreshCw } from '@lucide/vue';

import {
  loadDiscoveredPlugin,
  readPluginCatalog,
  readPluginEnablement,
  unloadRuntimePlugin,
} from '../api';
import PluginCommandForm from './plugins/PluginCommandForm.vue';
import PluginConfigPanel from './plugins/PluginConfigPanel.vue';
import PluginGrantList from './plugins/PluginGrantList.vue';
import PluginLoadDialog from './plugins/PluginLoadDialog.vue';
import PluginUnloadDialog from './plugins/PluginUnloadDialog.vue';

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

const startup = ref(null);
const catalog = ref(null);
const error = ref('');
const loading = ref(false);
const loadingPackages = ref({});
const unloadingInstances = ref({});
const selectedLoadPlugin = ref(null);
const selectedUnloadPlugin = ref(null);
const configRefreshNonces = ref({});
let activeRefresh = null;

function refreshPluginConfig(instanceId) {
  configRefreshNonces.value = {
    ...configRefreshNonces.value,
    [instanceId]: (configRefreshNonces.value[instanceId] ?? 0) + 1,
  };
}

const metrics = computed(() => [
  { label: 'Installed', value: catalog.value?.package_count ?? 0 },
  { label: 'Candidates', value: candidatePackages.value.length },
  { label: 'Loadable', value: candidatePackages.value.filter(canLoad).length },
  { label: 'Loaded instances', value: catalog.value?.runtime_plugin_count ?? 0 },
]);

const filteredStartupPlugins = computed(() => filterRows(startup.value?.plugins ?? [], [
  'instance_id',
  'manifest_path',
  'plugin_config_path',
]));

const candidatePackages = computed(() => (catalog.value?.packages ?? []).filter(
  (plugin) => !packageLoaded(plugin),
));

const filteredPackages = computed(() => filterRows(candidatePackages.value, [
  'package_key',
  'package_path',
  'plugin_id',
  'purpose',
  'runtime',
  'issue',
]));

const filteredRuntimePlugins = computed(() => filterRows(catalog.value?.runtime_plugins ?? [], [
  'instance_id',
  'plugin_id',
  'state',
  'purpose',
  'runtime',
]));

const sourceLabel = computed(() => catalog.value?.directory ?? 'Scanning');
const packageSummary = computed(() => `${filteredPackages.value.length}/${candidatePackages.value.length} candidates`);
const runtimeSummary = computed(() => {
  if (!catalog.value?.runtime_available) {
    return 'Unavailable';
  }
  return `${filteredRuntimePlugins.value.length}/${catalog.value.runtime_plugin_count} rows`;
});

onMounted(refreshPlugins);

watch(
  () => props.refreshNonce,
  refreshPlugins,
);

watch(
  loading,
  (value) => emit('loading', value),
  { immediate: true },
);

onBeforeUnmount(() => emit('loading', false));

async function refreshPlugins() {
  const refreshToken = Symbol('plugin-refresh');
  activeRefresh = refreshToken;
  loading.value = true;
  error.value = '';
  try {
    const [startupStatus, catalogStatus] = await Promise.all([
      readPluginEnablement(),
      readPluginCatalog(),
    ]);
    if (activeRefresh === refreshToken) {
      startup.value = startupStatus;
      catalog.value = catalogStatus;
    }
  } catch (err) {
    if (activeRefresh === refreshToken) {
      error.value = String(err.message ?? err);
    }
  } finally {
    if (activeRefresh === refreshToken) {
      loading.value = false;
    }
  }
}

function openLoadDialog(plugin) {
  selectedLoadPlugin.value = plugin;
}

function requestUnload(plugin) {
  selectedUnloadPlugin.value = plugin;
}

async function loadPlugin(plugin, options) {
  const packageKey = plugin.package_key;
  loadingPackages.value = { ...loadingPackages.value, [packageKey]: true };
  error.value = '';
  try {
    await loadDiscoveredPlugin(packageKey, options);
    selectedLoadPlugin.value = null;
    await refreshPlugins();
  } catch (err) {
    error.value = String(err.message ?? err);
  } finally {
    const next = { ...loadingPackages.value };
    delete next[packageKey];
    loadingPackages.value = next;
  }
}

async function unloadPlugin(instanceId) {
  unloadingInstances.value = { ...unloadingInstances.value, [instanceId]: true };
  error.value = '';
  try {
    await unloadRuntimePlugin(instanceId);
    selectedUnloadPlugin.value = null;
    await refreshPlugins();
  } catch (err) {
    error.value = String(err.message ?? err);
  } finally {
    const next = { ...unloadingInstances.value };
    delete next[instanceId];
    unloadingInstances.value = next;
  }
}

function filterRows(rows, fields) {
  const needle = props.query.trim().toLowerCase();
  if (!needle) {
    return rows;
  }
  return rows.filter((row) => fields
    .map((field) => row[field])
    .concat(row.requested_capabilities ?? [], row.loaded_instances ?? [], row.warnings ?? [])
    .filter(Boolean)
    .some((value) => String(value).toLowerCase().includes(needle)));
}

function canLoad(plugin) {
  return Boolean(
    catalog.value?.runtime_available
      && plugin.activation_ready
      && !packageLoaded(plugin),
  );
}

function packageLoaded(plugin) {
  return (plugin.loaded_instances?.length ?? 0) > 0;
}

function packageState(plugin) {
  if (plugin.issue) {
    return 'requires attention';
  }
  if (!catalog.value?.runtime_available) {
    return 'runtime unavailable';
  }
  if (plugin.parameterized_host_grants?.length) {
    return 'grant configuration required';
  }
  return 'ready to load';
}

function loadAvailabilityText(plugin) {
  if (!plugin.activation_ready) {
    return plugin.issue ?? 'Plugin package is not loadable';
  }
  if (!catalog.value?.runtime_available) {
    return catalog.value?.runtime_error ?? 'Daemon plugin runtime is unavailable';
  }
  if (plugin.parameterized_host_grants?.length) {
    return 'Configure instance identity and scoped permissions before loading';
  }
  return 'Ready to load through actrailweb';
}

function loadActionLabel(plugin) {
  return plugin.parameterized_host_grants?.length ? 'Configure & load' : 'Load plugin';
}

function loadedInstanceText(plugin) {
  if (plugin.loaded_instances == null) {
    return 'runtime unavailable';
  }
  return plugin.loaded_instances.length ? plugin.loaded_instances.join(', ') : 'none';
}

function queueText(plugin) {
  return `${plugin.queue_depth ?? 'none'}/${plugin.queue_capacity ?? 'none'}`;
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
