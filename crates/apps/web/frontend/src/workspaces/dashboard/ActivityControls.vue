<template>
  <section class="dashboard-activity-controls" :aria-busy="loading">
    <form @submit.prevent="activity.submit()">
      <label>{{ t('dashboard.activityFrom') }}<input v-model="draft.from" name="activity-from" type="datetime-local" step="1" required :max="draft.to" /></label>
      <label>{{ t('dashboard.activityTo') }}<input v-model="draft.to" name="activity-to" type="datetime-local" step="1" required :min="draft.from" /></label>
      <button class="dashboard-activity-load" type="submit" :disabled="loading || draft.from >= draft.to">{{ t(loading ? 'dashboard.activityLoading' : 'dashboard.activityLoad') }}</button>
    </form>
    <p v-if="data" class="dashboard-activity-range">{{ t('dashboard.activityRange', { from: date(data.range.from_ms), to: date(data.range.to_ms) }) }}</p>
    <p v-else>{{ t('dashboard.activityNotLoaded') }}</p>
    <p v-if="error" role="alert">{{ error }} <button class="dashboard-activity-retry" type="button" :disabled="loading" @click="activity.refresh()">{{ t('dashboard.activityRetry') }}</button></p>
  </section>
</template>
<script setup>
import { useLocale } from '../../locale';
const props = defineProps({ activity: { type: Object, required: true } });
const { draft, data, loading, error } = props.activity;
const { t } = useLocale();
const date = value => new Date(value).toLocaleString();
</script>
<style scoped>
.dashboard-activity-controls { margin: 20px 0; }
form { display: flex; flex-wrap: wrap; align-items: end; gap: 12px; }
label { display: grid; gap: 5px; font-size: 12px; }
input, button { padding: 7px 10px; border: 1px solid var(--ui-border); border-radius: 6px; background: var(--ui-surface); color: inherit; }
p { font-size: 12px; color: var(--ui-muted); }
[role=alert] { color: var(--ui-danger); }
</style>
