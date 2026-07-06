import { computed, inject, provide } from 'vue';

import enUS from './en-US';
import zhCN from './zh-CN';

export const DEFAULT_LANGUAGE_ID = 'en-US';
export const LOCALE_KEY = Symbol('actrail-locale');
export const LANGUAGES = Object.freeze([
  { id: enUS.id, label: enUS.label, flag: enUS.flag, flagIcon: enUS.flagIcon, strings: enUS.strings },
  { id: zhCN.id, label: zhCN.label, flag: zhCN.flag, flagIcon: zhCN.flagIcon, strings: zhCN.strings },
]);

const dictionaries = Object.freeze(
  Object.fromEntries(LANGUAGES.map((language) => [language.id, language.strings])),
);

export function provideLocale(languageRef) {
  const currentLanguage = computed(() =>
    LANGUAGES.some((language) => language.id === languageRef.value)
      ? languageRef.value
      : DEFAULT_LANGUAGE_ID,
  );
  const context = {
    currentLanguage,
    languages: LANGUAGES,
    t: (key, params = {}) => translate(currentLanguage.value, key, params),
  };
  provide(LOCALE_KEY, context);
  return context;
}

export function useLocale() {
  const context = inject(LOCALE_KEY, null);
  if (context) {
    return context;
  }
  return {
    currentLanguage: computed(() => DEFAULT_LANGUAGE_ID),
    languages: LANGUAGES,
    t: (key, params = {}) => translate(DEFAULT_LANGUAGE_ID, key, params),
  };
}

function translate(languageId, key, params) {
  const value = valueAt(dictionaries[languageId], key) ?? valueAt(dictionaries[DEFAULT_LANGUAGE_ID], key);
  if (typeof value !== 'string') {
    return key;
  }
  return value.replace(/\{([A-Za-z0-9_]+)\}/g, (match, name) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : match,
  );
}

function valueAt(source, key) {
  if (!source) {
    return null;
  }
  return String(key)
    .split('.')
    .reduce((value, segment) => (value && Object.prototype.hasOwnProperty.call(value, segment) ? value[segment] : null), source);
}
