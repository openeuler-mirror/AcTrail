<template>
  <section class="overview-panel">
    <article v-for="section in sections" :key="section.title" class="summary-card">
      <h2>{{ section.title }}</h2>
      <dl>
        <template v-for="item in section.items" :key="item.label">
          <dt>{{ item.label }}</dt>
          <dd>{{ item.value }}</dd>
        </template>
      </dl>
    </article>
  </section>
</template>

<script setup>
import { computed } from 'vue';

import { buildOverviewSections } from './model';

const props = defineProps({
  traceDetail: {
    type: Object,
    default: null,
  },
  actionTree: {
    type: Object,
    required: true,
  },
});

const sections = computed(() => buildOverviewSections(props.traceDetail, props.actionTree));
</script>

<style scoped>
.overview-panel {
  --overview-card-min-width: 220px;
  min-width: 0;
  min-height: 0;
  height: 100%;
  padding: 18px;
  overflow: auto;
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(var(--overview-card-min-width), 1fr));
  gap: 12px;
}

.summary-card {
  min-width: 0;
  padding: 16px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface);
}

.summary-card h2 {
  margin: 0 0 12px;
  color: var(--muted);
  font-size: 12px;
  font-weight: 800;
  text-transform: uppercase;
}

.summary-card dl {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: 8px 12px;
  margin: 0;
  font-size: 13px;
}

.summary-card dt {
  color: var(--muted);
}

.summary-card dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}
</style>
