<template>
  <div v-if="insight" class="llm-insight-panel">
    <InsightPanel
      :insight="insight"
      :loading-message="requestLoading ? 'Loading request insights' : ''"
      :error="requestError"
    />
    <button
      v-if="requestContentAvailable && !requestContent"
      class="detail-load-button llm-insight-load"
      type="button"
      :disabled="requestLoading"
      @click="$emit('load-request-content')"
    >
      <span>{{ requestLoading ? 'Loading request insights' : 'Load request insights' }}</span>
    </button>
  </div>
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
  requestContentAvailable: {
    type: Boolean,
    default: false,
  },
});

defineEmits(['load-request-content']);

const insight = computed(() => buildLlmDetailInsight(props.detail, props.requestContent));
</script>

<style scoped>
.llm-insight-load {
  margin: 0 0 10px;
}
</style>
