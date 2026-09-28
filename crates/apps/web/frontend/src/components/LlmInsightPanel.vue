<template>
  <div v-if="insight" class="llm-insight-panel">
    <InsightPanel
      :insight="insight"
      :loading-message="requestLoading ? t('llmInsight.loading') : ''"
      :error="requestError"
    />
    <button
      v-if="requestContentAvailable"
      class="detail-load-button llm-insight-load"
      type="button"
      :disabled="requestLoading"
      :aria-expanded="requestContent ? !requestInsightsHidden : false"
      @click="toggleRequestInsights"
    >
      <span>{{ requestInsightButtonLabel }}</span>
    </button>
  </div>
</template>

<script setup>
import { computed, ref, watch } from 'vue';

import { buildLlmDetailInsight } from '../llm/insight';
import { useModuleLocale } from '../locale';
import InsightPanel from './InsightPanel.vue';
import strings from './locale';

const { t } = useModuleLocale(strings);

const props = defineProps({
  detail: {
    type: Object,
    default: null,
  },
  requestContent: {
    type: Object,
    default: null,
  },
  requestLoading: {
    type: Boolean,
    default: false,
  },
  requestError: {
    type: String,
    default: '',
  },
  requestContentAvailable: {
    type: Boolean,
    default: false,
  },
});

const emit = defineEmits(['load-request-content']);
const requestInsightsHidden = ref(false);

const effectiveRequestContent = computed(() => (
  requestInsightsHidden.value ? null : props.requestContent
));
const insight = computed(() => buildLlmDetailInsight(props.detail, effectiveRequestContent.value));
const requestInsightButtonLabel = computed(() => {
  if (props.requestLoading) {
    return t('llmInsight.loading');
  }
  if (!props.requestContent) {
    return t('llmInsight.load');
  }
  return requestInsightsHidden.value ? t('llmInsight.show') : t('llmInsight.hide');
});

watch(
  () => props.detail?.raw?.id,
  () => {
    requestInsightsHidden.value = false;
  },
);

watch(
  () => props.requestContent,
  (content, previousContent) => {
    if (content && !previousContent) {
      requestInsightsHidden.value = false;
    }
  },
);

function toggleRequestInsights() {
  if (!props.requestContent) {
    emit('load-request-content');
    return;
  }
  requestInsightsHidden.value = !requestInsightsHidden.value;
}
</script>

<style scoped>
.llm-insight-load {
  margin: 0 0 10px;
}
</style>
