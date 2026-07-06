<template>
  <section class="overview-page">
    <div class="metrics">
      <MetricCard :label="t('stats.llm.metrics.totalTokens')" :value="formatNumber(summary.total_tokens)" />
      <MetricCard :label="t('stats.llm.metrics.completedRequests')" :value="formatNumber(summary.completed_requests)" />
      <MetricCard :label="t('stats.llm.metrics.missingUsage')" :value="formatNumber(summary.missing_usage_count)" />
      <MetricCard
        :label="t('stats.llm.metrics.promptCacheReuse')"
        :value="formatPercent(cacheReuseRate)"
        :detail="t('stats.llm.metrics.cacheHitTokenDetail', { count: formatNumber(summary.cache_hit_tokens) })"
      />
      <MetricCard
        :label="t('stats.llm.metrics.avgTtft')"
        :value="formatLatencyUs(latency.ttft.mean_us)"
        :detail="latencyDetail(latency.ttft)"
      />
      <MetricCard
        :label="t('stats.llm.metrics.avgTpot')"
        :value="formatLatencyUs(latency.tpot.mean_us)"
        :detail="latencyDetail(latency.tpot)"
      />
      <MetricCard v-if="pricingEnabled" :label="t('stats.llm.metrics.estimatedSpend')" :value="formatNumber(summary.estimated_spend_cny)" />
    </div>

    <div class="charts">
      <DistributionPanel
        :title="t('stats.llm.overview.tokenVolumeByModel')"
        :subtitle="topNSubtitle"
        :series="leaderboardSeries(overview.top_models, summary.total_tokens)"
        :share-denominator="summary.total_tokens"
        :format-value="formatNumber"
      />
      <DistributionPanel
        :title="t('stats.llm.overview.usageByEndpoint')"
        :subtitle="topNSubtitle"
        :series="leaderboardSeries(overview.top_endpoints, summary.total_tokens)"
        :share-denominator="summary.total_tokens"
        :format-value="formatNumber"
        :empty-label="t('stats.llm.overview.endpointEmpty')"
      />
      <DistributionPanel
        :title="t('stats.llm.overview.tokenBreakdown')"
        :series="tokenBreakdownSeries"
        :share-denominator="summary.total_tokens"
        :format-value="formatNumber"
        :resolve-hidden-keys="resolveBoundHiddenKeys"
        :resolve-visible-series="resolvePartitionedVisibleSeries"
      />
      <DistributionPanel
        :title="t('stats.llm.overview.usageByApp')"
        :subtitle="topNSubtitle"
        :series="leaderboardSeries(overview.top_apps, summary.total_tokens)"
        :share-denominator="summary.total_tokens"
        :format-value="formatNumber"
        :empty-label="t('stats.llm.overview.appEmpty')"
      />
    </div>

    <RequestRowsTable
      :rows="rows"
      :total-rows="rowTotal"
      :query="query"
      :can-load-more="canLoadMore"
      :title="t('stats.llm.overview.recentRows')"
      @load-more="$emit('load-more')"
      @open-trace="$emit('open-trace', $event)"
    />
  </section>
</template>

<script setup>
import { computed } from 'vue';

import { useLocale } from '../../../locale';
import DistributionPanel from './DistributionPanel.vue';
import MetricCard from './MetricCard.vue';
import RequestRowsTable from './RequestRowsTable.vue';
import {
  formatNumber,
  formatLatencyUs,
  formatPercent,
  resolveBoundHiddenKeys,
  resolvePartitionedVisibleSeries,
  tokenCategorySeries,
} from './model';

const props = defineProps({
  activity: {
    type: Object,
    required: true,
  },
  rows: {
    type: Array,
    default: () => [],
  },
  rowTotal: {
    type: Number,
    default: 0,
  },
  canLoadMore: {
    type: Boolean,
    default: false,
  },
  query: {
    type: String,
    default: '',
  },
});

defineEmits(['load-more', 'open-trace']);

const { t } = useLocale();
const summary = computed(() => props.activity.summary ?? {});
const overview = computed(() => props.activity.overview ?? {});
const latency = computed(() => ({
  ttft: normalizeLatencyDistribution(props.activity.latency?.ttft),
  tpot: normalizeLatencyDistribution(props.activity.latency?.tpot),
}));
const pricingEnabled = computed(() => Boolean(props.activity.capabilities?.pricing));
const tokenBreakdownSeries = computed(() => tokenCategorySeries(overview.value.token_categories ?? [], t));
const DEFAULT_OVERVIEW_TOP_N = 8;
const topNSubtitle = computed(() => t('stats.llm.common.topByTotalTokens', { count: DEFAULT_OVERVIEW_TOP_N }));
const cacheReuseRate = computed(() => {
  const input = Number(summary.value.input_tokens ?? 0);
  if (input <= 0) {
    return 0;
  }
  return Number(summary.value.cache_hit_tokens ?? 0) / input;
});

function leaderboardSeries(rows = [], denominator = 0) {
  const total = Number(denominator ?? 0);
  return rows.slice(0, DEFAULT_OVERVIEW_TOP_N).map((row) => ({
    key: row.key,
    label: row.label,
    total: row.total,
    share: total > 0 ? Number(row.total ?? 0) / total : 0,
  }));
}

function normalizeLatencyDistribution(value) {
  return {
    mean_us: value?.mean_us ?? null,
    p50_us: value?.p50_us ?? null,
    p95_us: value?.p95_us ?? null,
    sample_count: Number(value?.sample_count ?? 0),
  };
}

function latencyDetail(distribution) {
  return t('stats.llm.metrics.latencyDetail', {
    p50: formatLatencyUs(distribution.p50_us),
    p95: formatLatencyUs(distribution.p95_us),
    count: formatNumber(distribution.sample_count),
  });
}
</script>

<style scoped>
.overview-page {
  min-width: 0;
  display: grid;
  gap: var(--stats-section-gap);
}

.metrics {
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: var(--stats-space-lg);
}

.charts {
  min-width: 0;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  align-items: stretch;
  grid-auto-rows: 1fr;
  gap: var(--stats-space-lg);
}

.charts :deep(.distribution-panel:not(.expanded)) {
  height: 100%;
}

@media (max-width: 940px) {
  .metrics,
  .charts {
    grid-template-columns: minmax(0, 1fr);
    grid-auto-rows: auto;
  }
}
</style>
