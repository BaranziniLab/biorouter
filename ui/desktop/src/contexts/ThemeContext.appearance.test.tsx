import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MainAppearanceThemeProvider, ThemeProvider, useTheme } from './ThemeContext';
import ThemeSelector from '../components/BioRouterSidebar/ThemeSelector';
import ThemeFamilySelector from '../components/BioRouterSidebar/ThemeFamilySelector';
import { THEME_FAMILY_IDS } from '../styles/themes.generated';

/**
 * The dictation indicator has no stylesheet, no preload and no `localStorage`, so it can wear the
 * person's theme only if a chat window tells main which one it shows (DD-99). These drive the
 * real ThemeProvider through the real Appearance controls and read what reached the bridge: a
 * family id, a mode and whether the mode follows the system, on mount and on every change, and
 * never a colour.
 *
 * The second half is the window that shows main's appearance instead of its own (the dictation
 * review card): it adopts what main answers and announces, checks every word, and never reports.
 */
type Report = { family: string; mode: 'light' | 'dark'; followsSystem: boolean };
type Bridge = {
  setThemeAppearance?: ReturnType<typeof vi.fn>;
  broadcastThemeChange: ReturnType<typeof vi.fn>;
  on: ReturnType<typeof vi.fn>;
};

const originalElectron = window.electron;
const originalMatchMedia = window.matchMedia;

function installBridge(withAppearance = true): Bridge {
  const bridge: Bridge = {
    broadcastThemeChange: vi.fn(),
    on: vi.fn(() => () => {}),
    ...(withAppearance ? { setThemeAppearance: vi.fn() } : {}),
  };
  Object.defineProperty(window, 'electron', { configurable: true, writable: true, value: bridge });
  return bridge;
}

function setBridge(value: unknown) {
  Object.defineProperty(window, 'electron', { configurable: true, writable: true, value });
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

const reports = (bridge: Bridge): Report[] =>
  (bridge.setThemeAppearance?.mock.calls ?? []).map((call) => call[0] as Report);
const lastReport = (bridge: Bridge): Report | undefined => {
  const all = reports(bridge);
  return all[all.length - 1];
};

const mountControls = () =>
  render(
    <ThemeProvider>
      <ThemeSelector />
      <ThemeFamilySelector />
    </ThemeProvider>
  );

function resetDocument() {
  localStorage.clear();
  document.documentElement.classList.remove('dark', 'light');
  document.documentElement.removeAttribute('data-theme');
}

afterEach(() => {
  cleanup();
  setBridge(originalElectron);
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: originalMatchMedia,
  });
});

