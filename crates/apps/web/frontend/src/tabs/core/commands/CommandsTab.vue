<template>
  <TableTraceTab
    :trace-detail="traceDetail"
    :action-tree="actionTree"
    :tab-data="commands"
    :query="query"
    :projector="project"
    @select-detail="$emit('select-detail', $event)"
  />
</template>

<script setup>
import TableTraceTab from '../../TableTraceTab.vue';
import { project } from './model';

defineProps({
  traceDetail: {
    type: Object,
    default: null,
  },
  actionTree: {
    type: Object,
    required: true,
  },
  commands: {
    type: Object,
    required: true,
  },
  query: {
    type: String,
    default: '',
  },
});

defineEmits(['select-detail']);
</script>
