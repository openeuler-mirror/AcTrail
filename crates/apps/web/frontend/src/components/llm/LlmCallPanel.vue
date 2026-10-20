<template>
  <div class="llm-call-panel">
    <LlmCallMessage
      :key="`${traceId}:${call.id}:request:${requestId}`"
      :trace-id="traceId"
      :action-id="requestId"
      kind="llm.request"
    />
    <LlmCallMessage
      :key="`${traceId}:${call.id}:response:${responseId}`"
      :trace-id="traceId"
      :action-id="responseId"
      :kind="responseKind"
    />
  </div>
</template>

<script setup>
import { computed } from 'vue';
import LlmCallMessage from './LlmCallMessage.vue';

const props = defineProps({
  traceId: { type: [String, Number], required: true },
  call: { type: Object, required: true },
});

const requestId = computed(() => props.call.attributes?.['llm.call.request_action_id'] ?? '');
const responseId = computed(() => props.call.attributes?.['llm.call.response_action_id']
  || props.call.attributes?.['llm.call.http_response_action_id'] || '');
const responseKind = computed(() => props.call.attributes?.['llm.call.response_action_id']
  ? 'llm.response' : 'http.message');
</script>

<style scoped>
.llm-call-panel { display: grid; gap: 14px; }
</style>
