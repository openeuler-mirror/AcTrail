<template>
  <Tooltip :text="text" :delay="0">
    <button type="button" class="hint-trigger" :aria-label="ariaLabel">
      <CircleHelp :size="14" aria-hidden="true" />
    </button>
  </Tooltip>
</template>

<script setup>
import { computed } from 'vue';
import { CircleHelp } from '@lucide/vue';

import Tooltip from './Tooltip.vue';

const props = defineProps({
  /** Copy revealed on hover or keyboard focus. */
  text: { type: String, required: true },
  /** Accessible name of the trigger; defaults to the hint copy. */
  label: { type: String, default: '' },
});

const ariaLabel = computed(() => props.label || props.text);
</script>

<style scoped>
.hint-trigger {
  display: grid;
  place-items: center;
  padding: 0;
  border: 0;
  border-radius: 50%;
  background: transparent;
  color: var(--ui-muted);
  cursor: help;
}

.hint-trigger:hover {
  color: var(--ui-accent);
}

.hint-trigger:focus-visible {
  color: var(--ui-accent);
  outline: 2px solid color-mix(in srgb, var(--ui-accent) 45%, transparent);
  outline-offset: 2px;
}
</style>
