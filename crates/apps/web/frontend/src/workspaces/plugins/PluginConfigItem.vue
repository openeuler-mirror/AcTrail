<template>
  <fieldset
    class="config-item"
    :class="{
      root: depth === 0,
      'is-object': valueType === 'object',
      'is-array': valueType === 'array',
      'is-readonly': readOnly,
      'is-compact': compact,
    }"
    :disabled="readOnly"
  >
    <div v-if="depth > 0 && !compact" class="config-meta">
      <div class="config-title-row">
        <strong>{{ displayLabel }}</strong>
        <span v-if="requiredField" class="config-required">Required</span>
        <ConfigHint v-if="hintText" :label="displayLabel" :text="hintText" />
        <span v-if="readOnly" class="config-access readonly">
          <LockKeyhole :size="12" aria-hidden="true" />
          Read only
        </span>
      </div>
    </div>

    <div class="config-control">
      <div v-if="valueType === 'object'" class="config-object">
        <PluginConfigItem
          v-for="entry in objectEntries"
          :key="entry.key"
          :name="entry.key"
          :schema="entry.schema"
          :model-value="entry.value"
          :editable="!readOnly"
          :required-field="entry.required"
          :depth="depth + 1"
          @update:model-value="updateObjectValue(entry.key, $event)"
        />
      </div>

      <label v-else-if="valueType === 'boolean'" class="config-boolean">
        <input
          type="checkbox"
          :checked="Boolean(modelValue)"
          @change="$emit('update:modelValue', $event.target.checked)"
        />
        <span class="config-boolean-copy">
          <strong>{{ modelValue ? 'Enabled' : 'Disabled' }}</strong>
          <small>{{ readOnly ? 'Locked by schema' : 'Click to change' }}</small>
        </span>
      </label>

      <div v-else-if="valueType === 'array'" class="config-array">
        <div v-for="(item, index) in arrayValue" :key="index" class="config-array-row">
          <div class="config-array-row-heading">
            <strong>Entry {{ index + 1 }}</strong>
            <button
              v-if="!readOnly"
              class="config-remove"
              type="button"
              :aria-label="`Remove item ${index + 1} from ${displayLabel}`"
              @click="removeArrayValue(index)"
            >
              <Trash2 :size="14" aria-hidden="true" />
              Remove
            </button>
          </div>
          <PluginConfigItem
            :name="`${name}-${index + 1}`"
            :schema="schema.items ?? {}"
            :model-value="item"
            :editable="!readOnly"
            :depth="depth + 1"
            compact
            @update:model-value="updateArrayValue(index, $event)"
          />
        </div>
        <button v-if="!readOnly" class="config-add" type="button" @click="addArrayValue">
          <Plus :size="14" aria-hidden="true" />
          Add entry
        </button>
        <p v-if="arrayValue.length === 0" class="config-empty">No items configured.</p>
      </div>

      <select
        v-else-if="choiceOptions.length"
        class="config-input config-select"
        :value="selectedChoiceIndex"
        @change="selectChoice"
      >
        <option value="" disabled>{{ `Select ${displayLabel.toLowerCase()}` }}</option>
        <option
          v-for="(option, index) in choiceOptions"
          :key="option.key"
          :value="String(index)"
        >
          {{ option.label }}
        </option>
      </select>

      <input
        v-else-if="valueType === 'integer' || valueType === 'number'"
        class="config-input"
        type="number"
        :value="modelValue"
        :min="schema.minimum"
        :max="schema.maximum"
        :step="valueType === 'integer' ? 1 : 'any'"
        @input="$emit('update:modelValue', Number($event.target.value))"
      />

      <input
        v-else
        class="config-input"
        type="text"
        :value="modelValue ?? ''"
        :placeholder="schema.examples?.[0] ?? ''"
        @input="$emit('update:modelValue', $event.target.value)"
      />
    </div>
  </fieldset>
