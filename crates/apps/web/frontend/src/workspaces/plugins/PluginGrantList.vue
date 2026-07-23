<template>
  <ul v-if="items?.length" class="plugin-grant-list">
    <li v-for="item in items" :key="item"><code>{{ item }}</code></li>
  </ul>
  <span v-else class="plugin-grant-empty">none</span>
</template>

<script setup>
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
  gap: var(--stats-space-xs);
  margin: 0;
  padding: 0;
  list-style: none;
}

.plugin-grant-list li {
  min-width: 0;
  max-width: 100%;
  padding: var(--stats-space-xs) var(--stats-space-sm);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-soft);
}

.plugin-grant-list code {
  color: var(--stats-text);
  font-family: "SFMono-Regular", Consolas, "Liberation Mono", monospace;
  font-size: var(--stats-font-xs);
  overflow-wrap: anywhere;
  white-space: normal;
}

.plugin-grant-empty {
  color: var(--stats-muted);
}
</style>
