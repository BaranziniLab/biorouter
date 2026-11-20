import { useEffect, useSyncExternalStore } from 'react';

/**
 * Settings > App > Appearance > Text size: Small, Standard and Large, with Standard, the middle
 * one, the default.
 *
 * Until 1.94.5 the same three sizes were called Standard (1, the default), Large and Larger. The
 * owner renamed them one step down (2026-11-19), so the size most people read at is the middle of
 * the control and anyone can go a step smaller or larger. Nobody's text changes with the rename:
 * a saved old Standard reads as Small, Large as Standard and Larger as Large. Only a person who
 * never chose moves, from the old Standard to the new one.
 *
 * `index.html` applies the same rules before the bundle loads, so keep the two in step.
 */
export const FONT_SIZES = ['small', 'standard', 'large'] as const;
export type FontSize = (typeof FONT_SIZES)[number];
export const DEFAULT_FONT_SIZE: FontSize = 'standard';
export const FONT_SIZE_SCALE: Record<FontSize, number> = {
  small: 1,
  standard: 1.07,
  large: 1.15,
};

/**
 * Per-viewer convenience, so `localStorage` is its home. A new key rather than new values under
 * the old one, because two of the old names (`standard`, `large`) are also new names for other
 * sizes: under one key, a stored `standard` could not say which of the two it meant.
 */
export const FONT_SIZE_STORAGE_KEY = 'biorouter:text-size';

/**
 * Where 1.94.5 and earlier kept the choice, in the old names. Only those builds write it, and
 * this one deletes it once read (`migrateLegacyFontSize` at startup, and every choice), so a
 * value here is always the newest choice: a person who went back to an older build and chose a
 * size there gets that size on updating again.
 */
export const LEGACY_FONT_SIZE_STORAGE_KEY = 'app_font_size';

const CHANGE_EVENT = 'biorouter-font-size-changed';

type FontSizeStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function defaultStorage(): FontSizeStorage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    // Reading `localStorage` itself throws when site data is blocked.
    return null;
  }
}

function isFontSize(value: unknown): value is FontSize {
  return (FONT_SIZES as readonly unknown[]).includes(value);
}

/** An old name in the new vocabulary, or null for anything no build ever stored. */
export function fontSizeFromLegacy(value: string | null): FontSize | null {
  switch (value) {
    case 'standard':
      return 'small';
    case 'large':
      return 'standard';
    case 'larger':
      return 'large';
    default:
      return null;
  }
}

/**
 * The size to show. Total: an unknown or unreadable value resolves to the default, the way the
 * sidebar width is clamped on read.
 */
export function loadFontSize(storage: FontSizeStorage | null = defaultStorage()): FontSize {
  if (!storage) return DEFAULT_FONT_SIZE;
  try {
    const legacy = fontSizeFromLegacy(storage.getItem(LEGACY_FONT_SIZE_STORAGE_KEY));
    if (legacy) return legacy;
    const stored = storage.getItem(FONT_SIZE_STORAGE_KEY);
    return isFontSize(stored) ? stored : DEFAULT_FONT_SIZE;
  } catch {
    return DEFAULT_FONT_SIZE;
  }
}

/**
 * Moves a choice saved under the old key and names to the new key, once. The renderer calls it at
 * every start; with no old key it does nothing, so a fresh install stays unset and follows the
 * default. The new key is written before the old one is removed, so a start interrupted between
 * the two still reads the same size.
 */
export function migrateLegacyFontSize(storage: FontSizeStorage | null = defaultStorage()): void {
  if (!storage) return;
  try {
    const legacy = storage.getItem(LEGACY_FONT_SIZE_STORAGE_KEY);
    if (legacy === null) return;
    const size = fontSizeFromLegacy(legacy);
    if (size) storage.setItem(FONT_SIZE_STORAGE_KEY, size);
    storage.removeItem(LEGACY_FONT_SIZE_STORAGE_KEY);
  } catch {
    // Storage is blocked; the read above still answers from whatever it holds.
  }
}

export function applyFontSize(size: FontSize): void {
  document.documentElement.dataset.fontSize = size;
  document.documentElement.style.setProperty('--app-font-scale', String(FONT_SIZE_SCALE[size]));
}

/**
 * A choice storage would not keep (site data blocked), held for this window until it reloads, so
 * the choice still applies and the control still shows it. Null once a choice is saved.
 */
let unsavedFontSize: FontSize | null = null;

export function setFontSize(size: FontSize): void {
  unsavedFontSize = size;
  const storage = defaultStorage();
  try {
    if (storage) {
      storage.setItem(FONT_SIZE_STORAGE_KEY, size);
      storage.removeItem(LEGACY_FONT_SIZE_STORAGE_KEY);
      unsavedFontSize = null;
    }
  } catch {
    // Storage is blocked: the size applies to this window and is not remembered.
  }
  applyFontSize(size);
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

function subscribe(onChange: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (
      event.key === FONT_SIZE_STORAGE_KEY ||
      event.key === LEGACY_FONT_SIZE_STORAGE_KEY ||
      event.key === null
    ) {
      // Another window could write, so storage holds the choice again.
      unsavedFontSize = null;
      onChange();
    }
  };
  window.addEventListener('storage', onStorage);
  window.addEventListener(CHANGE_EVENT, onChange);
  return () => {
    window.removeEventListener('storage', onStorage);
    window.removeEventListener(CHANGE_EVENT, onChange);
  };
}

const readFontSize = () => unsavedFontSize ?? loadFontSize();

export function useFontSize() {
  const fontSize = useSyncExternalStore(subscribe, readFontSize);
  useEffect(() => applyFontSize(fontSize), [fontSize]);
  return { fontSize, setFontSize, fontScale: FONT_SIZE_SCALE[fontSize] };
}
