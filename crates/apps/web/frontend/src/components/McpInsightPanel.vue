<template>
  <InsightPanel
    :insight="insight"
    :loading-message="payloadLoading ? 'Loading MCP payload' : ''"
    :error="payloadError"
  />
</template>

<script setup>
import { computed } from 'vue';

import { buildMcpDetailInsight } from '../mcp/insight';
import InsightPanel from './InsightPanel.vue';

const props = defineProps({
  detail: {
    type: Object,
    default: null,
  },
  payloadText: {
    type: String,
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

const insight = computed(() => buildMcpDetailInsight(props.detail, props.payloadText));
</script>
