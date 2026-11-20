import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import FontSizeSelector from '../components/settings/app/FontSizeSelector';
import {
  DEFAULT_FONT_SIZE,
  FONT_SIZES,
  FONT_SIZE_SCALE,
  FONT_SIZE_STORAGE_KEY,
  LEGACY_FONT_SIZE_STORAGE_KEY,
  fontSizeFromLegacy,
  loadFontSize,
  migrateLegacyFontSize,
  useFontSize,
} from './useFontSize';

function Observer() {
  const { fontSize, fontScale } = useFontSize();
  return (
    <output>
      {fontSize}:{fontScale}
    </output>
  );
}

/** What each size measured before the rename (1.94.5 and earlier): Standard 1, Large 1.07, Larger 1.15. */
const SCALE_BEFORE_THE_RENAME = { standard: 1, large: 1.07, larger: 1.15 } as const;

/** Runs `index.html`'s pre-hydration script, as the window does before the bundle loads. */
function runBootScript() {
  const html = readFileSync(resolve(__dirname, '../../index.html'), 'utf8');
  const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
  expect(script).toBeDefined();
  new Function(script!)();
}

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.fontSize;
  document.documentElement.style.removeProperty('--app-font-scale');
});
afterEach(cleanup);

describe('app font size', () => {
  it('offers Small, Standard and Large, in that order, with the middle one the default', () => {
    expect(FONT_SIZES).toEqual(['small', 'standard', 'large']);
    expect(DEFAULT_FONT_SIZE).toBe('standard');
    render(<FontSizeSelector />);
    const radios = screen.getAllByRole('radio');
    expect(radios.map((radio) => radio.textContent)).toEqual(['Small', 'Standard', 'Large']);
    expect(radios[1]).toBeChecked();
    expect(radios[0]).not.toBeChecked();
    expect(radios[2]).not.toBeChecked();
  });

  it('keeps the three sizes that shipped, one name lower: Small is the old Standard, Standard the old Large, Large the old Larger', () => {
    expect(FONT_SIZE_SCALE).toEqual({
      small: SCALE_BEFORE_THE_RENAME.standard,
      standard: SCALE_BEFORE_THE_RENAME.large,
      large: SCALE_BEFORE_THE_RENAME.larger,
    });
  });

  it.each([null, 'unknown', 'Large', 'constructor', 'larger', '__proto__'])(
    'defaults to Standard (the old Large size) for %s',
    (stored) => {
      if (stored !== null) localStorage.setItem(FONT_SIZE_STORAGE_KEY, stored);
      render(<FontSizeSelector />);
      expect(screen.getByRole('radio', { name: 'Standard' })).toBeChecked();
      expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe('1.07');
      expect(document.documentElement.dataset.fontSize).toBe('standard');
    }
  );

  it.each([
    ['Small', 'small', '1'],
    ['Standard', 'standard', '1.07'],
    ['Large', 'large', '1.15'],
  ])('persists %s and restores it on remount', (label, value, scale) => {
    localStorage.setItem(FONT_SIZE_STORAGE_KEY, value === 'standard' ? 'large' : 'standard');
    const view = render(
      <>
        <FontSizeSelector />
        <Observer />
      </>
    );
    fireEvent.click(screen.getByRole('radio', { name: label }));
    expect(localStorage.getItem(FONT_SIZE_STORAGE_KEY)).toBe(value);
    expect(screen.getByRole('status')).toHaveTextContent(`${value}:${scale}`);
    expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe(scale);
    view.unmount();
    render(<FontSizeSelector />);
    expect(screen.getByRole('radio', { name: label })).toBeChecked();
  });

  it('converges when another window changes the preference or clears storage', () => {
    render(<FontSizeSelector />);
    act(() => {
      localStorage.setItem(FONT_SIZE_STORAGE_KEY, 'large');
      window.dispatchEvent(new StorageEvent('storage', { key: FONT_SIZE_STORAGE_KEY }));
    });
    expect(screen.getByRole('radio', { name: 'Large' })).toBeChecked();
    expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe('1.15');
    act(() => {
      localStorage.clear();
      window.dispatchEvent(new StorageEvent('storage', { key: null }));
    });
    expect(screen.getByRole('radio', { name: 'Standard' })).toBeChecked();
    expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe('1.07');
  });

  it('reads the default when storage cannot be read at all', () => {
    const storage = {
      getItem: () => {
        throw new Error('storage is blocked');
      },
      setItem: () => {},
      removeItem: () => {},
    };
    expect(loadFontSize(storage)).toBe('standard');
    expect(loadFontSize(null)).toBe('standard');
    expect(() => migrateLegacyFontSize(storage)).not.toThrow();
  });
});

