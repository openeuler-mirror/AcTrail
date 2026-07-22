<template>
  <section class="trace-alerts">
    <article v-for="alert in filteredAlerts" :key="alert.alert_id" class="trace-alert-card">
      <header>
        <span class="trace-alert-severity" :class="`severity-${alert.severity}`">
          {{ alert.severity }}
        </span>
        <h3>{{ alert.title }}</h3>
        <time>{{ formatTime(alert.created_at) }}</time>
      </header>
      <p>{{ alert.kind }} · alert-{{ alert.alert_id }}</p>
      <ul v-if="residualFiles(alert).length" class="trace-alert-paths">
        <li v-for="path in residualFiles(alert)" :key="path"><code>{{ path }}</code></li>
      </ul>
      <JsonTree v-else :value="alert.payload" />
    </article>
    <p v-if="!filteredAlerts.length" class="trace-alert-empty">{{ t('alerts.traceEmpty') }}</p>
  </section>
</template>

<script setup>
import { computed } from 'vue';

import JsonTree from '../../../components/JsonTree.vue';
import { useLocale } from '../../../locale';

const props = defineProps({
  traceDetail: {
    type: Object,
    default: null,
  },
  query: {
    type: String,
    default: '',
  },
});

const { t } = useLocale();
const filteredAlerts = computed(() => {
  const alerts = props.traceDetail?.alerts ?? [];
  const needle = props.query.trim().toLowerCase();
  if (!needle) {
    return alerts;
  }
  return alerts.filter((alert) => JSON.stringify(alert).toLowerCase().includes(needle));
});

function residualFiles(alert) {
  return Array.isArray(alert?.payload?.residual_files) ? alert.payload.residual_files : [];
}

function formatTime(timestamp) {
  const value = Number(timestamp);
  return Number.isFinite(value) ? new Date(value).toLocaleString() : String(timestamp ?? '');
}
</script>

<style scoped>
.trace-alerts {
  display: grid;
  gap: var(--stats-space-md, 12px);
  padding: var(--stats-space-lg, 16px);
  overflow-y: auto;
}

.trace-alert-card {
  padding: 16px;
  border: 1px solid var(--stats-border, var(--border));
  border-radius: var(--stats-radius-md, 10px);
  background: var(--stats-surface-panel, var(--surface));
}

.trace-alert-card header {
  display: flex;
  align-items: center;
  gap: 10px;
}

.trace-alert-card h3 {
  flex: 1;
  margin: 0;
}

.trace-alert-card time,
.trace-alert-card p,
.trace-alert-empty {
  color: var(--stats-muted, var(--muted));
}

.trace-alert-severity {
  padding: 2px 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--stats-muted, #777) 18%, transparent);
  font-size: 0.75rem;
  font-weight: 700;
  text-transform: uppercase;
}

.severity-medium,
.severity-high,
.severity-critical {
  color: var(--danger, #d04b4b);
}

.trace-alert-paths {
  display: grid;
  gap: 8px;
  padding-left: 20px;
}

.trace-alert-paths code {
  overflow-wrap: anywhere;
}
</style>