</template>

<script setup>
import { computed } from 'vue';
import { LockKeyhole, Plus, Trash2 } from '@lucide/vue';

import ConfigHint from './ConfigHint.vue';

const props = defineProps({
  name: { type: String, required: true },
  schema: { type: Object, default: () => ({}) },
  modelValue: { default: null },
  editable: { type: Boolean, default: true },
  requiredField: { type: Boolean, default: false },
  depth: { type: Number, default: 0 },
  compact: { type: Boolean, default: false },
});

const emit = defineEmits(['update:modelValue']);
const readOnly = computed(() => !props.editable || props.schema.readOnly === true);
const valueType = computed(() => props.schema.type ?? inferType(props.modelValue));
const displayLabel = computed(() => props.schema.title ?? humanize(props.name));
const arrayValue = computed(() => Array.isArray(props.modelValue) ? props.modelValue : []);
const choiceOptions = computed(() => schemaChoices(props.schema));
const selectedChoiceIndex = computed(() => {
  const current = JSON.stringify(props.modelValue);
  const index = choiceOptions.value.findIndex((option) => JSON.stringify(option.value) === current);
  return index < 0 ? '' : String(index);
});
const rangeLabel = computed(() => {
  if (props.schema.minimum == null && props.schema.maximum == null) return '';
  if (props.schema.minimum != null && props.schema.maximum != null) {
    return `Allowed range: ${props.schema.minimum}–${props.schema.maximum}`;
  }
  return props.schema.minimum != null
    ? `Minimum: ${props.schema.minimum}`
    : `Maximum: ${props.schema.maximum}`;
});
const hintText = computed(() => [props.schema.description, rangeLabel.value].filter(Boolean).join(' '));
const objectEntries = computed(() => {
  const properties = props.schema.properties ?? {};
  const value = isObject(props.modelValue) ? props.modelValue : {};
  return Object.keys(properties)
    .map((key, index) => ({
      key,
      index,
      schema: properties[key] ?? {},
      value: value[key],
      required: Array.isArray(props.schema.required) && props.schema.required.includes(key),
    }))
    .sort((left, right) => Number(left.schema.readOnly === true) - Number(right.schema.readOnly === true)
      || Number(right.required) - Number(left.required)
      || left.index - right.index);
});

function updateObjectValue(key, value) {
  emit('update:modelValue', { ...(isObject(props.modelValue) ? props.modelValue : {}), [key]: value });
}

function updateArrayValue(index, value) {
  const next = arrayValue.value.slice();
  next[index] = value;
  emit('update:modelValue', next);
}

function removeArrayValue(index) {
  const next = arrayValue.value.slice();
  next.splice(index, 1);
  emit('update:modelValue', next);
}

function addArrayValue() {
  const next = arrayValue.value.slice();
  next.push(defaultValue(props.schema.items ?? {}));
  emit('update:modelValue', next);
}

function selectChoice(event) {
  const index = Number(event.target.value);
  const option = choiceOptions.value[index];
  if (option) emit('update:modelValue', option.value);
}

function defaultValue(schema) {
  if (Object.prototype.hasOwnProperty.call(schema, 'default')) return schema.default;
  if (schema.type === 'object') {
    return Object.fromEntries(Object.entries(schema.properties ?? {})
      .filter(([, property]) => Object.prototype.hasOwnProperty.call(property, 'default'))
      .map(([key, property]) => [key, defaultValue(property)]));
  }
  if (schema.type === 'array') return [];
  if (schema.type === 'boolean') return false;
  if (schema.type === 'integer' || schema.type === 'number') return schema.minimum ?? 0;
  return '';
}

