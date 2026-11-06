<template>
  <div class="config-field" :class="{ 'is-modified': modified, 'is-wide': field.wide }" :data-config-key="field.key">
    <label v-if="field.type !== 'tags'" :id="`${field.key}-label`" :for="field.key" class="config-field-label">{{ t(`config.${field.label}`) }}</label>
    <span v-else :id="`${field.key}-label`" class="config-field-label">{{ t(`config.${field.label}`) }}</span>
    <code>{{ field.key }}</code>

    <label v-if="field.type === 'boolean'" class="config-switch-control">
      <input :id="field.key" :name="field.key" type="checkbox" role="switch" :checked="modelValue" @change="update($event.target.checked)" />
      <span class="config-switch-track" aria-hidden="true"></span>
      <span>{{ modelValue ? t('config.enabled') : t('config.disabled') }}</span>
    </label>

    <fieldset v-else-if="field.type === 'tags'" :name="field.key" class="config-tags" :aria-labelledby="`${field.key}-label`">
      <legend class="config-sr-only">{{ t('config.protectedTags') }}</legend>
      <div v-for="(tag, index) in modelValue" :key="index" class="config-tag-row">
        <input :value="tag" type="text" :aria-label="t('config.tagIndex', { index: index + 1 })" @input="update(modelValue.map((value, i) => i === index ? $event.target.value : value))" />
        <button type="button" :aria-label="t('config.removeTag', { index: index + 1 })" @click="update(modelValue.filter((_, i) => i !== index))"><X :size="14" aria-hidden="true" /></button>
      </div>
      <span v-if="!modelValue.length" class="config-empty-tags">{{ t('config.noTags') }}</span>
      <button class="config-add-tag" type="button" @click="update([...modelValue, ''])"><Plus :size="14" aria-hidden="true" />{{ t('config.addTag') }}</button>
    </fieldset>

    <template v-else>
      <label v-if="isAlert" class="config-alert-toggle">
        <input type="checkbox" :checked="modelValue !== 'disabled'" :aria-label="t('config.enableAlert', { label: t(`config.${field.label}`) })" @change="update($event.target.checked ? '' : 'disabled')" />
        {{ modelValue === 'disabled' ? t('config.alertDisabled') : t('config.alertEnabled') }}
      </label>
      <span class="config-control" :class="{ 'has-unit': field.unit }">
        <select v-if="field.options" :id="field.key" :name="field.key" :value="modelValue" @change="update($event.target.value)">
          <option v-for="option in field.options" :key="option" :value="option">{{ option }}</option>
        </select>
        <input
          v-else
          :id="field.key"
          :name="field.key"
          :value="isAlert && modelValue === 'disabled' ? '' : modelValue"
          :type="field.type === 'integer' ? 'number' : 'text'"
          :inputmode="field.type === 'percent' ? 'decimal' : isAlert ? 'numeric' : undefined"
          :pattern="isAlert ? field.type === 'percent' ? '(?:[0-9]+(?:\\.[0-9]{0,3})?|\\.[0-9]{1,3})' : '[0-9]+' : undefined"
          :step="field.type === 'integer' ? '1' : undefined"
          :required="field.required || isAlert || field.type === 'integer'"
          :disabled="isAlert && modelValue === 'disabled'"
          :placeholder="isAlert ? t('config.enterThreshold') : undefined"
          :aria-describedby="field.hint || field.unit ? `${field.key}-hint` : undefined"
          spellcheck="false"
          autocapitalize="none"
          @input="update($event.target.value)"
        />
        <ChevronDown v-if="field.options" :size="14" aria-hidden="true" />
        <span v-if="field.unit" class="config-control-unit" aria-hidden="true">{{ t(`config.units.${field.unit}`) }}</span>
      </span>
    </template>
    <small v-if="field.hint || field.unit" :id="`${field.key}-hint`">
      {{ field.hint ? t(`config.${field.hint}`) : t('config.unitHint', { unit: t(`config.units.${field.unit}`) }) }}
    </small>
  </div>
</template>

<script setup>
import { ChevronDown, Plus, X } from '@lucide/vue';
import { computed } from 'vue';
import { useModuleLocale } from '../../locale';
import strings from '../locale';

const props = defineProps({ field: { type: Object, required: true }, modelValue: { required: true }, modified: Boolean });
const emit = defineEmits(['update:modelValue']);
const { t } = useModuleLocale(strings);
const isAlert = computed(() => ['percent', 'threshold'].includes(props.field.type));
const update = value => emit('update:modelValue', value);

