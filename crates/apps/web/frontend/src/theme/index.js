import './contract.css';

const themeModules = import.meta.glob('./*/manifest.js', { eager: true });
const tokenModules = import.meta.glob('./*/tokens.css', { query: '?raw', import: 'default' });
const componentModules = import.meta.glob('./*/components/*.css', {
  query: '?raw',
  import: 'default',
});
const themeLabelCollator = new Intl.Collator('en-US', { sensitivity: 'base' });

export const THEMES = Object.freeze(
  Object.values(themeModules)
    .map((module) => module.default)
    .filter((theme) => theme?.id && theme?.label)
    .sort(
      (left, right) =>
        themeLabelCollator.compare(left.label, right.label) || left.id.localeCompare(right.id),
    ),
);

export const DEFAULT_THEME_ID = THEMES[0]?.id ?? 'granola';

const loadedThemes = new Set();
const loadingThemes = new Map();

export async function loadTheme(themeId) {
  const normalizedThemeId = normalizeThemeId(themeId);
  if (loadedThemes.has(normalizedThemeId)) {
    return;
  }
  const pending = loadingThemes.get(normalizedThemeId);
  if (pending) {
    await pending;
    return;
  }

  const tokenLoader = tokenModules[`./${normalizedThemeId}/tokens.css`];
  if (!tokenLoader) {
    throw new Error(`missing theme tokens for ${normalizedThemeId}`);
  }
  const componentLoaders = Object.entries(componentModules)
    .filter(([path]) => path.startsWith(`./${normalizedThemeId}/components/`))
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([, loader]) => loader);

  const load = Promise.all([tokenLoader(), ...componentLoaders.map((loader) => loader())])
    .then((cssParts) => {
      installThemeStyle(normalizedThemeId, cssParts.join('\n'));
      loadedThemes.add(normalizedThemeId);
    })
    .finally(() => {
      loadingThemes.delete(normalizedThemeId);
    });
  loadingThemes.set(normalizedThemeId, load);
  await load;
}

function normalizeThemeId(themeId) {
  const id = String(themeId ?? DEFAULT_THEME_ID);
  if (THEMES.some((theme) => theme.id === id)) {
    return id;
  }
  throw new Error(`unknown theme ${id}`);
}

function installThemeStyle(themeId, cssText) {
  const elementId = `actrail-theme-${themeId}`;
  const existing = document.getElementById(elementId);
  if (existing) {
    existing.textContent = cssText;
    return;
  }
  const style = document.createElement('style');
  style.id = elementId;
  style.dataset.actrailTheme = themeId;
  style.textContent = cssText;
  document.head.appendChild(style);
}
