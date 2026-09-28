<template>
  <section class="policy-scope-editor">
    <div class="policy-scope-heading">
      <div>
        <span>{{ title }}</span>
        <small>{{ description }}</small>
      </div>
      <strong>{{ t('policy.required') }}</strong>
    </div>

    <div class="policy-scope-list">
      <div v-for="(scope, index) in modelValue" :key="scope.key" class="policy-scope-row">
        <label>
          <span>{{ pathLabel || t('policy.path') }}</span>
          <input
            :value="scope.path_scope"
            type="text"
            :placeholder="placeholder"
            autocomplete="off"
            :disabled="busy"
            @input="updatePath(index, $event.target.value)"
            @blur="$emit('blur')"
          />
          <small>{{ pathHint }}</small>
        </label>
        <fieldset>
          <legend>{{ t('policy.ruleTypes') }}</legend>
          <label v-for="decision in decisionOptions" :key="decision.value">
            <input
              type="checkbox"
              :value="decision.value"
              :checked="scope.decisions.includes(decision.value)"
              :disabled="busy"
              @change="updateDecision(index, decision.value, $event.target.checked)"
            />
            <span>{{ decision.label }}</span>
          </label>
        </fieldset>
        <button
          v-if="modelValue.length > 1"
          class="policy-scope-remove"
          type="button"
          :disabled="busy"
          @click="removeScope(index)"
        >
          <Trash2 :size="15" aria-hidden="true" />
          {{ t('policy.removeScope') }}
        </button>
      </div>
    </div>
    <button class="policy-scope-add" type="button" :disabled="busy" @click="addScope">
      <Plus :size="15" aria-hidden="true" />
      {{ addLabel || t('policy.addAnotherPath') }}
    </button>
  </section>
</template>

<script setup>
import { Plus, Trash2 } from '@lucide/vue';

import { useModuleLocale } from '../../locale';
import strings from './locale';

const { t } = useModuleLocale(strings);

const props = defineProps({
  modelValue: { type: Array, required: true },
  title: { type: String, required: true },
  description: { type: String, required: true },
  pathLabel: { type: String, default: '' },
  placeholder: { type: String, required: true },
  pathHint: { type: String, required: true },
  busy: { type: Boolean, default: false },
  addLabel: { type: String, default: '' },
});

const emit = defineEmits(['update:modelValue', 'blur']);
const decisionOptions = [
  { value: 'allow', label: t('policy.allow') },
  { value: 'deny', label: t('policy.deny') },
  { value: 'gray', label: t('policy.askPlugin') },
];
let nextKey = 0;

function newScope() {
  nextKey += 1;
  return {
    key: `policy-scope-${nextKey}`,
    path_scope: '',
    decisions: decisionOptions.map((decision) => decision.value),
  };
}

function updateScope(index, update) {
  const scopes = props.modelValue.map((scope, scopeIndex) => (
    scopeIndex === index ? { ...scope, ...update } : scope
  ));
  emit('update:modelValue', scopes);
}

function updatePath(index, pathScope) {
  updateScope(index, { path_scope: pathScope });
}

function updateDecision(index, decision, checked) {
  const current = props.modelValue[index].decisions;
  const decisions = checked
    ? [...new Set([...current, decision])]
    : current.filter((value) => value !== decision);
  updateScope(index, { decisions });
}

function addScope() {
  emit('update:modelValue', [...props.modelValue, newScope()]);
}

function removeScope(index) {
  emit('update:modelValue', props.modelValue.filter((_, scopeIndex) => scopeIndex !== index));
}
</script>

<style scoped>
.policy-scope-editor {
  min-width: 0;
  display: grid;
  gap: var(--ui-space-lg);
  padding: var(--ui-space-lg);
  border: 1px solid var(--ui-accent-soft);
  border-radius: var(--ui-radius-md);
  background: var(--ui-accent-faint);
}

.policy-scope-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-lg);
}

.policy-scope-heading > div,
.policy-scope-row > label {
  display: grid;
  gap: var(--ui-space-xs);
}

.policy-scope-heading span,
.policy-scope-row > label > span,
.policy-scope-row legend {
  color: var(--ui-text);
  font-size: var(--ui-font-md);
  font-weight: var(--ui-weight-medium);
}

.policy-scope-heading small,
.policy-scope-row small {
  color: var(--ui-muted);
  font-size: var(--ui-font-sm);
}

.policy-scope-heading strong {
  padding: var(--ui-space-xs) var(--ui-space-sm);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
  color: var(--ui-accent);
  font-size: var(--ui-font-xs);
  font-weight: var(--ui-weight-medium);
  text-transform: uppercase;
}

.policy-scope-list {
  display: grid;
  gap: var(--ui-space-md);
}

.policy-scope-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: var(--ui-space-md);
  padding: var(--ui-space-md);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
}

.policy-scope-row input[type="text"] {
  width: 100%;
  padding: var(--ui-space-md);
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface);
  color: var(--ui-text);
  font: inherit;
}

.policy-scope-row fieldset {
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ui-space-md);
  margin: 0;
  padding: 0;
  border: 0;
}

.policy-scope-row fieldset label,
.policy-scope-add,
.policy-scope-remove {
  display: inline-flex;
  align-items: center;
  gap: var(--ui-space-xs);
}

.policy-scope-row fieldset label {
  min-height: var(--ui-control-height-sm);
  padding: 0 var(--ui-space-sm);
  border: 1px solid var(--ui-border);
  border-radius: var(--ui-radius-sm);
  background: var(--ui-surface-soft);
  color: var(--ui-text);
  font-size: var(--ui-font-sm);
}

.policy-scope-row legend {
  margin-bottom: var(--ui-space-xs);
}

.policy-scope-add,
.policy-scope-remove {
  min-height: var(--ui-control-height-md);
  border: 1px solid var(--ui-border-strong);
  border-radius: var(--ui-radius-sm);
  background: transparent;
}

.policy-scope-remove {
  grid-column: 1 / -1;
  justify-self: end;
  min-height: var(--ui-control-height-sm);
  padding: 0 var(--ui-space-md);
  color: var(--ui-danger);
}

.policy-scope-add {
  justify-self: start;
  padding: 0 var(--ui-space-md);
}

button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}

@media (max-width: 47.5rem) {
  .policy-scope-row {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
