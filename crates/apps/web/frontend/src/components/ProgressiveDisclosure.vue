<template>
  <div class="progressive-disclosure">
    <slot name="primary" />

    <div
      v-if="!enabled || expanded"
      :id="secondaryId"
      class="progressive-disclosure-secondary"
    >
      <slot name="secondary" />
    </div>

    <div v-if="enabled" class="progressive-disclosure-control">
      <span class="progressive-disclosure-copy">
        <strong>{{ t('progressiveDisclosure.detailLevel') }}</strong>
        <small>{{ expanded ? allLabel : focusedLabel }}</small>
      </span>
      <label class="progressive-disclosure-switch">
        <span>{{ focusedLabel }}</span>
        <input
          type="checkbox"
          :checked="expanded"
          :aria-label="t('progressiveDisclosure.showAll', { level: allLabel })"
          :aria-controls="secondaryId"
          @change="setExpanded($event.target.checked)"
        />
        <i aria-hidden="true"><b></b></i>
        <span>{{ allLabel }}</span>
      </label>
    </div>
  </div>
</template>

<script>
let nextDisclosureId = 0;
</script>

<script setup>
import { computed } from 'vue';

import { useModuleLocale } from '../locale';
import strings from './locale';

const { t } = useModuleLocale(strings);

const props = defineProps({
  enabled: {
    type: Boolean,
    default: true,
  },
  expanded: {
    type: Boolean,
    default: false,
  },
  focusedLabel: {
    type: String,
    default: '',
  },
  allLabel: {
    type: String,
    default: '',
  },
});

const focusedLabel = computed(() => props.focusedLabel || t('progressiveDisclosure.focused'));
const allLabel = computed(() => props.allLabel || t('progressiveDisclosure.allData'));

const emit = defineEmits(['update:expanded']);
const secondaryId = `progressive-disclosure-${++nextDisclosureId}`;
const expanded = computed(() => !props.enabled || props.expanded);

function setExpanded(value) {
  emit('update:expanded', value);
}
</script>

<style scoped>
.progressive-disclosure-control {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin: 18px 0 0;
  padding: 10px 12px;
  border: 1px solid var(--ui-border);
  border-radius: 8px;
  background: color-mix(in srgb, var(--ui-surface) 82%, var(--trace-interactive-bg));
}

.progressive-disclosure-copy {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.progressive-disclosure-copy strong {
  font-size: 12px;
}

.progressive-disclosure-copy small {
  color: var(--ui-muted);
  font-size: 11px;
}

.progressive-disclosure-switch {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--ui-muted);
  font-size: 11px;
  white-space: nowrap;
  cursor: pointer;
}

.progressive-disclosure-switch input {
  position: absolute;
  width: 1px;
  height: 1px;
  opacity: 0;
  pointer-events: none;
}

.progressive-disclosure-switch i {
  position: relative;
  width: 34px;
  height: 20px;
  border: 1px solid var(--ui-border);
  border-radius: 999px;
  background: var(--ui-surface-soft);
  transition: background 120ms ease, border-color 120ms ease;
}

.progressive-disclosure-switch b {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background: var(--ui-muted);
  transition: transform 120ms ease, background 120ms ease;
}

.progressive-disclosure-switch input:checked + i {
  border-color: var(--ui-accent);
  background: var(--trace-interactive-bg);
}

.progressive-disclosure-switch input:checked + i b {
  background: var(--ui-accent-strong);
  transform: translateX(14px);
}

.progressive-disclosure-switch input:focus-visible + i {
  outline: 2px solid var(--ui-accent);
  outline-offset: 2px;
}
</style>