function schemaChoices(schema) {
  if (Array.isArray(schema.enum)) {
    return schema.enum.map((value) => ({ key: JSON.stringify(value), label: choiceLabel(value), value }));
  }
  if (Array.isArray(schema.oneOf)
    && schema.oneOf.every((option) => Object.prototype.hasOwnProperty.call(option, 'const'))) {
    return schema.oneOf.map((option) => ({
      key: JSON.stringify(option.const),
      label: option.title ?? choiceLabel(option.const),
      value: option.const,
    }));
  }
  return [];
}

function choiceLabel(value) {
  if (value === null) return 'None';
  if (typeof value === 'boolean') return value ? 'Yes' : 'No';
  if (typeof value === 'string') return humanize(value);
  return String(value);
}

function inferType(value) {
  if (Array.isArray(value)) return 'array';
  if (isObject(value)) return 'object';
  if (typeof value === 'boolean') return 'boolean';
  if (typeof value === 'number') return Number.isInteger(value) ? 'integer' : 'number';
  return 'string';
}

function isObject(value) {
  return value != null && typeof value === 'object' && !Array.isArray(value);
}

function humanize(value) {
  return value.replaceAll('_', ' ').replace(/\b\w/g, (letter) => letter.toUpperCase());
}
</script>

<style scoped>
.config-item {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
  align-items: start;
  gap: var(--stats-space-xl);
  margin: 0;
  padding: var(--stats-space-lg) 0;
  border: 0;
  border-bottom: 1px solid var(--stats-border);
  background: transparent;
}

.config-item.root {
  display: block;
  padding: 0;
  border: 0;
  background: transparent;
}

.config-item.is-object:not(.root),
.config-item.is-array:not(.root) {
  grid-template-columns: minmax(0, 1fr);
  padding: var(--stats-space-xl);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface-soft);
}

.config-item.is-readonly:not(.root) {
  color: var(--stats-muted);
}

.config-item.is-compact:not(.root) {
  display: block;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
}

.config-meta {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-xs);
}

.config-title-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--stats-space-sm);
}

.config-title-row strong {
  color: var(--stats-text);
  font-size: var(--stats-font-md);
  font-weight: var(--stats-weight-medium);
}

.config-access {
  min-height: var(--stats-control-height-sm);
  display: inline-flex;
  align-items: center;
  gap: var(--stats-space-2xs);
  padding: 0 var(--stats-space-sm);
  border: 1px solid var(--stats-border-strong);
  border-radius: 100vmax;
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
}

.config-access.readonly {
  border-color: var(--stats-border-strong);
  background: var(--stats-surface-bar);
  color: var(--stats-muted);
}

.config-required {
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
  font-weight: var(--stats-weight-medium);
  text-transform: uppercase;
}

.config-empty {
  margin: 0;
  color: var(--stats-muted);
  font-size: var(--stats-font-sm);
  line-height: 1.45;
}

.config-control,
.config-object,
.config-array {
  min-width: 0;
}

.config-object,
.config-array {
  display: grid;
  gap: var(--stats-space-md);
}

.config-boolean {
  min-height: var(--stats-control-height-lg);
  display: flex;
  align-items: center;
  gap: var(--stats-space-md);
  padding: var(--stats-space-md) var(--stats-space-lg);
  border: 1px solid var(--stats-border-strong);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface-strong);
  color: var(--stats-text);
  cursor: pointer;
}

.config-boolean input {
  width: 1.125rem;
  height: 1.125rem;
  flex: 0 0 auto;
  accent-color: var(--stats-accent);
}

.config-boolean-copy {
  display: grid;
  gap: var(--stats-space-2xs);
}

.config-boolean-copy strong {
  font-size: var(--stats-font-md);
  font-weight: var(--stats-weight-medium);
}

.config-boolean-copy small {
  color: var(--stats-muted);
  font-size: var(--stats-font-xs);
}

.config-item.is-readonly .config-boolean {
  border-style: dashed;
  background: transparent;
  color: var(--stats-muted);
  cursor: not-allowed;
}

