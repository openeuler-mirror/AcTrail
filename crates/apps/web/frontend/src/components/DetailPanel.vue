<template>
  <aside class="detail-panel">
    <div class="detail-header">
      <div>
        <span>{{ detailKind }}</span>
        <h2>{{ detailTitle }}</h2>
      </div>
      <button class="icon-button subtle-button" type="button" title="Clear" @click="$emit('clear')">
        <X :size="18" aria-hidden="true" />
      </button>
    </div>

    <dl v-if="detailRows.length" class="detail-rows">
      <template v-for="[key, value] in detailRows" :key="key">
        <dt>{{ key }}</dt>
        <dd>{{ value }}</dd>
      </template>
    </dl>

    <section v-if="panelError" class="detail-section">
      <h3>Error</h3>
      <p class="detail-error">{{ panelError }}</p>
    </section>

    <section v-if="Object.keys(detailAttributes).length" class="detail-section">
      <h3>Attributes</h3>
      <JsonTree :value="detailAttributes" />
    </section>

    <section v-if="payloadText" class="detail-section">
      <h3>Payload</h3>
      <pre>{{ payloadText }}</pre>
    </section>

    <section v-if="detailRawValue" class="detail-section">
      <h3>JSON</h3>
      <JsonTree :value="detailRawValue" />
    </section>
  </aside>
</template>

<script setup>
import { computed, ref, watch } from 'vue';
import { X } from '@lucide/vue';

import { readPayload } from '../api';
import JsonTree from './JsonTree.vue';

const props = defineProps({
  detail: {
    type: Object,
    default: null,
  },
  traceId: {
    type: [String, Number],
    default: null,
  },
  error: {
    type: String,
    default: '',
  },
});

defineEmits(['clear']);

const payloadText = ref('');
const payloadError = ref('');
let activePayloadLoad = null;

const detailTitle = computed(() => props.detail?.title ?? 'No selection');
const detailKind = computed(() => props.detail?.kind ?? 'detail');
const detailRows = computed(() => Object.entries(props.detail?.rows ?? {}));
const detailAttributes = computed(() => props.detail?.attributes ?? {});
const detailRawValue = computed(() => props.detail?.raw ?? null);
const panelError = computed(() => props.error || payloadError.value);

watch(
  () => [props.detail, props.traceId],
  async ([nextDetail, traceId]) => {
    const token = Symbol();
    activePayloadLoad = token;
    payloadText.value = '';
    payloadError.value = '';
    if (!nextDetail?.payloadId || !traceId) {
      return;
    }
    try {
      const payload = await readPayload(traceId, nextDetail.payloadId);
      if (activePayloadLoad === token) {
        payloadText.value = payload.text ?? '';
      }
    } catch (err) {
      if (activePayloadLoad === token) {
        payloadError.value = String(err.message ?? err);
      }
    }
  },
  { immediate: true },
);
</script>