describe('ThemeProvider reports the appearance to main', () => {
  beforeEach(resetDocument);

  it('reports the stored family and mode on mount, as words only', () => {
    installSystemScheme(false);
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'dark');
    localStorage.setItem('theme_family', 'alma-mater');
    const bridge = installBridge();
    mountControls();
    expect(lastReport(bridge)).toEqual({
      family: 'alma-mater',
      mode: 'dark',
      followsSystem: false,
    });
    for (const report of reports(bridge)) {
      expect(Object.keys(report).sort()).toEqual(['family', 'followsSystem', 'mode']);
      expect(THEME_FAMILY_IDS as readonly string[]).toContain(report.family);
      expect(JSON.stringify(report)).not.toMatch(/#[0-9a-f]{3,8}|rgba?\(|hsla?\(/i);
    }
  });

  it('reports again on every family change', () => {
    installSystemScheme(false);
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'light');
    const bridge = installBridge();
    mountControls();
    expect(lastReport(bridge)).toEqual({
      family: 'parchment',
      mode: 'light',
      followsSystem: false,
    });
    for (const family of THEME_FAMILY_IDS) {
      fireEvent.click(screen.getByTestId(`theme-family-${family}-button`));
      expect(lastReport(bridge)).toEqual({ family, mode: 'light', followsSystem: false });
    }
  });

  it('reports again on every mode change', () => {
    installSystemScheme(false);
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'light');
    localStorage.setItem('theme_family', 'roche-limit');
    const bridge = installBridge();
    mountControls();
    fireEvent.click(screen.getByTestId('dark-mode-button'));
    expect(lastReport(bridge)).toEqual({
      family: 'roche-limit',
      mode: 'dark',
      followsSystem: false,
    });
    fireEvent.click(screen.getByTestId('light-mode-button'));
    expect(lastReport(bridge)).toEqual({
      family: 'roche-limit',
      mode: 'light',
      followsSystem: false,
    });
  });

  it('reports a preference change even when the mode it shows stays the same', () => {
    // The system is dark and the person picked Dark; choosing System changes no colour, but main
    // must now follow the system while no window is open.
    installSystemScheme(true);
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'dark');
    const bridge = installBridge();
    mountControls();
    expect(lastReport(bridge)).toEqual({ family: 'parchment', mode: 'dark', followsSystem: false });
    fireEvent.click(screen.getByTestId('system-mode-button'));
    expect(lastReport(bridge)).toEqual({ family: 'parchment', mode: 'dark', followsSystem: true });
    fireEvent.click(screen.getByTestId('dark-mode-button'));
    expect(lastReport(bridge)).toEqual({ family: 'parchment', mode: 'dark', followsSystem: false });
  });

  it('follows the system under System, and reports each flip', () => {
    const scheme = installSystemScheme(false);
    const bridge = installBridge();
    mountControls();
    expect(lastReport(bridge)).toEqual({ family: 'parchment', mode: 'light', followsSystem: true });
    act(() => scheme.flip(true));
    expect(lastReport(bridge)).toEqual({ family: 'parchment', mode: 'dark', followsSystem: true });
    act(() => scheme.flip(false));
    expect(lastReport(bridge)).toEqual({ family: 'parchment', mode: 'light', followsSystem: true });
  });

  it('reports a change another window broadcast, so every window agrees with main', () => {
    installSystemScheme(false);
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'light');
    let broadcast: ((event: unknown, data: unknown) => void) | null = null;
    const bridge = installBridge();
    bridge.on.mockImplementation((channel: string, handler: typeof broadcast) => {
      if (channel === 'theme-changed') broadcast = handler;
      return () => {};
    });
    mountControls();
    expect(broadcast).not.toBeNull();
    act(() =>
      broadcast!(null, { useSystemTheme: false, theme: 'dark', themeFamily: 'alma-mater' })
    );
    expect(lastReport(bridge)).toEqual({
      family: 'alma-mater',
      mode: 'dark',
      followsSystem: false,
    });
  });

  it('is a no-op on a bridge without the method, and with no bridge at all', () => {
    installSystemScheme(false);
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', 'dark');
    installBridge(false);
    expect(() => mountControls()).not.toThrow();
    expect(() => fireEvent.click(screen.getByTestId('light-mode-button'))).not.toThrow();
    expect(() =>
      fireEvent.click(screen.getByTestId('theme-family-alma-mater-button'))
    ).not.toThrow();
    expect(document.documentElement.getAttribute('data-theme')).toBe('alma-mater');
    cleanup();

    setBridge(undefined);
    expect(() => mountControls()).not.toThrow();
    expect(document.documentElement.classList.contains('light')).toBe(true);
  });
});

type View = { family: string; mode: 'light' | 'dark' };

function installMainBridge(initial: unknown) {
  let push: ((view: View) => void) | null = null;
  const bridge = {
    getThemeAppearance: vi.fn(async () => initial),
    onThemeAppearance: vi.fn((callback: (view: View) => void) => {
      push = callback;
      return () => {
        push = null;
      };
    }),
    setThemeAppearance: vi.fn(),
    broadcastThemeChange: vi.fn(),
    on: vi.fn(() => () => {}),
  };
  setBridge(bridge);
  return {
    bridge,
    push: (view: unknown) => act(() => push?.(view as View)),
    subscribed: () => push !== null,
  };
}

