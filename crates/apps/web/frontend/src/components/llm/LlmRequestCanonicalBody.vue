<template>
  <section v-if="available" class="detail-section canonical-request-body">
    <h3>Canonical request body</h3>
    <p class="canonical-request-meta">
      {{ metadata.state }} · {{ formatBytes(metadata.bytes) }}
      <template v-if="metadata.blocks"> · {{ metadata.blocks }} blocks</template>
    </p>
    <LazyJsonTreeNode
      :key="actionId"
      :descriptor="rootDescriptor"
      :load-node="loadNode"
    />
  </section>
</template>

<script setup>
import { computed } from 'vue';

import { readActionLlmRequestContentNode } from '../../api';
import LazyJsonTreeNode from '../json/LazyJsonTreeNode.vue';

const props = defineProps({
  traceId: {
    type: [String, Number],
    default: null,
  },
  actionId: {
    type: String,
    default: '',
  },
  metadata: {
    type: Object,
    default: null,
  },
});

const available = computed(
  () => Boolean(props.traceId && props.actionId && props.metadata?.state === 'canonical_blocks'),
);
const rootDescriptor = computed(() => ({
  token: 'Load content',
  pointer: '',
  type: 'object',
  expandable: true,
  child_count: null,
}));

async function loadNode(query) {
  const response = await readActionLlmRequestContentNode(
    props.traceId,
    props.actionId,
    query,
  );
  return response.content?.node ?? null;
}

function formatBytes(value) {
  const bytes = Number(value);
  if (!Number.isFinite(bytes) || bytes < 0) {
    return 'unknown size';
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KiB`;
  }
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}
</script>

<style scoped>
.canonical-request-meta {
  margin: -2px 0 8px;
  color: var(--text-muted, #7b8494);
  font-size: 0.8rem;
}
</style>
