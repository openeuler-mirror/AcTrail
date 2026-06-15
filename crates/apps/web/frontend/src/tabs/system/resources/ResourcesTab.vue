<template>
  <TableTraceTab
    :trace-key="traceKey"
    :trace-detail="traceDetail"
    :action-tree="actionTree"
    :query="query"
    :projector="project"
  />
</template>

<script setup>
import TableTraceTab from '../../TableTraceTab.vue';
import { project } from './model';

defineProps({
  traceKey: {
    type: [String, Number],
    default: null,
  },
  traceDetail: {
    type: Object,
    default: null,
  },
  actionTree: {
    type: Object,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
});
</script>