</script>

<style scoped>
.config-field { display: flex; flex-direction: column; min-width: 0; gap: 6px; }
.config-field.is-wide { grid-column: 1 / -1; }
.config-field-label { display: flex; align-items: center; gap: 7px; font-size: 13px; font-weight: 500; }
.is-modified .config-field-label::after { content: ''; width: 5px; height: 5px; flex-shrink: 0; border-radius: 50%; background: var(--ui-accent-strong); }
.config-field > code { color: var(--ui-muted); font: 10px/1.5 var(--ui-mono); overflow-wrap: anywhere; }
.config-field > small { color: var(--ui-muted); font-size: 11px; line-height: 1.6; }
.config-control { position: relative; display: flex; min-width: 0; align-items: center; margin-top: 3px; }
.config-control input, .config-control select, .config-tag-row input {
  width: 100%; min-width: 0; height: 40px; padding: 0 12px; border: 1px solid var(--ui-border-strong); border-radius: 6px;
  background: var(--ui-bg-base); color: var(--ui-text); font: 12px var(--ui-mono);
  transition: border-color var(--ui-duration-fast), box-shadow var(--ui-duration-fast);
}
.config-control input:hover, .config-control select:hover, .config-tag-row input:hover { border-color: var(--ui-border-hover); }
.config-control input:focus, .config-control select:focus, .config-tag-row input:focus { outline: none; border-color: var(--ui-accent-strong); box-shadow: 0 0 0 3px var(--ui-accent-muted); }
.config-control select { appearance: none; padding-right: 34px; cursor: pointer; }
.config-control input:disabled { background: var(--ui-bg-alt); color: var(--ui-muted); opacity: .6; cursor: not-allowed; }
.config-control.has-unit input { padding-right: 70px; }
.config-control input[type='number'] { appearance: textfield; }
.config-control input::-webkit-inner-spin-button, .config-control input::-webkit-outer-spin-button { appearance: none; margin: 0; }
.config-control svg, .config-control-unit { position: absolute; right: 12px; color: var(--ui-muted); pointer-events: none; }
.config-control-unit { font: 11px var(--ui-mono); }
.config-switch-control { position: relative; display: inline-flex; align-items: center; gap: 10px; min-height: 40px; align-self: flex-start; cursor: pointer; font-size: 12px; color: var(--ui-muted); }
.config-switch-control input { position: absolute; inset: 0; z-index: 1; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: pointer; }
.config-switch-track { position: relative; width: 30px; height: 18px; border: 1px solid var(--ui-border-strong); border-radius: 10px; background: var(--ui-bg-alt); transition: background var(--ui-duration-base); }
.config-switch-track::after { content: ''; position: absolute; left: 3px; top: 3px; width: 10px; height: 10px; border-radius: 50%; background: var(--ui-muted); transition: transform var(--ui-duration-base), background var(--ui-duration-base); }
input:checked + .config-switch-track { background: var(--ui-text); border-color: var(--ui-text); }
input:checked + .config-switch-track::after { transform: translateX(12px); background: var(--ui-bg-base); }
input:focus-visible + .config-switch-track { outline: 2px solid var(--ui-focus-ring-color); outline-offset: 3px; }
.config-alert-toggle { display: flex; align-items: center; gap: 7px; margin-top: 5px; color: var(--ui-muted); font-size: 11px; cursor: pointer; }
.config-alert-toggle input { accent-color: var(--ui-accent-strong); }
.config-tags { position: relative; display: flex; flex-direction: column; gap: 8px; min-width: 0; margin: 4px 0 0; padding: 0; border: 0; }
.config-tag-row { display: flex; align-items: center; gap: 8px; }
.config-tag-row button, .config-add-tag { display: inline-flex; align-items: center; justify-content: center; gap: 6px; padding: 8px; border: 0; border-radius: 5px; background: transparent; color: var(--ui-muted); font: inherit; font-size: 12px; cursor: pointer; }
.config-tag-row button:hover, .config-add-tag:hover { background: var(--ui-bg-hover); color: var(--ui-text); }
.config-add-tag { align-self: flex-start; }
.config-empty-tags { padding: 8px 0; font-size: 12px; color: var(--ui-muted); }
.config-sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
</style>
