import './contract.css';
import './light/tokens.css';

const paletteModules = import.meta.glob('./*/tokens.css', { query: '?raw', import: 'default' });
const SYSTEM_DARK_QUERY = '(prefers-color-scheme: dark)';
const PALETTE_ATTRIBUTE = 'palette';
const MODE_ATTRIBUTE = 'themeMode';

/**
 * Theme modes offered to the operator. `system` follows the platform colour
 * scheme and resolves to the pure white or dark palette, matching the
 * management console's behaviour. The remaining modes pin a palette.
 */
export const THEME_MODES = Object.freeze([
  { id: 'system', palette: null },
  { id: 'light', palette: 'light' },
  { id: 'white', palette: 'white' },
  { id: 'dark', palette: 'dark' },
]);

export const DEFAULT_THEME_MODE = 'system';

const BUNDLED_PALETTES = new Set(['light']);
const loadedPalettes = new Set(BUNDLED_PALETTES);
const paletteLoads = new Map();
const systemQuery = window.matchMedia(SYSTEM_DARK_QUERY);
let activeMode = DEFAULT_THEME_MODE;
let systemListenerBound = false;

export function isThemeMode(mode) {
  return THEME_MODES.some((candidate) => candidate.id === mode);
}

export async function applyThemeMode(mode) {
  const normalizedMode = isThemeMode(mode) ? mode : DEFAULT_THEME_MODE;
  const palette = resolvePalette(normalizedMode);
  await ensurePalette(palette);
  activeMode = normalizedMode;
  const root = document.documentElement;
  root.dataset[PALETTE_ATTRIBUTE] = palette;
  root.dataset[MODE_ATTRIBUTE] = normalizedMode;
  bindSystemListener();
}

export function resolvePalette(mode) {
  const entry = THEME_MODES.find((candidate) => candidate.id === mode);
  if (entry?.palette) {
    return entry.palette;
  }
  return systemQuery.matches ? 'dark' : 'white';
}

async function ensurePalette(palette) {
  if (loadedPalettes.has(palette)) {
    return;
  }
  const pending = paletteLoads.get(palette);
  if (pending) {
    await pending;
    return;
  }
  const load = loadPalette(palette)
    .then((cssText) => {
      installPaletteStyle(palette, cssText);
      loadedPalettes.add(palette);
    })
    .finally(() => {
      paletteLoads.delete(palette);
    });
  paletteLoads.set(palette, load);
  await load;
}

async function loadPalette(palette) {
  const loader = paletteModules[`./${palette}/tokens.css`];
  if (!loader) {
    throw new Error(`missing theme tokens for ${palette}`);
  }
  return await loader();
}

function bindSystemListener() {
  if (systemListenerBound) {
    return;
  }
  systemListenerBound = true;
  systemQuery.addEventListener('change', () => {
    if (activeMode !== 'system') {
      return;
    }
    const palette = resolvePalette(activeMode);
    void ensurePalette(palette).then(() => {
      document.documentElement.dataset[PALETTE_ATTRIBUTE] = palette;
    });
  });
}

function installPaletteStyle(palette, cssText) {
  const elementId = `actrail-palette-${palette}`;
  const existing = document.getElementById(elementId);
  if (existing) {
    existing.textContent = cssText;
    return;
  }
  const style = document.createElement('style');
  style.id = elementId;
  style.dataset.actrailPalette = palette;
  style.textContent = cssText;
  document.head.appendChild(style);
}
