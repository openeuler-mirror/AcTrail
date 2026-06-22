<template>
  <InsightPanel
    :insight="insight"
    :loading-message="requestLoading ? 'Loading request body' : ''"
    :error="requestError"
  />
</template>

<script setup>
import { computed } from 'vue';

import { buildLlmDetailInsight } from '../llm/insight';
import InsightPanel from './InsightPanel.vue';

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
});

const insight = computed(() => buildLlmDetailInsight(props.detail, props.requestContent));
</script>
