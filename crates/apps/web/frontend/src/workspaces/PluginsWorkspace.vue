<template>
  <main class="runtime-workspace">
    <div class="runtime-content plugins-runtime-content">
      <section class="runtime-hero">
        <div>
          <span>{{ t('workspace.kicker') }}</span>
          <h2>{{ t('workspace.title') }}</h2>
        </div>
        <div class="runtime-source">{{ sourceLabel }}</div>
      </section>

      <section class="runtime-metrics">
        <div v-for="metric in metrics" :key="metric.label" class="runtime-metric">
          <span>{{ metric.label }}</span>
          <strong>{{ metric.value }}</strong>
        </div>
      </section>

      <div v-if="loading && !catalog" v-reveal class="runtime-panel plugin-skeleton">
        <SkeletonBlock width="42%" height="14px" />
        <SkeletonBlock width="100%" height="64px" radius="var(--ui-radius-md)" />
        <SkeletonBlock width="100%" height="64px" radius="var(--ui-radius-md)" />
        <SkeletonBlock width="78%" height="64px" radius="var(--ui-radius-md)" />
      </div>

        <section v-else-if="!catalog?.available" v-reveal class="runtime-panel">
        <EmptyState :title="t('workspace.discoveryUnavailable')" :description="catalog?.reason ?? error" />
      </section>

      <template v-else>
        <section class="plugin-status-strip">
          <span class="plugin-status-item">
            <i class="plugin-status-dot" :class="startup?.global_enabled ? 'is-on' : 'is-off'" aria-hidden="true"></i>
            {{ t('workspace.globalStatus') }}
            <b>{{ startup?.global_enabled ? t('workspace.enabled') : t('workspace.disabled') }}</b>
          </span>
          <span class="plugin-status-item">{{ t('workspace.directory') }} <b>{{ directoryLabel }}</b></span>
          <span class="plugin-status-item">{{ t('workspace.discovered') }} <b>{{ catalog.package_count ?? 0 }}</b></span>
          <span class="plugin-status-item">
            {{ t('workspace.effective') }} <b>{{ startup?.enabled_count ?? 0 }}/{{ startup?.configured_count ?? 0 }}</b>
          </span>
        </section>

        <section class="plugin-toolbar">
          <label class="plugin-search">
            <Search :size="16" aria-hidden="true" />
            <input v-model="pluginsQuery" type="search" :placeholder="t('workspace.searchPlaceholder')" />
          </label>
          <button class="plugin-toolbar-action" type="button" :disabled="loading" @click="refreshPlugins">
            <RefreshCw :size="15" aria-hidden="true" />
            <span>{{ t('workspace.refresh') }}</span>
          </button>
        </section>

        <section class="runtime-panel plugins-panel">
          <header class="runtime-panel-header">
            <div>
              <span>{{ t('workspace.title') }}</span>
              <strong>{{ t('workspace.packagesSummary', { count: pluginRows.length, runtime: runtimeSummary }) }}</strong>
            </div>
          </header>

          <EmptyState
            v-if="!pluginRows.length"
            :title="t('workspace.noMatch')"
            :description="t('workspace.noMatchHint')"
          />

          <ul v-else class="plugin-manage-list">
            <li
              v-for="row in pluginRows"
              :key="row.key"
              class="plugin-manage-row"
              :class="{ 'is-open': detailRow?.key === row.key }"
            >
              <button class="plugin-manage-open" type="button" @click="openDetail(row)">
                <span class="plugin-manage-tile" aria-hidden="true"><Puzzle :size="16" /></span>
                <span class="plugin-manage-identity">
                  <strong>{{ row.pluginId }}</strong>
                  <small>
                    {{ row.packageKey }}<template v-if="row.purposeLabel"> · {{ row.purposeLabel }}</template>
                  </small>
                </span>
                <span class="plugin-manage-chips">
                  <span
                    v-for="chip in row.chips"
                    :key="chip.label"
                    class="plugin-runtime-chip"
                    :class="chip.tone"
                  >{{ chip.label }}</span>
                </span>
              </button>
              <span class="plugin-manage-actions">
                <ToggleSwitch
                  :checked="row.loaded"
                  :disabled="loading || (!row.loaded && !row.canLoad)"
                  :aria-label="`${row.loaded ? t('workspace.unload') : t('workspace.load')} ${row.pluginId}`"
                  @change="toggleRow(row, $event)"
                />
                <button class="plugin-manage-action" type="button" @click="openDetail(row)">
                  {{ t('workspace.configure') }}
                </button>
              </span>
            </li>
          </ul>
        </section>

        <section class="runtime-panel plugins-panel plugins-panel-secondary">
          <header class="runtime-panel-header">
            <div>
              <span>{{ t('workspace.startup') }}</span>
              <strong>{{ t('workspace.startupConfigured', { count: filteredStartupPlugins.length }) }}</strong>
            </div>
          </header>
          <div v-if="!filteredStartupPlugins.length" class="runtime-compact-empty">
            <strong>{{ t('workspace.startupEmptyTitle') }}</strong>
            <span>{{ t('workspace.startupEmptyHint') }}</span>
          </div>
          <ul v-else class="startup-plugin-list">
            <li v-for="plugin in filteredStartupPlugins" :key="plugin.instance_id">
              <div>
                <strong>{{ plugin.instance_id }}</strong>
                <span>{{ plugin.effective_enabled ? t('workspace.enabled') : t('workspace.disabled') }}</span>
              </div>
              <code>{{ plugin.manifest_path }}</code>
            </li>
          </ul>
        </section>
      </template>
    </div>

    <PluginDetailPanel
      v-if="detailRow"
      :row="detailRow"
      :instances="detailRow.instances"
      :config-nonces="configRefreshNonces"
      @close="detailRow = null"
      @unload="requestUnload"
      @config-changed="refreshPluginConfig"
      @config-updated="refreshPlugins"
    />

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
import { Puzzle, RefreshCw, Search } from '@lucide/vue';

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
import EmptyState from '../components/EmptyState.vue';
import SkeletonBlock from '../components/SkeletonBlock.vue';
import ToggleSwitch from '../components/ToggleSwitch.vue';
import PluginDetailPanel from './plugins/PluginDetailPanel.vue';
import { useModuleLocale } from '../locale';
import pluginStrings from './plugins/locale';

