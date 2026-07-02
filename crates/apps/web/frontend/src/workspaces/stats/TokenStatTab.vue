<template>
  <section class="token-stat-tab">
    <TokenFilterBar
      :from-date="fromDate"
      :to-date="toDate"
      :model-options="modelOptions"
      :model-selection="modelSelection"
      :categories="categoryOptions"
      :category-selection="categorySelection"
      :validity-selection="validitySelection"
      :loading="loading"
      @update-range="setDateRange"
      @update-model-selection="setModelSelection"
      @update-category-selection="setCategorySelection"
      @update-validity-selection="setValiditySelection"
      @query="runQuery"
      @abort-query="abortQuery"
    />

    <div v-if="error" class="detail-error">{{ error }}</div>

    <TokenUsageSummary
      :summary="filteredSummary"
      :selected-categories="selectedCategories"
      :loading="loading"
    />

    <TokenVisualizationPanel
      :requests="filteredRequests"
      :breakdown-by-category="breakdownByCategory"
      :breakdown-by-model="breakdownByModel"
      :selected-categories="selectedCategories"
      :loading="loading"
    />

    <TokenRequestPanel
      :requests="filteredRequests"
      :selected-categories="selectedCategories"
      :loading="loading"
      @open-trace="$emit('open-trace', $event)"
    />
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';

import { readTokenUsageStats } from '../../api';
import TokenFilterBar from './token/TokenFilterBar.vue';
import TokenRequestPanel from './token/TokenRequestPanel.vue';
import TokenUsageSummary from './token/TokenUsageSummary.vue';
import TokenVisualizationPanel from './token/TokenVisualizationPanel.vue';
import {
  TOKEN_CATEGORY_FILTERS,
  allTokenCategoryIds,
  applyTokenFilters,
  buildTokenBreakdownByCategory,
  buildTokenBreakdownByModel,
  buildTokenSummary,
  defaultDateRange,
  listTokenModels,
  rangeToMillis,
} from './tokenModel';

