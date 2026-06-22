<template>
  <InsightPanel :insight="insight" />
</template>

<script setup>
import { computed } from 'vue';

import { buildHttpDetailInsight } from '../http/insight';
import InsightPanel from './InsightPanel.vue';

const props = defineProps({
  detail: {
    type: Object,
    default: null,
  },
});

const insight = computed(() => buildHttpDetailInsight(props.detail));
</script>
