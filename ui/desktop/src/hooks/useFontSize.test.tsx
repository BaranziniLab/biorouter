import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import FontSizeSelector from '../components/settings/app/FontSizeSelector';
import { FONT_SIZE_STORAGE_KEY, FONT_SIZE_SCALE, loadFontSize, useFontSize } from './useFontSize';

function Observer() {
  const { fontSize, fontScale } = useFontSize();
  return (
    <output>
      {fontSize}:{fontScale}
    </output>
  );
}

beforeEach(() => localStorage.clear());
afterEach(cleanup);

describe('app font size', () => {
  it.each([null, 'small', 'unknown', 'Large', 'constructor'])(
    'defaults to Standard for %s',
    (stored) => {
      if (stored !== null) localStorage.setItem(FONT_SIZE_STORAGE_KEY, stored);
      render(<FontSizeSelector />);
      expect(screen.getByRole('radio', { name: 'Standard' })).toBeChecked();
      expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe('1');
    }
  );

  it.each([
    ['Standard', 'standard', '1'],
    ['Large', 'large', '1.07'],
    ['Larger', 'larger', '1.15'],
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
    expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe('1.07');
    act(() => {
      localStorage.clear();
      window.dispatchEvent(new StorageEvent('storage', { key: null }));
    });
    expect(screen.getByRole('radio', { name: 'Standard' })).toBeChecked();
  });

  it.each([null, 'small', 'standard', 'large', 'larger', 'invalid', 'constructor'])(
    'first paint agrees with the hydrated preference for %s',
    (stored) => {
      if (stored !== null) localStorage.setItem(FONT_SIZE_STORAGE_KEY, stored);
      const html = readFileSync(resolve(__dirname, '../../index.html'), 'utf8');
      const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
      expect(script).toBeDefined();
      new Function(script!)();
      expect(document.documentElement.dataset.fontSize).toBe(loadFontSize());
      expect(document.documentElement.style.getPropertyValue('--app-font-scale')).toBe(
        String(FONT_SIZE_SCALE[loadFontSize()])
      );
    }
  );
});
