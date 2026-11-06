<template>
  <main class="runtime-workspace config-workspace">
    <div class="runtime-content config-content">
      <header v-reveal class="config-page-header">
        <div>
          <span class="config-kicker">{{ t('config.kicker') }}</span>
          <h2>{{ t('app.titles.config') }}</h2>
          <p>{{ t('config.pageHint') }}</p>
        </div>
        <div class="config-startup-note">
          <Clock3 :size="14" aria-hidden="true" />
          <span>{{ t('config.startupLoading') }}</span>
        </div>
      </header>

      <ConfigEditor :refresh-nonce="refreshNonce" />

      <div v-if="loading && !config" v-reveal class="config-skeleton">
        <SkeletonBlock width="38%" height="14px" />
        <SkeletonBlock width="72%" height="10px" />
        <SkeletonBlock width="100%" height="180px" radius="var(--ui-radius-md)" />
      </div>

      <section v-else-if="!config?.available" v-reveal>
        <EmptyState :title="t('config.unavailable')" :description="config?.reason ?? error" />
      </section>

      <section v-if="config?.available" v-reveal class="config-document-panel">
        <header class="config-snapshot-header">
          <div>
            <div class="config-snapshot-title">
              <h3>{{ t('config.snapshotTitle') }}</h3>
              <span><LockKeyhole :size="12" aria-hidden="true" />{{ t('config.readonly') }}</span>
            </div>
            <p>{{ t('config.snapshotHint') }}</p>
          </div>
          <span class="config-line-count">{{ t('config.lineCount', { count: configLineCount }) }}</span>
        </header>
        <div class="config-snapshot-body">
          <aside class="config-runtime-summary">
            <h4>{{ t('config.source') }}</h4>
            <dl class="runtime-rows">
              <dt>{{ t('config.mode') }}</dt>
              <dd>{{ config.source?.mode ?? t('config.unknown') }}</dd>
              <dt>{{ t('config.path') }}</dt>
              <dd>{{ config.source?.path ?? t('config.notAvailable') }}</dd>
              <dt>{{ t('config.format') }}</dt>
              <dd>{{ config.format }}</dd>
            </dl>
            <h4>{{ t('config.summary') }}</h4>
            <dl class="runtime-rows">
              <template v-for="row in summaryRows" :key="row.label">
                <dt>{{ row.label }}</dt>
                <dd>{{ row.value }}</dd>
              </template>
            </dl>
          </aside>
          <div class="config-document" tabindex="0" role="region" :aria-label="t('config.toml')">
            <pre>{{ filteredConfigText || t('config.noMatches') }}</pre>
          </div>
        </div>
      </section>
    </div>

    <div v-if="error" class="error-bar">{{ error }}</div>
  </main>
</template>

<script setup>
import { Clock3, LockKeyhole } from '@lucide/vue';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import { readCurrentConfig } from '../api';
import EmptyState from '../components/EmptyState.vue';
import SkeletonBlock from '../components/SkeletonBlock.vue';
import { useModuleLocale } from '../locale';
import strings from './locale';
import ConfigEditor from './ConfigEditor.vue';

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

const summaryRows = computed(() => {
  const summary = config.value?.summary ?? {};
  return [
    { label: t('config.socket'), value: summary.socket_path ?? t('config.notAvailable') },
    { label: t('config.storage'), value: summary.storage_path ?? t('config.notAvailable') },
    { label: t('config.listen'), value: summary.listen_addr ?? t('config.notAvailable') },
    { label: t('config.pluginsEnabled'), value: `${summary.plugin_enabled_count ?? 0}/${summary.plugin_count ?? 0}` },
    { label: t('config.startupPlugins'), value: summary.startup_plugins_enabled ? t('config.enabled') : t('config.disabled') },
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

</script>

<style scoped>
.config-content { max-width: 1120px; gap: 36px; padding-top: 88px; padding-bottom: 48px; }
.config-page-header { display: flex; align-items: center; justify-content: space-between; gap: 24px; padding-bottom: 28px; border-bottom: 1px solid var(--ui-border); }
.config-kicker { color: var(--ui-muted); font-size: 11px; font-weight: 500; letter-spacing: .06em; text-transform: uppercase; }
.config-page-header h2 { margin: 8px 0 10px; font-size: 30px; font-weight: 600; line-height: 1.2; letter-spacing: -.035em; }
.config-page-header p { margin: 0; color: var(--ui-muted); font-size: 13px; line-height: 1.6; }
.config-startup-note { display: flex; align-items: center; flex-shrink: 0; gap: 7px; color: var(--ui-muted); font-size: 11px; }
.config-skeleton { display: grid; gap: 16px; }
.config-document-panel { display: block; min-height: 0; border-top: 1px solid var(--ui-border); }
.config-snapshot-header { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; padding: 28px 0 24px; }
.config-snapshot-title { display: flex; align-items: center; flex-wrap: wrap; gap: 12px; }
.config-snapshot-title h3 { margin: 0; font-size: 15px; font-weight: 600; }
.config-snapshot-title > span { display: inline-flex; align-items: center; gap: 5px; color: var(--ui-muted); font-size: 11px; }
.config-snapshot-header p { margin: 8px 0 0; color: var(--ui-muted); font-size: 12px; line-height: 1.7; }
.config-line-count { flex-shrink: 0; padding-top: 3px; color: var(--ui-muted); font: 11px var(--ui-mono); }
.config-snapshot-body { display: grid; grid-template-columns: 240px minmax(0, 1fr); gap: 32px; }
.config-runtime-summary { min-width: 0; }
.config-runtime-summary h4 { margin: 0 0 14px; font-size: 12px; font-weight: 500; }
.config-runtime-summary h4:not(:first-child) { margin-top: 24px; }
.runtime-rows { grid-template-columns: minmax(0, 1fr); gap: 4px; font-size: 11px; line-height: 1.6; }
.runtime-rows dd { margin-bottom: 8px; font-family: var(--ui-mono); }
.config-document { max-height: 480px; border: 1px solid var(--ui-border); border-radius: 8px; background: var(--ui-bg-quinary); scrollbar-gutter: stable; }
.config-document:focus-visible { outline: 2px solid var(--ui-focus-ring-color); outline-offset: 3px; }
.config-document pre { min-width: 0; padding: 20px; color: var(--ui-muted); font: 11px/1.9 var(--ui-mono); }
@media (max-width: 1100px) {
  .config-snapshot-body { grid-template-columns: 190px minmax(0, 1fr); gap: 24px; }
}
@media (max-width: 760px) {
  .config-content { gap: 28px; padding-top: calc(var(--ui-header-offset) + 24px); }
  .config-page-header { align-items: flex-start; flex-direction: column; gap: 16px; padding-bottom: 24px; }
  .config-page-header h2 { font-size: 26px; }
  .config-snapshot-header { gap: 12px; }
  .config-snapshot-body { grid-template-columns: minmax(0, 1fr); }
  .config-runtime-summary { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 14px 20px; }
  .config-runtime-summary h4 { margin: 0; }
  .config-runtime-summary h4:not(:first-child) { grid-column: 2; grid-row: 1; margin-top: 0; }
  .config-document { max-height: 360px; }
  .config-document pre { padding: 14px; }
}
</style>