.config-input {
  min-width: 0;
  width: 100%;
  min-height: var(--stats-control-height-lg);
  padding: 0 var(--stats-space-md);
  border: 1px solid color-mix(in srgb, var(--stats-accent) 42%, var(--stats-border-strong));
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
  color: var(--stats-text);
  font: inherit;
}

.config-select {
  cursor: pointer;
}

.config-input:focus {
  border-color: var(--stats-accent);
  outline: 0.125rem solid color-mix(in srgb, var(--stats-accent) 18%, transparent);
  outline-offset: 0.0625rem;
}

.config-item.is-readonly .config-input {
  border-style: dashed;
  background: transparent;
  color: var(--stats-muted);
}

.config-array-row {
  min-width: 0;
  display: grid;
  gap: var(--stats-space-lg);
  padding: var(--stats-space-lg);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-md);
  background: var(--stats-surface);
}

.config-array-row-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--stats-space-md);
  padding-bottom: var(--stats-space-md);
  border-bottom: 1px solid var(--stats-border);
}

.config-array-row-heading strong {
  color: var(--stats-text);
  font-size: var(--stats-font-sm);
  font-weight: var(--stats-weight-medium);
}

.config-remove,
.config-add {
  min-height: var(--stats-control-height-md);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--stats-space-xs);
  padding: 0 var(--stats-space-md);
  border: 1px solid var(--stats-border);
  border-radius: var(--stats-radius-sm);
  background: var(--stats-surface);
  color: var(--stats-muted);
  cursor: pointer;
  font: inherit;
  font-size: var(--stats-font-sm);
}

:global(.stats-theme-arc-glass) .config-item.is-object:not(.root),
:global(.stats-theme-arc-glass) .config-item.is-array:not(.root) {
  border-color: rgb(15 15 20 / 9%);
  background: rgb(139 92 246 / 3.5%);
  box-shadow: inset 0 0 0 1px rgb(255 255 255 / 52%);
}

:global(.stats-theme-arc-glass) .config-array-row {
  border-color: rgb(139 92 246 / 20%);
  background: rgb(255 255 255 / 84%);
  box-shadow:
    0 0.35rem 1rem rgb(15 15 20 / 8%),
    inset 0 1px 0 rgb(255 255 255 / 82%);
  transition:
    border-color 140ms ease,
    box-shadow 140ms ease,
    transform 140ms ease;
}

:global(.stats-theme-arc-glass) .config-array-row:focus-within {
  border-color: rgb(139 92 246 / 48%);
  box-shadow:
    0 0 0 0.15rem rgb(139 92 246 / 12%),
    0 0.55rem 1.35rem rgb(15 15 20 / 11%),
    inset 0 1px 0 rgb(255 255 255 / 86%);
}

:global(.stats-theme-arc-glass) .config-array-row-heading {
  border-bottom-color: rgb(15 15 20 / 10%);
}

.config-remove:hover,
.config-add:hover {
  border-color: var(--stats-accent);
  color: var(--stats-accent);
}

.config-remove:focus-visible,
.config-add:focus-visible,
.config-boolean:has(input:focus-visible),
.config-input:focus-visible {
  outline: 2px solid var(--stats-accent);
  outline-offset: var(--stats-space-xs);
}

.config-remove {
  justify-self: end;
  border-color: color-mix(in srgb, var(--stats-danger) 28%, var(--stats-border));
}

.config-remove:hover {
  border-color: var(--stats-danger);
  background: color-mix(in srgb, var(--stats-danger) 7%, var(--stats-surface));
  color: var(--stats-danger);
}

.config-add {
  justify-self: start;
  border-style: dashed;
  background: transparent;
  color: var(--stats-accent);
}

@container plugin-config (max-width: 52rem) {
  .config-item {
    grid-template-columns: minmax(0, 1fr);
    gap: var(--stats-space-md);
  }
}

@container plugin-config (max-width: 36rem) {
  .config-remove {
    justify-self: end;
  }
}
</style>
