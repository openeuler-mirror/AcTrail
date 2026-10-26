<template>
  <InsightPanel
    :insight="insight"
    :loading-message="payloadLoading ? t('mcpInsight.loading') : ''"
    :error="payloadError"
  />
</template>

<script setup>
import { computed } from 'vue';

import { buildMcpDetailInsight } from '../mcp/insight';
import { useModuleLocale } from '../locale';
import InsightPanel from './InsightPanel.vue';
import strings from './locale';

const { t } = useModuleLocale(strings);

const props = defineProps({
  detail: {
    type: Object,
    default: null,
  },
  payload: {
    type: [String, Object],
    default: '',
  },
  payloadLoading: {
    type: Boolean,
    default: false,
  },
  payloadError: {
    type: String,
    default: '',
  },
});

const insight = computed(() => buildMcpDetailInsight(props.detail, props.payload));
</script>
