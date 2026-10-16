<template>
  <section class="idle-controls" :aria-busy="loading" :data-page="data.page" :data-count="data.intervals.length" :data-has-more="data.hasMore">
    <form @submit.prevent="pager.load()">
      <label>{{ t('limit') }}<input v-model.number="limit" class="idle-page-size" type="number" min="1" required /></label>
      <button type="submit" class="idle-load" :disabled="loading">{{ t('load') }}</button>
      <button type="button" class="idle-next" :disabled="loading || !data.hasMore" @click="pager.next()">{{ t('next') }}</button>
    </form>
    <p>{{ t('range', { from: rangeLabel(range.from), to: rangeLabel(range.to) }) }}</p>
    <p v-if="data.page">{{ t('partial', { page: data.page, count: data.intervals.length, scanned: data.scanned }) }}</p>
    <p v-if="data.page && !data.hasMore">{{ t('end') }}</p>
    <p v-if="loading" role="status">{{ t('loading') }}</p>
    <p v-if="error" role="alert">{{ error }} <button type="button" :disabled="loading" @click="pager.retry?.()">{{ t('retry') }}</button></p>
  </section>
</template>

<script setup>
import { useModuleLocale } from '../../../../locale';
const props = defineProps({ pager: { type: Object, required: true } });
const { data, loading, error, limit, range } = props.pager;
const { t } = useModuleLocale({
  'en-US': {
    limit: 'Stall intervals per page', load: 'Load stall intervals for current range', next: 'Next stall page',
    range: 'Stall range: {from} – {to}', unbounded: 'No time bound',
    partial: 'Stall page {page}: {count} intervals; {scanned} candidates scanned. Only this interval page is shown.',
    end: 'No more stall candidates in this range.', loading: 'Loading stall intervals…', retry: 'Retry',
  },
  'zh-CN': {
    limit: '每页停滞区间数', load: '加载当前范围的停滞区间', next: '停滞区间下一页',
    range: '停滞范围：{from} – {to}', unbounded: '不限时间',
    partial: '停滞区间第 {page} 页：{count} 个区间，已扫描 {scanned} 个候选；仅展示当前区间页。',
    end: '此范围没有更多停滞候选。', loading: '正在加载停滞区间…', retry: '重试',
  },
});
function rangeLabel(value) {
  return value ? new Date(Number(BigInt(value) / 1_000_000n)).toLocaleString() : t('unbounded');
}
</script>

<style scoped>
.idle-controls { padding: 10px 16px; border-bottom: 1px solid var(--ui-border); }
form { display: flex; flex-wrap: wrap; align-items: end; gap: 8px; }
label { display: grid; gap: 3px; font-size: 12px; }
input { width: 90px; padding: 5px; }
button { padding: 5px 8px; }
p { font-size: 12px; margin: 6px 0; }
</style>
