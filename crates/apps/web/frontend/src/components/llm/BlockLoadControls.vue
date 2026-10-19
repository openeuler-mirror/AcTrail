<template>
  <div class="block-load-controls">
    <span class="block-load-count">{{ t('count', { loaded, total }) }}</span>
    <template v-if="hasMore">
      <label>{{ t('quantity') }}<input v-model.number="quantity" type="number" min="1" :max="max" step="1" /></label>
      <button type="button" :disabled="loading || !valid" @click="$emit('load', quantity)">{{ t('load', { count: valid ? Math.min(quantity, total - loaded) : '—' }) }}</button>
    </template>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue';
import { useModuleLocale } from '../../locale';
const props = defineProps({ loaded: Number, total: Number, hasMore: Boolean, loading: Boolean,
  defaultSize: { type: Number, default: 5 }, max: { type: Number, default: 250 } });
defineEmits(['load']);
const quantity = ref(props.defaultSize);
const valid = computed(() => Number.isSafeInteger(quantity.value) && quantity.value >= 1 && quantity.value <= props.max);
const { t } = useModuleLocale({
  'en-US': { count: '{loaded} / {total} blocks loaded', quantity: 'Batch size', load: 'Load {count} more' },
  'zh-CN': { count: '已加载 {loaded} / {total} 个 block', quantity: '每次加载', load: '再加载 {count} 个' },
});
</script>

<style scoped>
.block-load-controls { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 12px; font-size: var(--ui-font-sm); }
.block-load-count { flex: 1 1 100%; color: var(--ui-muted); font-variant-numeric: tabular-nums; }
label { display: flex; align-items: center; gap: 8px; color: var(--ui-muted); }
input { width: 64px; }
input, button { padding: 6px 8px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface); color: var(--ui-text); font: inherit; }
button { cursor: pointer; }
button:disabled { opacity: .5; cursor: default; }
</style>
