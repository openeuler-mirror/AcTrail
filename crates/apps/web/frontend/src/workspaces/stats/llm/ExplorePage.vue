<template>
  <section class="explore-page">
    <section class="query-builder" :aria-busy="loading">
      <header class="query-head">
        <div>
          <h3>{{ queryTitle }}</h3>
          <p>{{ querySubtitle }}</p>
        </div>
        <span class="query-state">{{ loading ? t('stats.llm.compare.updating') : t('stats.llm.compare.ready') }}</span>
      </header>

      <div class="query-controls">
        <label class="query-field wide">
          <span>{{ t('stats.llm.compare.measure') }}</span>
          <select v-model="metric">
            <option v-for="option in metrics" :key="option.id" :value="option.id">{{ option.label }}</option>
          </select>
        </label>
        <label class="query-field wide">
          <span>{{ t('stats.llm.compare.breakDownBy') }}</span>
          <select v-model="group">
            <option v-for="option in groups" :key="option.id" :value="option.id">{{ option.label }}</option>
          </select>
        </label>
        <label class="query-field">
          <span>{{ t('stats.llm.compare.timeBucket') }}</span>
          <select v-model="rollup">
            <option v-for="option in rollups" :key="option.id" :value="option.id">{{ option.label }}</option>
          </select>
        </label>
        <label class="query-field">
          <span>{{ t('stats.llm.compare.show') }}</span>
          <select v-model.number="topN">
            <option v-for="option in topOptions" :key="option" :value="option">
              {{ t('stats.llm.common.topGroups', { count: option }) }}
            </option>
          </select>
        </label>
        <fieldset class="query-segment">
          <legend>{{ t('stats.llm.compare.rank') }}</legend>
          <div>
            <button
              v-for="option in sortOptions"
              :key="option.id"
              type="button"
              :class="{ active: sort === option.id }"
              @click="sort = option.id"
            >
              {{ option.label }}
            </button>
          </div>
        </fieldset>
        <fieldset class="query-segment chart-segment">
          <legend>{{ t('stats.llm.compare.view') }}</legend>
          <div>
            <button
              v-for="option in chartKinds"
              :key="option.id"
              type="button"
              :class="{ active: chartKind === option.id }"
              @click="chartKind = option.id"
            >
              {{ option.label }}
            </button>
          </div>
        </fieldset>
      </div>
    </section>

    <div v-if="error" class="error">{{ error }}</div>

    <ChartPanel
      :title="queryTitle"
      :subtitle="querySubtitle"
      :series="result.series ?? []"
      :mode="chartKind"
      :format-value="formatNumber"
      :empty-label="emptyGroupsLabel"
    />

    <div class="result-table">
      <table v-if="(result.rows ?? []).length">
        <thead>
          <tr>
            <th>{{ selectedGroup.label }}</th>
            <th class="numeric">{{ metricColumnLabel }}</th>
            <th class="numeric">{{ shareColumnLabel }}</th>
            <th class="numeric">{{ t('stats.llm.compare.completed') }}</th>
            <th class="numeric">{{ t('stats.llm.metrics.missingUsage') }}</th>
            <th class="numeric">{{ t('stats.llm.compare.traces') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="row in result.rows" :key="row.key">
            <td>{{ row.label }}</td>
            <td class="numeric">{{ formatNumber(row.total) }}</td>
            <td class="numeric">{{ formatPercent(row.share) }}</td>
            <td class="numeric">{{ formatNumber(row.completed_requests) }}</td>
            <td class="numeric">{{ formatNumber(row.missing_usage_count) }}</td>
            <td class="numeric">{{ formatNumber(row.trace_count) }}</td>
          </tr>
        </tbody>
      </table>
      <div v-else class="empty">{{ emptyGroupsLabel }}</div>
    </div>
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import { runLlmRequestsExplore } from '../../../api';
import { useLocale } from '../../../locale';
import ChartPanel from './ChartPanel.vue';
import {
  CHART_KINDS,
  EXPLORE_GROUPS,
  EXPLORE_METRICS,
  ROLLUPS,
  TOP_N_OPTIONS,
  formatNumber,
  formatPercent,
} from './model';

const props = defineProps({
  fromMs: {
    type: Number,
    required: true,
  },
  toMs: {
    type: Number,
    required: true,
  },
  defaultRollup: {
    type: String,
    default: 'day',
  },
});

const { t } = useLocale();
const metrics = computed(() => EXPLORE_METRICS.map(localizedMetric));
const groups = computed(() => EXPLORE_GROUPS.map(localizedGroup));
const rollups = computed(() => ROLLUPS.map(localizedRollup));
const chartKinds = computed(() => CHART_KINDS.map(localizedChartKind));
const topOptions = TOP_N_OPTIONS;
const sortOptions = computed(() => [
  { id: 'top', label: t('stats.llm.compare.highestFirst') },
  { id: 'bottom', label: t('stats.llm.compare.lowestFirst') },
]);
const metric = ref('total_tokens');
const group = ref('model');
const rollup = ref(validRollup(props.defaultRollup) ? props.defaultRollup : 'day');
const topN = ref(10);
const sort = ref('top');
const chartKind = ref('bar');
const result = ref({});
const error = ref('');
const loading = ref(false);
let controller = null;

const selectedMetric = computed(() => optionById(metrics.value, metric.value));
const selectedGroup = computed(() => optionById(groups.value, group.value));
const selectedRollup = computed(() => optionById(rollups.value, rollup.value));
const selectedSort = computed(() => optionById(sortOptions.value, sort.value));
const selectedChart = computed(() => optionById(chartKinds.value, chartKind.value));
const selectedDimensionPlural = computed(() => t(`stats.llm.dimensions.${dimensionPluralKey(group.value)}`));
const queryTitle = computed(
  () =>
    t('stats.llm.compare.title', {
      count: topN.value,
      dimension: selectedDimensionPlural.value,
      metric: selectedMetric.value.label,
    }),
);
const querySubtitle = computed(
  () =>
    t('stats.llm.compare.subtitle', {
      rank: selectedSort.value.label,
      rollup: selectedRollup.value.label,
      view: selectedChart.value.label,
    }),
);
const metricColumnLabel = computed(() => selectedMetric.value.label);
const shareColumnLabel = computed(() =>
  t('stats.llm.compare.shareOf', { metric: selectedMetric.value.label.toLowerCase() }),
);
const emptyGroupsLabel = computed(() =>
  t('stats.llm.compare.emptyGroups', { dimension: selectedDimensionPlural.value }),
);
const controlSignature = computed(
  () => `${metric.value}|${group.value}|${rollup.value}|${topN.value}|${sort.value}|${chartKind.value}`,
);

watch(
  () => props.defaultRollup,
  (value) => {
    if (validRollup(value)) {
      rollup.value = value;
    }
  },
);

watch(
  () => [props.fromMs, props.toMs],
  () => {
    runExplore();
  },
);

watch(controlSignature, () => {
  runExplore();
});

onMounted(() => {
  runExplore();
});

onBeforeUnmount(() => {
  controller?.abort();
});

async function runExplore() {
  if (!props.fromMs || !props.toMs) {
    return;
  }
  controller?.abort();
  const requestController = new AbortController();
  controller = requestController;
  loading.value = true;
  error.value = '';
  try {
    result.value = await runLlmRequestsExplore(
      {
        from_ms: props.fromMs,
        to_ms: props.toMs,
        metric: metric.value,
        group: group.value,
        rollup: rollup.value,
        top_n: topN.value,
        sort: sort.value,
        chart_kind: chartKind.value,
      },
      { signal: requestController.signal },
    );
  } catch (err) {
    if (err?.name !== 'AbortError') {
      error.value = String(err.message ?? err);
      result.value = {};
    }
  } finally {
    if (controller === requestController) {
      loading.value = false;
    }
  }
}

function optionById(options, id) {
  return options.find((option) => option.id === id) ?? options[0] ?? { id, label: String(id) };
}

function validRollup(value) {
  return ROLLUPS.some((option) => option.id === value);
}

function localizedMetric(option) {
  return {
    id: option.id,
    label: t(metricLocaleKey(option.id)),
  };
}

function localizedGroup(option) {
  return {
    id: option.id,
    label: t(`stats.llm.dimensions.${option.id}`),
  };
}

function localizedRollup(option) {
  return {
    id: option.id,
    label: t(`stats.llm.rollups.${option.id}`),
  };
}

function localizedChartKind(option) {
  return {
    id: option.id,
    label: t(chartLocaleKey(option.id)),
  };
}

function metricLocaleKey(id) {
  switch (id) {
    case 'total_tokens':
      return 'stats.llm.metrics.totalTokens';
    case 'input_tokens':
      return 'stats.llm.metrics.inputTokens';
    case 'output_tokens':
      return 'stats.llm.metrics.outputTokens';
    case 'reasoning_tokens':
      return 'stats.llm.metrics.reasoningTokens';
    case 'cache_hit_tokens':
      return 'stats.llm.metrics.cacheHitTokens';
    case 'cache_miss_tokens':
      return 'stats.llm.metrics.cacheMissTokens';
    case 'completed_requests':
      return 'stats.llm.metrics.completedRequests';
    case 'missing_usage_count':
      return 'stats.llm.metrics.missingUsage';
    default:
      return id;
  }
}

function chartLocaleKey(id) {
  switch (id) {
    case 'line':
      return 'stats.llm.common.line';
    case 'bar':
      return 'stats.llm.common.bars';
    case 'histogram':
      return 'stats.llm.common.timeBars';
    case 'donut':
      return 'stats.llm.common.share';
    default:
      return id;
  }
}

function dimensionPluralKey(id) {
  switch (id) {
    case 'model':
      return 'models';
    case 'endpoint':
      return 'endpoints';
    case 'app':
      return 'apps';
    default:
      return 'groups';
  }
}
</script>

<style scoped src="./ExplorePage.css"></style>
