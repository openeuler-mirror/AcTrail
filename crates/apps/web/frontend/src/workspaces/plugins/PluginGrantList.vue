<template>
  <ul v-if="items?.length" class="plugin-grant-list">
    <li v-for="item in items" :key="item"><code>{{ item }}</code></li>
  </ul>
  <span v-else class="plugin-grant-empty">{{ t('grantList.none') }}</span>
</template>

<script setup>
import { useModuleLocale } from '../../locale';
import strings from './locale';

const { t } = useModuleLocale(strings);

defineProps({
  items: {
    type: Array,
    default: () => [],
  },
});
</script>

<style scoped>
.plugin-grant-list {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  gap: var(--ui-space-xs);
  margin: 0;
  padding: 0;
  list-style: none;
}

.plugin-grant-list li {
  min-width: 0;
  max-width: 100%;
  padding: var(--ui-space-xs) var(--ui-space-sm);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-soft);
}

.plugin-grant-list code {
  color: var(--ui-text);
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: var(--ui-font-xs);
  overflow-wrap: anywhere;
  white-space: normal;
}

.plugin-grant-empty {
  color: var(--ui-muted);
}
</style>
