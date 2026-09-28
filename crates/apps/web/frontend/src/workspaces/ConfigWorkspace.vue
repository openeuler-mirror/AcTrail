<template>
  <main class="runtime-workspace">
    <div class="runtime-content">
      <section class="runtime-hero">
        <div>
          <span>{{ t('config.kicker') }}</span>
          <h2>{{ t('app.titles.config') }}</h2>
        </div>
        <div class="runtime-source">{{ sourceLabel }}</div>
      </section>

      <section v-reveal class="runtime-metrics">
        <div v-for="metric in metrics" :key="metric.label" class="runtime-metric">
          <span>{{ metric.label }}</span>
          <strong>{{ metric.value }}</strong>
        </div>
      </section>

      <div v-if="loading && !config" v-reveal class="runtime-panel config-skeleton">
        <SkeletonBlock width="38%" height="14px" />
        <SkeletonBlock width="72%" height="10px" />
        <SkeletonBlock width="100%" height="180px" radius="var(--ui-radius-md)" />
      </div>

      <section v-else-if="!config?.available" v-reveal class="runtime-panel">
        <EmptyState :title="t('config.unavailable')" :description="config?.reason ?? error" />
      </section>

      <section v-else class="config-layout">
        <aside v-reveal class="runtime-panel runtime-side">
          <div class="runtime-side-heading">{{ t('config.source') }}</div>
          <dl class="runtime-rows">
            <dt>{{ t('config.mode') }}</dt>
            <dd>{{ config.source?.mode ?? t('config.unknown') }}</dd>
            <dt>{{ t('config.path') }}</dt>
            <dd>{{ config.source?.path ?? t('config.notAvailable') }}</dd>
            <dt>{{ t('config.format') }}</dt>
            <dd>{{ config.format }}</dd>
          </dl>

          <div class="runtime-side-heading">{{ t('config.summary') }}</div>
          <dl class="runtime-rows">
            <template v-for="row in summaryRows" :key="row.label">
              <dt>{{ row.label }}</dt>
              <dd>{{ row.value }}</dd>
            </template>
          </dl>
        </aside>

        <section v-reveal="1" class="runtime-panel config-document-panel">
          <header class="runtime-panel-header">
            <div>
              <span>{{ t('config.toml') }}</span>
              <strong>{{ t('config.lineCount', { count: configLineCount }) }}</strong>
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
import EmptyState from '../components/EmptyState.vue';
import SkeletonBlock from '../components/SkeletonBlock.vue';
import { useModuleLocale } from '../locale';
import strings from './locale';

const { t } = useModuleLocale(strings);

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
    { label: t('config.storage'), value: basename(summary.storage_path) },
    { label: t('config.listen'), value: summary.listen_addr ?? t('config.notAvailable') },
    { label: t('config.plugins'), value: `${summary.plugin_enabled_count ?? 0}/${summary.plugin_count ?? 0}` },
    { label: t('config.startupPlugins'), value: summary.startup_plugins_enabled ? t('config.enabled') : t('config.disabled') },
  ];
});

const summaryRows = computed(() => {
  const summary = config.value?.summary ?? {};
  return [
    { label: t('config.socket'), value: summary.socket_path ?? t('config.notAvailable') },
    { label: t('config.storage'), value: summary.storage_path ?? t('config.notAvailable') },
    { label: t('config.listen'), value: summary.listen_addr ?? t('config.notAvailable') },
    { label: t('config.pluginsEnabled'), value: `${summary.plugin_enabled_count ?? 0}/${summary.plugin_count ?? 0}` },
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
