<template>
  <main class="runtime-workspace">
    <div class="runtime-content">
      <section class="runtime-hero">
        <div>
          <span>Runtime Config</span>
          <h2>Current configuration</h2>
        </div>
        <div class="runtime-source">{{ sourceLabel }}</div>
      </section>

      <section class="runtime-metrics">
        <div v-for="metric in metrics" :key="metric.label" class="runtime-metric">
          <span>{{ metric.label }}</span>
          <strong>{{ metric.value }}</strong>
        </div>
      </section>

      <div v-if="loading && !config" class="runtime-panel loading-panel">
        <span class="loading-spinner" aria-hidden="true"></span>
        <p>Loading configuration...</p>
      </div>

      <section v-else-if="!config?.available" class="runtime-panel runtime-empty">
        <h2>Configuration unavailable</h2>
        <p>{{ config?.reason ?? error }}</p>
      </section>

      <section v-else class="config-layout">
        <aside class="runtime-panel runtime-side">
          <div class="runtime-side-heading">Source</div>
          <dl class="runtime-rows">
            <dt>Mode</dt>
            <dd>{{ config.source?.mode ?? 'unknown' }}</dd>
            <dt>Path</dt>
            <dd>{{ config.source?.path ?? 'n/a' }}</dd>
            <dt>Format</dt>
            <dd>{{ config.format }}</dd>
          </dl>

          <div class="runtime-side-heading">Summary</div>
          <dl class="runtime-rows">
            <template v-for="row in summaryRows" :key="row.label">
              <dt>{{ row.label }}</dt>
              <dd>{{ row.value }}</dd>
            </template>
          </dl>
        </aside>

        <section class="runtime-panel config-document-panel">
          <header class="runtime-panel-header">
            <div>
              <span>Rendered TOML</span>
              <strong>{{ configLineCount }} lines</strong>
            </div>
          </header>
          <div class="config-document">
            <pre>{{ filteredConfigText }}</pre>
          </div>
        </section>
      </section>
    </div>

    <div v-if="error" class="error-bar">{{ error }}</div>
  </main>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import { readCurrentConfig } from '../api';

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

const config = ref(null);
const error = ref('');
const loading = ref(false);
let activeLoad = null;

const metrics = computed(() => {
  const summary = config.value?.summary ?? {};
  return [
    { label: 'Storage', value: basename(summary.storage_path) },
    { label: 'Listen', value: summary.listen_addr ?? 'n/a' },
    { label: 'Plugins', value: `${summary.plugin_enabled_count ?? 0}/${summary.plugin_count ?? 0}` },
    { label: 'Startup Plugins', value: summary.startup_plugins_enabled ? 'Enabled' : 'Disabled' },
  ];
});

const summaryRows = computed(() => {
  const summary = config.value?.summary ?? {};
  return [
    { label: 'Socket', value: summary.socket_path ?? 'n/a' },
    { label: 'Storage', value: summary.storage_path ?? 'n/a' },
    { label: 'Listen', value: summary.listen_addr ?? 'n/a' },
    { label: 'Plugins enabled', value: `${summary.plugin_enabled_count ?? 0}/${summary.plugin_count ?? 0}` },
  ];
});

const filteredConfigText = computed(() => {
  const text = config.value?.text ?? '';
  const needle = props.query.trim().toLowerCase();
  if (!needle) {
    return text;
  }
  return text
    .split('\n')
    .filter((line) => line.toLowerCase().includes(needle))
    .join('\n');
});
const configLineCount = computed(() => (config.value?.text ? config.value.text.split('\n').length : 0));
const sourceLabel = computed(() => config.value?.source?.path ?? config.value?.source?.mode ?? 'Loading');

onMounted(loadConfig);

watch(
  () => props.refreshNonce,
  () => {
    loadConfig();
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

async function loadConfig() {
  const loadToken = Symbol('config-load');
  activeLoad = loadToken;
  loading.value = true;
  error.value = '';
  try {
    const data = await readCurrentConfig();
    if (activeLoad === loadToken) {
      config.value = data;
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

function basename(path) {
  if (!path) {
    return 'n/a';
  }
  return String(path).split('/').filter(Boolean).pop() ?? path;
}
</script>
