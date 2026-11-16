<template>
  <button
    type="button"
    class="checkpoint-marker"
    :class="`checkpoint-${tone}`"
    :title="label"
    :aria-label="label"
    @click.stop="$emit('open', marker)"
    @keydown.enter.stop
    @keydown.space.stop
  >
    C{{ marker.checkpointId ?? '?' }}
  </button>
</template>

<script setup>
import { computed } from 'vue';

import { useModuleLocale } from '../../../locale';
import strings from './locale';

const props = defineProps({
  marker: {
    type: Object,
    required: true,
  },
});
defineEmits(['open']);

const { t } = useModuleLocale(strings);
const succeeded = computed(() => props.marker.outcome === 'succeeded');
const tone = computed(() => {
  if (props.marker.phase !== 'finished' || !succeeded.value) return 'attention';
  return props.marker.unavailable ? 'unavailable' : 'available';
});
const label = computed(() => {
  const { kind, checkpointId, phase, outcome } = props.marker;
  const progress = outcome ? t(`status.${outcome}`) : t(`phases.${phase}`);
  if (kind === 'save') {
    return succeeded.value
      ? t('markerSave', { id: checkpointId })
      : t('markerSaveFailed', { outcome: progress });
  }
  return succeeded.value
    ? t('markerRestore', { id: checkpointId })
    : t('markerRestorePending', { id: checkpointId, phase: progress });
});
</script>

<style scoped>
.checkpoint-marker {
  min-width: 28px;
  height: 22px;
  padding: 0 7px;
  border: 1px solid var(--trace-interactive-border);
  border-radius: 999px;
  background: var(--trace-interactive-bg);
  color: var(--trace-interactive-text);
  font-size: 10px;
  font-weight: 600;
  cursor: pointer;
}

.checkpoint-marker.checkpoint-unavailable {
  border-color: var(--ui-border);
  border-style: dashed;
  background: var(--ui-surface-soft);
  color: var(--ui-muted);
}

.checkpoint-marker.checkpoint-attention {
  border-color: var(--ui-warning);
  color: var(--ui-warning);
  background: color-mix(in srgb, var(--ui-warning) 8%, var(--ui-surface));
}

.checkpoint-marker:hover,
.checkpoint-marker:focus-visible {
  border-color: var(--trace-interactive-text);
  box-shadow: 0 0 0 3px rgba(45, 143, 133, 0.16);
  outline: none;
}
</style>
