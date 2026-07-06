<template>
  <details ref="root" class="toolbar-icon-picker">
    <summary :title="selectedTitle" :aria-label="selectedTitle" aria-haspopup="listbox">
      <OptionIcon :option="selectedOption" />
      <ChevronDown :size="14" aria-hidden="true" />
    </summary>

    <div class="picker-menu" role="listbox" :aria-label="label">
      <button
        v-for="option in options"
        :key="option.id"
        type="button"
        role="option"
        :aria-selected="isSelected(option.id)"
        :class="{ selected: isSelected(option.id) }"
        @click="selectOption(option.id)"
      >
        <OptionIcon :option="option" />
        <span>{{ option.label }}</span>
        <Check v-if="isSelected(option.id)" :size="14" aria-hidden="true" />
      </button>
    </div>
  </details>
</template>

<script setup>
import { computed, h, onBeforeUnmount, onMounted, ref } from 'vue';
import { Check, ChevronDown } from '@lucide/vue';

const props = defineProps({
  label: {
    type: String,
    required: true,
  },
  modelValue: {
    type: String,
    required: true,
  },
  options: {
    type: Array,
    required: true,
  },
});

const emit = defineEmits(['update:modelValue']);
const root = ref(null);
const selectedOption = computed(
  () =>
    props.options.find((option) => option.id === props.modelValue) ??
    props.options[0] ?? { id: props.modelValue, label: props.modelValue },
);
const selectedTitle = computed(() => `${props.label}: ${selectedOption.value.label}`);

const OptionIcon = {
  props: {
    option: {
      type: Object,
      required: true,
    },
  },
  setup(optionProps) {
    return () => renderOptionIcon(optionProps.option);
  },
};

onMounted(() => {
  document.addEventListener('pointerdown', closeOnOutside);
  document.addEventListener('keydown', closeOnEscape);
});

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', closeOnOutside);
  document.removeEventListener('keydown', closeOnEscape);
});

function selectOption(optionId) {
  emit('update:modelValue', optionId);
  if (root.value) {
    root.value.open = false;
  }
}

function isSelected(optionId) {
  return optionId === props.modelValue;
}

function closeOnOutside(event) {
  if (root.value && !root.value.contains(event.target)) {
    root.value.open = false;
  }
}

function closeOnEscape(event) {
  if (event.key === 'Escape' && root.value?.open) {
    root.value.open = false;
  }
}

function renderOptionIcon(option) {
  if (option.flagIcon) {
    return h('span', { class: ['picker-flag', `picker-flag-${option.flagIcon}`], 'aria-hidden': 'true' });
  }
  if (option.flag) {
    return h('span', { class: 'picker-emoji-flag', 'aria-hidden': 'true' }, option.flag);
  }
  if (Array.isArray(option.swatch) && option.swatch.length) {
    return h('span', {
      class: 'picker-swatch',
      style: swatchStyle(option.swatch),
      'aria-hidden': 'true',
    });
  }
  return h('span', { class: 'picker-initials', 'aria-hidden': 'true' }, initials(option.label));
}

function swatchStyle(swatch) {
  const [primary, secondary = primary, surface = secondary] = swatch;
  return {
    '--picker-swatch-primary': primary,
    '--picker-swatch-secondary': secondary,
    '--picker-swatch-surface': surface,
  };
}

function initials(label) {
  return String(label ?? '')
    .trim()
    .split(/\s+/)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? '')
    .join('');
}
</script>

<style scoped>
.toolbar-icon-picker {
  position: relative;
  z-index: 1;
  flex: 0 0 auto;
}

.toolbar-icon-picker[open] {
  z-index: 200;
}

summary {
  width: 42px;
  height: 38px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 2px;
  padding: 0;
  border: 1px solid var(--stats-border, var(--border));
  border-radius: var(--stats-radius-md, 8px);
  background: var(--stats-surface, var(--surface));
  color: var(--stats-muted, var(--muted));
  cursor: pointer;
  list-style: none;
  backdrop-filter: var(--stats-control-filter, none);
}

summary::-webkit-details-marker {
  display: none;
}

summary:hover,
.toolbar-icon-picker[open] summary {
  border-color: var(--stats-accent-soft, var(--teal));
  color: var(--stats-accent, var(--teal-deep));
}

.picker-menu {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  z-index: 200;
  min-width: 168px;
  display: grid;
  gap: 2px;
  padding: 6px;
  border: 1px solid var(--stats-border, var(--border));
  border-radius: var(--stats-radius-md, 8px);
  background: var(--stats-surface-strong, var(--surface));
  box-shadow: var(--stats-shadow, var(--shadow));
  backdrop-filter: var(--stats-control-filter, none);
}

.picker-menu button {
  min-width: 0;
  height: 34px;
  display: grid;
  grid-template-columns: 24px minmax(0, 1fr) 16px;
  align-items: center;
  gap: 8px;
  padding: 0 8px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--stats-text, var(--text));
  cursor: pointer;
  font: inherit;
  font-size: 13px;
  text-align: left;
}

.picker-menu button:hover,
.picker-menu button.selected {
  background: var(--stats-accent-muted, var(--surface-muted));
  color: var(--stats-accent, var(--teal-deep));
}

.picker-menu button span:nth-child(2) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.picker-flag,
.picker-emoji-flag,
.picker-initials,
.picker-swatch {
  width: 24px;
  height: 24px;
  overflow: hidden;
  display: inline-grid;
  border: 1px solid var(--stats-border, var(--border));
  border-radius: 50%;
  background: var(--stats-surface-soft, var(--surface-muted));
  color: var(--stats-text, var(--text));
  font-size: 10px;
  font-weight: 700;
}

.picker-emoji-flag {
  place-items: center;
  font-size: 18px;
  line-height: 1;
}

.picker-flag {
  position: relative;
  border-radius: 50%;
  background: var(--stats-surface-soft, var(--surface-muted));
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.04);
}

.picker-flag-us {
  background:
    linear-gradient(180deg, #b22234 0 7.69%, #fff 7.69% 15.38%, #b22234 15.38% 23.07%, #fff 23.07% 30.76%, #b22234 30.76% 38.45%, #fff 38.45% 46.14%, #b22234 46.14% 53.83%, #fff 53.83% 61.52%, #b22234 61.52% 69.21%, #fff 69.21% 76.9%, #b22234 76.9% 84.59%, #fff 84.59% 92.28%, #b22234 92.28% 100%);
}

.picker-flag-us::before {
  content: "";
  position: absolute;
  inset: 0 44% 48% 0;
  background: #3c3b6e;
}

.picker-flag-cn {
  background: #de2910;
}

.picker-flag-cn::before {
  content: "";
  position: absolute;
  width: 7px;
  height: 7px;
  left: 5px;
  top: 5px;
  clip-path: polygon(50% 0, 61% 34%, 98% 34%, 68% 55%, 79% 91%, 50% 69%, 21% 91%, 32% 55%, 2% 34%, 39% 34%);
  background: #ffde00;
}

.picker-initials {
  place-items: center;
}

.picker-swatch {
  background:
    linear-gradient(135deg, var(--picker-swatch-primary) 0 46%, transparent 46%),
    linear-gradient(45deg, var(--picker-swatch-secondary) 0 52%, var(--picker-swatch-surface) 52%);
}

</style>