const props = defineProps({
  traces: {
    type: Array,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
});

const emit = defineEmits(['loading', 'open-trace']);

const initialRange = defaultDateRange(props.traces);
const fromDate = ref(initialRange.fromDate);
const toDate = ref(initialRange.toDate);
const rangeTouched = ref(false);
const stats = ref(emptyStats());
const error = ref('');
const loading = ref(false);
const modelSelection = ref({});
const categorySelection = ref(selectionFromIds(allTokenCategoryIds()));
const validitySelection = ref({ valid_llm: true });
const modelsTouched = ref(false);
let activeLoad = null;
let activeController = null;

const currentRange = computed(() => ({ fromDate: fromDate.value, toDate: toDate.value }));
const categoryOptions = TOKEN_CATEGORY_FILTERS;
const modelOptions = computed(() => listTokenModels(stats.value.requests));
const selectedModels = computed(() => selectedIdsFromOptions(modelOptions.value, modelSelection.value));
const selectedCategories = computed(() => selectedIdsFromOptions(categoryOptions, categorySelection.value));
const validOnly = computed(() => Boolean(validitySelection.value.valid_llm));
const filteredRequests = computed(() =>
  applyTokenFilters(stats.value.requests, {
    query: props.query,
    models: selectedModels.value,
    categories: selectedCategories.value,
    validOnly: validOnly.value,
  }),
);
const filteredSummary = computed(() => buildTokenSummary(filteredRequests.value));
const breakdownByCategory = computed(() =>
  buildTokenBreakdownByCategory(filteredRequests.value, selectedCategories.value),
);
const breakdownByModel = computed(() => buildTokenBreakdownByModel(filteredRequests.value));

watch(
  () => props.traces,
  (traces) => {
    if (rangeTouched.value) {
      return;
    }
    const next = defaultDateRange(traces);
    fromDate.value = next.fromDate;
    toDate.value = next.toDate;
  },
);

watch(
  modelOptions,
  (models) => {
    syncSelectedModels(models);
  },
  { immediate: true },
);

watch(
  loading,
  (value) => {
    emit('loading', value);
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  abortQuery();
  emit('loading', false);
});

onMounted(async () => {
  await loadStats(currentRange.value);
});

function setDateRange(range) {
  rangeTouched.value = true;
  fromDate.value = range.fromDate;
  toDate.value = range.toDate;
}

function setModelSelection(selection) {
  modelsTouched.value = true;
  modelSelection.value = normalizeSelection(modelOptions.value, selection);
}

function setCategorySelection(selection) {
  categorySelection.value = normalizeSelection(categoryOptions, selection);
}

function setValiditySelection(selection) {
  validitySelection.value = normalizeSelection([{ id: 'valid_llm' }], selection);
}

async function runQuery() {
  if (loading.value) {
    abortQuery();
    return;
  }
  await loadStats(currentRange.value);
}

function abortQuery() {
  activeController?.abort();
  activeController = null;
}

async function loadStats(range) {
  const parsed = rangeToMillis(range);
  if (!parsed.ok) {
    error.value = parsed.error;
    stats.value = emptyStats();
    return;
  }
  const token = Symbol();
  activeController?.abort();
  const controller = new AbortController();
  activeController = controller;
  activeLoad = token;
  loading.value = true;
  error.value = '';
  try {
    const data = await readTokenUsageStats({
      fromMs: parsed.fromMs,
      toMs: parsed.toMs,
      signal: controller.signal,
    });
    if (activeLoad === token) {
      stats.value = normalizeStats(data);
    }
  } catch (err) {
    if (err?.name === 'AbortError') {
      return;
    }
    if (activeLoad === token) {
      error.value = String(err.message ?? err);
      stats.value = emptyStats();
    }
  } finally {
    if (activeLoad === token) {
      activeController = null;
      loading.value = false;
    }
  }
}

function syncSelectedModels(models) {
  if (!modelsTouched.value) {
    modelSelection.value = selectionFromIds(models);
    return;
  }
  modelSelection.value = normalizeSelection(
    models.map((model) => ({ id: model })),
    modelSelection.value,
  );
}

function selectedIdsFromOptions(options, selection) {
  return options.filter((option) => Boolean(selection?.[option.id ?? option])).map((option) => option.id ?? option);
}

function selectionFromIds(ids, selected = true) {
  return Object.fromEntries(ids.map((id) => [id, selected]));
}

function normalizeSelection(options, selection) {
  return Object.fromEntries(
    options.map((option) => {
      const id = option.id ?? option;
      return [id, Boolean(selection?.[id])];
    }),
  );
}

function normalizeStats(data) {
  return {
    range: data?.range ?? null,
    summary: data?.summary ?? emptyStats().summary,
    requests: Array.isArray(data?.requests) ? data.requests : [],
  };
}

function emptyStats() {
  return {
    range: null,
    summary: {
      response_count: 0,
      usage_response_count: 0,
      missing_usage_count: 0,
      trace_count: 0,
      model_count: 0,
      prompt_tokens: 0,
      completion_tokens: 0,
      total_tokens: 0,
      cached_prompt_tokens: 0,
      reasoning_tokens: 0,
      prompt_cache_hit_tokens: 0,
      prompt_cache_miss_tokens: 0,
    },
    requests: [],
  };
}
</script>

<style scoped>
.token-stat-tab {
  min-width: 0;
  min-height: 0;
  height: 100%;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: var(--stats-section-gap);
  width: min(100%, var(--stats-shell-max-width));
  margin: 0 auto;
  padding: var(--stats-viewport-padding);
}

.token-stat-tab :deep(.token-visualization) {
  min-height: var(--stats-visualization-min-height);
  flex: 0 0 auto;
}

.token-stat-tab :deep(.token-request-panel) {
  min-height: var(--stats-request-panel-min-height);
  flex: 1 1 320px;
}

@media (max-width: 760px) {
  .token-stat-tab {
    gap: var(--stats-section-gap-mobile);
    padding: var(--stats-viewport-padding-mobile);
  }
}
</style>
