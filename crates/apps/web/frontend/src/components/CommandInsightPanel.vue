<template>
  <InsightPanel :insight="insight" />
</template>

<script setup>
import { computed } from 'vue';

import { buildCommandDetailInsight } from '../command/insight';
import InsightPanel from './InsightPanel.vue';

const props = defineProps({
  detail: {
    type: Object,
    default: null,
  },
});

const insight = computed(() => buildCommandDetailInsight(props.detail));
</script>
