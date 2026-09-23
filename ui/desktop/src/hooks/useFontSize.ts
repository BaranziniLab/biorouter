import { useEffect, useSyncExternalStore } from 'react';

export const FONT_SIZES = ['standard', 'large', 'larger'] as const;
export type FontSize = (typeof FONT_SIZES)[number];
export const FONT_SIZE_SCALE: Record<FontSize, number> = {
  standard: 1,
  large: 1.07,
  larger: 1.15,
};
export const FONT_SIZE_STORAGE_KEY = 'app_font_size';
const CHANGE_EVENT = 'biorouter-font-size-changed';

export function loadFontSize(): FontSize {
  const stored = localStorage.getItem(FONT_SIZE_STORAGE_KEY);
  return FONT_SIZES.includes(stored as FontSize) ? (stored as FontSize) : 'standard';
}

export function applyFontSize(size: FontSize): void {
  document.documentElement.dataset.fontSize = size;
  document.documentElement.style.setProperty('--app-font-scale', String(FONT_SIZE_SCALE[size]));
}

export function setFontSize(size: FontSize): void {
  localStorage.setItem(FONT_SIZE_STORAGE_KEY, size);
  applyFontSize(size);
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

function subscribe(onChange: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === FONT_SIZE_STORAGE_KEY || event.key === null) onChange();
  };
  window.addEventListener('storage', onStorage);
  window.addEventListener(CHANGE_EVENT, onChange);
  return () => {
    window.removeEventListener('storage', onStorage);
    window.removeEventListener(CHANGE_EVENT, onChange);
  };
}

export function useFontSize() {
  const fontSize = useSyncExternalStore(subscribe, loadFontSize);
  useEffect(() => applyFontSize(fontSize), [fontSize]);
  return { fontSize, setFontSize, fontScale: FONT_SIZE_SCALE[fontSize] };
}
