<!-- Filter traces and navigate numbered pages without exposing query internals. -->
<template>
  <section class="trace-list-controls" :aria-busy="loading">
    <button class="trace-filter-toggle" type="button" :aria-expanded="filtersOpen" @click="filtersOpen = !filtersOpen">
      <Filter :size="14" aria-hidden="true" /><span>{{ t('filters') }}</span>
      <ChevronUp v-if="filtersOpen" :size="14" aria-hidden="true" /><ChevronDown v-else :size="14" aria-hidden="true" />
    </button>
    <form v-if="filtersOpen" class="trace-filter-form" @submit.prevent="pager.load({ filters: draft.query() })">
      <label>{{ t('name') }}<input v-model="draft.name" /></label>
      <label>{{ t('id') }}<input v-model="draft.trace_id" /></label>
      <label>{{ t('from') }}<input type="datetime-local" step="0.001" :value="draft.localTime('from')" @input="draft.setLocalTime('from', $event.target.value)" /></label>
      <label>{{ t('to') }}<input type="datetime-local" step="0.001" :value="draft.localTime('to')" @input="draft.setLocalTime('to', $event.target.value)" /></label>
      <button type="submit" :disabled="loading">{{ t('apply') }}</button>
    </form>
    <div class="trace-page-size">
      <label><select :value="filters.limit" :disabled="loading" :aria-label="t('limit')" @change="changeLimit($event.target.value)">
        <option v-for="size in pageSizes" :key="size" :value="size">{{ t('perPage', { size }) }}</option>
      </select></label>
      <span v-if="state.page">{{ t('total', { count: state.total }) }}</span>
    </div>
    <form v-if="state.totalPages > 0" class="trace-list-paging" @submit.prevent="pager.go(pageDraft)">
      <button type="button" :disabled="loading || state.page <= 1" :aria-label="t('previous')" @click="pager.go(state.page - 1)"><ChevronLeft :size="15" /></button>
      <input v-model.number="pageDraft" class="trace-page-input" type="number" min="1" :max="state.totalPages" :disabled="loading" :aria-label="t('page')" required />
      <span>/ {{ state.totalPages }}</span>
      <button class="trace-page-go" type="submit" :disabled="loading">GO</button>
      <button type="button" :disabled="loading || state.page >= state.totalPages" :aria-label="t('next')" @click="pager.next()"><ChevronRight :size="15" /></button>
    </form>
    <small v-if="loading" role="status">{{ t('loading') }}</small>
    <p v-if="error" role="alert">{{ error }} <button type="button" :disabled="loading" @click="pager.retry?.()">{{ t('retry') }}</button></p>
  </section>
</template>

<script setup>
import { computed, reactive, ref, watch } from 'vue';
import { ChevronDown, ChevronLeft, ChevronRight, ChevronUp, Filter } from '@lucide/vue';
import { useModuleLocale } from '../../locale';
import { TraceFilters } from './filters.js';

const props = defineProps({ pager: { type: Object, required: true } });
const { filters, loading, error, state } = props.pager;
const draft = reactive(new TraceFilters(filters.value));
const filtersOpen = ref(false);
const pageDraft = ref(1);
const pageSizes = computed(() => [...new Set([10, 25, 50, 100, 250, filters.value.limit])].sort((a, b) => a - b));
watch(filters, value => draft.reset(value));
watch(state, value => { pageDraft.value = value.page || 1; });
function changeLimit(value) {
  props.pager.load({ filters: { ...filters.value, limit: Number(value) } });
}
const { t } = useModuleLocale({
  'en-US': { filters: 'Filters', name: 'Trace name', id: 'Trace ID', from: 'Created after', to: 'Created before', limit: 'Items per page', perPage: '{size} / page', apply: 'Apply', page: 'Page', total: '{count} traces', previous: 'Previous page', next: 'Next page', loading: 'Loading…', retry: 'Retry' },
  'zh-CN': { filters: '筛选', name: 'Trace 名称', id: 'Trace ID', from: '创建起始时间', to: '创建结束时间', limit: '每页条数', perPage: '{size} 条 / 页', apply: '应用', page: '页码', total: '共 {count} 条', previous: '上一页', next: '下一页', loading: '加载中…', retry: '重试' },
});
</script>

<style scoped>
.trace-list-controls { display: grid; gap: 10px; padding: 8px 0 12px; color: var(--ui-text); }
.trace-list-controls button, .trace-list-controls input, .trace-list-controls select { box-sizing: border-box; min-width: 0; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); padding: 6px 8px; background: var(--ui-surface); color: var(--ui-text); font: inherit; font-size: var(--ui-font-sm); }
.trace-list-controls button { cursor: pointer; display: inline-flex; align-items: center; justify-content: center; gap: 6px; }
.trace-list-controls button:disabled { opacity: .45; cursor: default; }
.trace-filter-toggle { justify-self: start; }
.trace-filter-form { display: grid; gap: 10px; padding: 12px; border: 1px solid var(--ui-border); border-radius: var(--ui-radius-md); background: var(--ui-surface-soft); }
.trace-filter-form label { display: grid; gap: 5px; font-size: var(--ui-font-xs); color: var(--ui-muted); }
.trace-filter-form input { width: 100%; }
.trace-filter-form button { justify-self: start; }
.trace-page-size { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: var(--ui-font-xs); color: var(--ui-muted); }
.trace-list-paging { display: flex; align-items: center; gap: 5px; font-size: var(--ui-font-sm); }
.trace-list-paging .trace-page-input { width: 54px; text-align: center; }
.trace-list-paging > span { margin-right: auto; color: var(--ui-muted); white-space: nowrap; }
.trace-list-controls p, .trace-list-controls small { font-size: var(--ui-font-xs); margin: 0; }
</style>