const { t } = useModuleLocale(pluginStrings);

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
  { label: t('workspace.metricInstalled'), value: catalog.value?.package_count ?? 0 },
  { label: t('workspace.metricCandidates'), value: candidatePackages.value.length },
  { label: t('workspace.metricLoadable'), value: candidatePackages.value.filter(canLoad).length },
  { label: t('workspace.metricLoadedInstances'), value: catalog.value?.runtime_plugin_count ?? 0 },
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

const sourceLabel = computed(() => catalog.value?.directory ?? t('workspace.scanning'));
const directoryLabel = computed(() => catalog.value?.directory ?? t('workspace.scanning'));
const pluginsQuery = ref('');
const detailRow = ref(null);

/**
 * One row per discovered package, with its runtime instances joined in. The
 * previous two-section split made a plugin jump between panels as it was
 * loaded; a single row keeps the lifecycle in one place.
 */
const pluginRows = computed(() => {
  const instancesByPlugin = new Map();
  for (const instance of catalog.value?.runtime_plugins ?? []) {
    const list = instancesByPlugin.get(instance.plugin_id) ?? [];
    list.push(instance);
    instancesByPlugin.set(instance.plugin_id, list);
  }
  const query = pluginsQuery.value.trim().toLowerCase();
  return (catalog.value?.packages ?? [])
    .map((pkg) => {
      const instances = instancesByPlugin.get(pkg.plugin_id) ?? [];
      const chips = [];
      if (!catalog.value?.runtime_available) {
        chips.push({ label: t('workspace.chipRuntimeUnavailable'), tone: 'warn' });
      }
      if (instances.length) {
        chips.push({ label: t('workspace.chipLoaded', { count: instances.length }), tone: 'primary' });
      }
      if (pkg.runtime) {
        chips.push({ label: pkg.runtime, tone: '' });
      }
      if (pkg.purpose) {
        chips.push({ label: purposeLabel(pkg.purpose), tone: '' });
      }
      if (!pkg.activation_ready) {
        chips.push({ label: pkg.issue ?? t('workspace.chipNotLoadable'), tone: 'warn' });
      } else if (!instances.length) {
        chips.push({ label: t('workspace.chipReady'), tone: '' });
      }
      if (pkg.warnings?.length) {
        chips.push({ label: t('workspace.chipWarnings', { count: pkg.warnings.length }), tone: 'warn' });
      }
      return {
        key: pkg.package_key,
        packageKey: pkg.package_key,
        pluginId: pkg.plugin_id ?? pkg.package_key,
        purposeLabel: pkg.purpose ? purposeLabel(pkg.purpose) : '',
        package: pkg,
        instances,
        loaded: instances.length > 0,
        canLoad: canLoad(pkg),
        chips,
        search: [pkg.package_key, pkg.plugin_id, pkg.purpose, pkg.runtime, ...(pkg.requested_capabilities ?? [])]
          .filter(Boolean)
          .join(' ')
          .toLowerCase(),
      };
    })
    .filter((row) => !query || row.search.includes(query));
});
const packageSummary = computed(() => `${filteredPackages.value.length}/${candidatePackages.value.length} candidates`);
const runtimeSummary = computed(() => {
  if (!catalog.value?.runtime_available) {
    return t('workspace.runtimeUnavailableRow');
  }
  return t('workspace.runtimeRows', {
    filtered: filteredRuntimePlugins.value.length,
    total: catalog.value.runtime_plugin_count,
  });
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

function openDetail(row) {
  detailRow.value = row;
}

/**
 * The switch mirrors the loaded state: turning it on runs the existing load
 * flow (which collects grants), turning it off asks for unload confirmation.
 */
function toggleRow(row, next) {
  if (next) {
    openLoadDialog(row.package);
    return;
  }
  const instance = row.instances[0];
  if (instance) {
    requestUnload(instance);
  }
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
    return t('workspace.stateRequiresAttention');
  }
  if (!catalog.value?.runtime_available) {
    return t('workspace.chipRuntimeUnavailable');
  }
  if (plugin.parameterized_host_grants?.length) {
    return t('workspace.stateGrantRequired');
  }
  return t('workspace.stateReadyToLoad');
}

function loadAvailabilityText(plugin) {
  if (!plugin.activation_ready) {
    return plugin.issue ?? t('workspace.unavailablePackage');
  }
  if (!catalog.value?.runtime_available) {
    return catalog.value?.runtime_error ?? t('workspace.unavailableRuntime');
  }
  if (plugin.parameterized_host_grants?.length) {
    return t('workspace.grantRequiredHint');
  }
  return t('workspace.readyToLoadHint');
}

function loadActionLabel(plugin) {
  return plugin.parameterized_host_grants?.length ? t('workspace.configureAndLoad') : t('workspace.loadPlugin');
}

function loadedInstanceText(plugin) {
  if (plugin.loaded_instances == null) {
    return t('workspace.chipRuntimeUnavailable');
  }
  return plugin.loaded_instances.length ? plugin.loaded_instances.join(', ') : t('workspace.none');
}

function queueText(plugin) {
  return `${plugin.queue_depth ?? t('workspace.none')}/${plugin.queue_capacity ?? t('workspace.none')}`;
}

function recordsText(plugin) {
  return t('workspace.recordsSummary', {
    observed: plugin.observed_records ?? 0,
    dropped: plugin.dropped_records ?? 0,
  });
}

function warningText(warnings) {
  return warnings?.length ? warnings.join('; ') : t('workspace.none');
}

function payloadReadText(plugin) {
  const metrics = plugin.hostcall_metrics?.payload_read ?? {};
  return t('workspace.payloadReadSummary', {
    calls: metrics.calls ?? 0,
    bytes: metrics.bytes ?? 0,
    truncated: metrics.truncated ?? 0,
  });
}

function runtimeStateLabel(state) {
  if (state === 'active') return t('workspace.stateActive');
  if (state === 'draining') return t('workspace.stateDraining');
  if (state === 'stopped') return t('workspace.stateStopped');
  if (state === 'failed') return t('workspace.stateFailed');
  return state ?? t('workspace.stateUnknown');
}

function canUnload(plugin) {
  return plugin.purpose !== 'alert-consumer';
}

function purposeLabel(purpose) {
  if (purpose === 'observation-consumer') {
    return t('workspace.purposeObserver');
  }
  if (purpose === 'control-decider') {
    return t('workspace.purposeController');
  }
  if (purpose === 'alert-consumer') {
    return t('workspace.purposeAlertForwarding');
  }
  return purpose ?? t('workspace.unknown');
}
</script>
