import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { ThemeProvider } from './ThemeContext';
import ThemeSelector from '../components/BioRouterSidebar/ThemeSelector';
import ThemeFamilySelector from '../components/BioRouterSidebar/ThemeFamilySelector';

/**
 * A `biorouter serve` browser has no Electron. `renderer.tsx` installs a bridge
 * of browser-safe stand-ins in its place, and that bridge has no other windows
 * to tell about a theme change and no native window to paint, so it carries
 * neither `broadcastThemeChange` nor `setWindowCanvas`.
 *
 * `window.electron?.broadcastThemeChange(…)` guards only the bridge, not the
 * method, so every Mode or Palette click in a browser threw "is not a function"
 * inside the click handler (measured on `serve`, 2026-09-14). The theme still
 * changed, because the state updates run before the call, which is why nobody
 * noticed. These click the real Appearance controls against bridges shaped like
 * that one, and fail on the throw itself.
 */

const originalElectron = window.electron;

/** Every error React reports for an event handler, which does not reach the caller. */
function captureHandlerErrors() {
  const errors: unknown[] = [];
  const onError = (event: ErrorEvent) => {
    errors.push(event.error ?? event.message);
    event.preventDefault();
  };
  window.addEventListener('error', onError);
  return { errors, stop: () => window.removeEventListener('error', onError) };
}

function installBridge(bridge: Record<string, unknown> | undefined) {
  Object.defineProperty(window, 'electron', { configurable: true, writable: true, value: bridge });
}

const mount = () =>
  render(
    <ThemeProvider>
      <ThemeSelector />
      <ThemeFamilySelector />
    </ThemeProvider>
  );

// The shape `renderer.tsx` installs for a browser, minus everything a theme
// never touches; and a bridge with nothing at all, so no call in ThemeProvider
// relies on any one method being present.
const bridges: Array<[string, () => Record<string, unknown>]> = [
  [
    'the browser bridge (no broadcastThemeChange, no setWindowCanvas)',
    () => ({
      on: () => () => {},
      off: () => {},
      platform: 'linux',
    }),
  ],
  ['a bridge with no methods at all', () => ({})],
];

describe.each(bridges)('Appearance controls on %s', (_name, makeBridge) => {
  let capture: ReturnType<typeof captureHandlerErrors>;

  beforeEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove('dark', 'light');
    document.documentElement.removeAttribute('data-theme');
    installBridge(makeBridge());
    capture = captureHandlerErrors();
  });

  afterEach(() => {
    capture.stop();
    cleanup();
    installBridge(originalElectron as unknown as Record<string, unknown>);
  });

  it('switches Mode without throwing, and the choice survives a reload', () => {
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'light');
    const view = mount();

    expect(() => fireEvent.click(screen.getByTestId('dark-mode-button'))).not.toThrow();
    expect(capture.errors).toEqual([]);
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(screen.getByTestId('dark-mode-button').getAttribute('aria-pressed')).toBe('true');

    expect(() => fireEvent.click(screen.getByTestId('system-mode-button'))).not.toThrow();
    expect(() => fireEvent.click(screen.getByTestId('dark-mode-button'))).not.toThrow();
    expect(capture.errors).toEqual([]);

    // A reload is a fresh provider reading what the click stored.
    view.unmount();
    document.documentElement.classList.remove('dark', 'light');
    mount();
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(screen.getByTestId('dark-mode-button').getAttribute('aria-pressed')).toBe('true');
    expect(capture.errors).toEqual([]);
  });

  it('switches Palette without throwing, and the choice survives a reload', () => {
    const view = mount();

    expect(() =>
      fireEvent.click(screen.getByTestId('theme-family-alma-mater-button'))
    ).not.toThrow();
    expect(capture.errors).toEqual([]);
    expect(document.documentElement.getAttribute('data-theme')).toBe('alma-mater');

    view.unmount();
    document.documentElement.removeAttribute('data-theme');
    mount();
    expect(document.documentElement.getAttribute('data-theme')).toBe('alma-mater');
    expect(capture.errors).toEqual([]);
  });
});