describe('a size chosen before the rename (1.94.5 and earlier)', () => {
  it.each([
    ['standard', 'small'],
    ['large', 'standard'],
    ['larger', 'large'],
  ] as const)('reads the old %s as %s, at the same size it had', (old, renamed) => {
    expect(fontSizeFromLegacy(old)).toBe(renamed);
    expect(FONT_SIZE_SCALE[renamed]).toBe(SCALE_BEFORE_THE_RENAME[old]);
    localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, old);
    expect(loadFontSize()).toBe(renamed);
    render(<FontSizeSelector />);
    expect(
      screen.getByRole('radio', { name: renamed[0].toUpperCase() + renamed.slice(1) })
    ).toBeChecked();
    expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe(
      String(SCALE_BEFORE_THE_RENAME[old])
    );
  });

  it.each([null, 'small', 'Larger', 'constructor', '__proto__', 'hasOwnProperty', ''])(
    'is ignored when it holds %s, which no build ever wrote',
    (old) => {
      expect(fontSizeFromLegacy(old)).toBeNull();
      if (old !== null) localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, old);
      expect(loadFontSize()).toBe('standard');
      localStorage.setItem(FONT_SIZE_STORAGE_KEY, 'small');
      expect(loadFontSize()).toBe('small');
    }
  );

  it.each([
    ['standard', 'small'],
    ['large', 'standard'],
    ['larger', 'large'],
  ] as const)(
    'moves the old %s to the new key as %s once, and a second run changes nothing',
    (old, renamed) => {
      localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, old);
      migrateLegacyFontSize();
      expect(localStorage.getItem(FONT_SIZE_STORAGE_KEY)).toBe(renamed);
      expect(localStorage.getItem(LEGACY_FONT_SIZE_STORAGE_KEY)).toBeNull();
      expect(loadFontSize()).toBe(renamed);

      migrateLegacyFontSize();
      expect(localStorage.getItem(FONT_SIZE_STORAGE_KEY)).toBe(renamed);
      expect(localStorage.length).toBe(1);

      // A choice made after the move is never undone by a later start.
      localStorage.setItem(FONT_SIZE_STORAGE_KEY, 'large');
      migrateLegacyFontSize();
      expect(loadFontSize()).toBe('large');
    }
  );

  it('leaves a fresh install unset, so it keeps following the default', () => {
    migrateLegacyFontSize();
    expect(localStorage.length).toBe(0);
    expect(loadFontSize()).toBe('standard');
  });

  it('drops an old value no build wrote and keeps the new choice', () => {
    localStorage.setItem(FONT_SIZE_STORAGE_KEY, 'large');
    localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, 'huge');
    migrateLegacyFontSize();
    expect(localStorage.getItem(LEGACY_FONT_SIZE_STORAGE_KEY)).toBeNull();
    expect(localStorage.getItem(FONT_SIZE_STORAGE_KEY)).toBe('large');
  });

  it('takes the old key over the new one, because only an older build writes it after the move', () => {
    // A person who went back to 1.94.5 and chose Larger there, then updated again.
    localStorage.setItem(FONT_SIZE_STORAGE_KEY, 'small');
    localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, 'larger');
    expect(loadFontSize()).toBe('large');
    migrateLegacyFontSize();
    expect(localStorage.getItem(FONT_SIZE_STORAGE_KEY)).toBe('large');
    expect(localStorage.getItem(LEGACY_FONT_SIZE_STORAGE_KEY)).toBeNull();
  });

  it('drops the old key when the person chooses a size, so the old value cannot come back', () => {
    localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, 'larger');
    render(<FontSizeSelector />);
    expect(screen.getByRole('radio', { name: 'Large' })).toBeChecked();
    fireEvent.click(screen.getByRole('radio', { name: 'Small' }));
    expect(localStorage.getItem(LEGACY_FONT_SIZE_STORAGE_KEY)).toBeNull();
    expect(localStorage.getItem(FONT_SIZE_STORAGE_KEY)).toBe('small');
    expect(loadFontSize()).toBe('small');
  });

  it('is moved at renderer startup, before React mounts', () => {
    const renderer = readFileSync(resolve(__dirname, '../renderer.tsx'), 'utf8');
    expect(renderer).toMatch(/^\s*migrateLegacyFontSize\(\);$/m);
    expect(renderer.indexOf('migrateLegacyFontSize();')).toBeLessThan(
      renderer.indexOf('ReactDOM.createRoot')
    );
  });

  it('follows another window that moved the old key', () => {
    localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, 'larger');
    render(<FontSizeSelector />);
    expect(screen.getByRole('radio', { name: 'Large' })).toBeChecked();
    act(() => {
      localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, 'standard');
      window.dispatchEvent(new StorageEvent('storage', { key: LEGACY_FONT_SIZE_STORAGE_KEY }));
    });
    expect(screen.getByRole('radio', { name: 'Small' })).toBeChecked();
  });
});

describe('first paint', () => {
  const values = [null, 'small', 'standard', 'large', 'larger', 'invalid', 'constructor'];
  const pairs = values.flatMap((legacy) => values.map((current) => [legacy, current] as const));

  it.each(pairs)(
    'agrees with the hydrated preference for old key %s and new key %s',
    (legacy, current) => {
      if (legacy !== null) localStorage.setItem(LEGACY_FONT_SIZE_STORAGE_KEY, legacy);
      if (current !== null) localStorage.setItem(FONT_SIZE_STORAGE_KEY, current);
      runBootScript();
      expect(document.documentElement.dataset.fontSize).toBe(loadFontSize());
      expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe(
        String(FONT_SIZE_SCALE[loadFontSize()])
      );
      // And still after the renderer moves the old key at startup.
      migrateLegacyFontSize();
      expect(document.documentElement.dataset.fontSize).toBe(loadFontSize());
    }
  );

  it('paints the default size when storage cannot be read', () => {
    const getItem = Storage.prototype.getItem;
    Storage.prototype.getItem = () => {
      throw new Error('storage is blocked');
    };
    try {
      runBootScript();
    } finally {
      Storage.prototype.getItem = getItem;
    }
    expect(document.documentElement.dataset.fontSize).toBe(DEFAULT_FONT_SIZE);
    expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe(
      String(FONT_SIZE_SCALE[DEFAULT_FONT_SIZE])
    );
  });
});