const settle = async () => {
  for (let i = 0; i < 4; i++) await act(async () => {});
};

function Probe() {
  const { resolvedTheme, themeFamily } = useTheme();
  return <output data-testid="probe">{`${themeFamily} ${resolvedTheme}`}</output>;
}

const root = () => document.documentElement;

describe('MainAppearanceThemeProvider follows main', () => {
  beforeEach(resetDocument);

  it('starts from what the document shows, then adopts main’s answer', async () => {
    root().classList.add('light');
    const main = installMainBridge({ family: 'alma-mater', mode: 'dark' });
    render(
      <MainAppearanceThemeProvider>
        <Probe />
      </MainAppearanceThemeProvider>
    );
    await settle();
    expect(main.bridge.getThemeAppearance).toHaveBeenCalledTimes(1);
    expect(root().getAttribute('data-theme')).toBe('alma-mater');
    expect(root().classList.contains('dark')).toBe(true);
    expect(root().classList.contains('light')).toBe(false);
    expect(screen.getByTestId('probe')).toHaveTextContent('alma-mater dark');
  });

  it('follows every change main announces while it is open', async () => {
    const main = installMainBridge(null);
    render(
      <MainAppearanceThemeProvider>
        <Probe />
      </MainAppearanceThemeProvider>
    );
    await settle();
    expect(main.subscribed()).toBe(true);
    main.push({ family: 'roche-limit', mode: 'dark' });
    expect(root().getAttribute('data-theme')).toBe('roche-limit');
    expect(root().classList.contains('dark')).toBe(true);
    main.push({ family: 'parchment', mode: 'light' });
    expect(root().getAttribute('data-theme')).toBe('parchment');
    expect(root().classList.contains('light')).toBe(true);
    expect(screen.getByTestId('probe')).toHaveTextContent('parchment light');
  });

  it('ignores an unknown family, an unknown mode and a value of the wrong type', async () => {
    root().classList.add('light');
    root().setAttribute('data-theme', 'alma-mater');
    const main = installMainBridge({ family: 'neon', mode: 'dark' });
    render(
      <MainAppearanceThemeProvider>
        <Probe />
      </MainAppearanceThemeProvider>
    );
    await settle();
    for (const bad of [
      { family: 'roche-limit', mode: 'dim' },
      { family: 7, mode: 'dark' },
      { family: 'roche-limit' },
      'roche-limit',
      null,
    ]) {
      main.push(bad);
    }
    expect(root().getAttribute('data-theme')).toBe('alma-mater');
    expect(root().classList.contains('light')).toBe(true);
    expect(screen.getByTestId('probe')).toHaveTextContent('alma-mater light');
  });

  it('never reports an appearance of its own, and unsubscribes on unmount', async () => {
    const main = installMainBridge({ family: 'alma-mater', mode: 'dark' });
    const view = render(
      <MainAppearanceThemeProvider>
        <Probe />
      </MainAppearanceThemeProvider>
    );
    await settle();
    expect(main.bridge.setThemeAppearance).not.toHaveBeenCalled();
    expect(main.bridge.broadcastThemeChange).not.toHaveBeenCalled();
    expect(localStorage.getItem('theme_family')).toBeNull();
    view.unmount();
    expect(main.subscribed()).toBe(false);
  });

  it('keeps the document’s theme on a bridge without the methods', async () => {
    root().classList.add('dark');
    root().setAttribute('data-theme', 'roche-limit');
    setBridge({ on: () => () => {} });
    render(
      <MainAppearanceThemeProvider>
        <Probe />
      </MainAppearanceThemeProvider>
    );
    await settle();
    expect(screen.getByTestId('probe')).toHaveTextContent('roche-limit dark');
    expect(root().classList.contains('dark')).toBe(true);
  });
});
