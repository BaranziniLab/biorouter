import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeProvider } from './ThemeContext';
import ThemeSelector from '../components/BioRouterSidebar/ThemeSelector';

/**
 * The renderer is the only party that knows which theme a window is showing, so
 * it is the one that has to tell the main process — otherwise the window's
 * native background (what shows wherever a late frame does not reach during a
 * resize) stays whatever it was created with, and a dark app gets a white band.
 * See utils/windowCanvas.ts.
 *
 * These drive the real ThemeProvider through the real ThemeSelector buttons and
 * read what reached the bridge. jsdom cannot show the band itself; that is
 * measured in the running app (docs/desktop-ui/window-scaling-regressions.md,
 * "Unpainted window area"). What it CAN show is the half that lives here: the
 * report goes out on mount, on every click, and when the OS flips under a
 * "System" preference.
 */
type Bridge = {
  setWindowCanvas?: ReturnType<typeof vi.fn>;
  broadcastThemeChange: ReturnType<typeof vi.fn>;
  on: ReturnType<typeof vi.fn>;
};

const originalElectron = window.electron;
const originalMatchMedia = window.matchMedia;

function installBridge(withCanvas = true): Bridge {
  const bridge: Bridge = {
    broadcastThemeChange: vi.fn(),
    on: vi.fn(() => () => {}),
    ...(withCanvas ? { setWindowCanvas: vi.fn() } : {}),
  };
  Object.defineProperty(window, 'electron', { configurable: true, writable: true, value: bridge });
  return bridge;
}

/** A `prefers-color-scheme` the test can flip, like the OS switching at sunset. */
function installSystemScheme(dark: boolean) {
  const listeners = new Set<() => void>();
  let matches = dark;
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: (query: string) => ({
      get matches() {
        return query.includes('prefers-color-scheme: dark') ? matches : false;
      },
      media: query,
      onchange: null,
      addListener: () => {},
      removeListener: () => {},
      addEventListener: (_: string, fn: () => void) => listeners.add(fn),
      removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
      dispatchEvent: () => false,
    }),
  });
  return {
    flip(next: boolean) {
      matches = next;
      listeners.forEach((fn) => fn());
    },
  };
}

const lastCanvas = (bridge: Bridge) => {
  const calls = bridge.setWindowCanvas?.mock.calls ?? [];
  return calls[calls.length - 1]?.[0];
};

describe('ThemeProvider reports the window canvas', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove('dark', 'light');
  });

  afterEach(() => {
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: originalElectron,
    });
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      writable: true,
      value: originalMatchMedia,
    });
  });

  it.each(['dark', 'light'] as const)('reports a stored %s theme on mount', (theme) => {
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', theme);
    const bridge = installBridge();
    render(
      <ThemeProvider>
        <ThemeSelector />
      </ThemeProvider>
    );
    expect(bridge.setWindowCanvas).toHaveBeenCalledWith(theme);
    expect(lastCanvas(bridge)).toBe(theme);
  });

  it('follows every click on the theme buttons', () => {
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'light');
    const bridge = installBridge();
    render(
      <ThemeProvider>
        <ThemeSelector />
      </ThemeProvider>
    );
    fireEvent.click(screen.getByTestId('dark-mode-button'));
    expect(lastCanvas(bridge)).toBe('dark');
    fireEvent.click(screen.getByTestId('light-mode-button'));
    expect(lastCanvas(bridge)).toBe('light');
  });

  // The case a one-shot report would miss: "System" chosen, app left open, the
  // OS goes dark. The page repaints dark through matchMedia, so the window must
  // hear about it through the same path.
  it('follows the OS when the preference is System', () => {
    const scheme = installSystemScheme(false);
    const bridge = installBridge();
    render(
      <ThemeProvider>
        <ThemeSelector />
      </ThemeProvider>
    );
    expect(lastCanvas(bridge)).toBe('light');
    act(() => scheme.flip(true));
    expect(lastCanvas(bridge)).toBe('dark');
    act(() => scheme.flip(false));
    expect(lastCanvas(bridge)).toBe('light');
  });

  // A `biorouter serve` browser installs a bridge with no window behind it.
  it('does not require the bridge to carry setWindowCanvas', () => {
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'dark');
    installBridge(false);
    expect(() =>
      render(
        <ThemeProvider>
          <ThemeSelector />
        </ThemeProvider>
      )
    ).not.toThrow();
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });
});
