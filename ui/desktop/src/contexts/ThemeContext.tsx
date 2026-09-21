import { useFontSize } from '../hooks/useFontSize';
import { THEME_FAMILY_IDS } from '../styles/themes.generated';
import React, { createContext, useContext, useEffect, useState, useCallback } from 'react';

type ThemePreference = 'light' | 'dark' | 'system';
type ResolvedTheme = 'light' | 'dark';
/**
 * Every theme family, in the order they appear in the Appearance settings.
 *
 * This is the single source of truth: `loadThemeFamily`, the cross-window IPC
 * convergence guard, and `ThemeFamilySelector` all derive from it, so adding a
 * family is one edit rather than three hardcoded string comparisons.
 *
 * `index.html`'s pre-hydration script cannot import TypeScript and duplicates
 * this list deliberately — keep the two in lockstep, exactly as
 * `loadThemePreference` already is.
 */
/**
 * Every theme family, derived from the generated theme data — the ONE list.
 * There used to be three hand-maintained copies of this (here, index.html's
 * pre-hydration script, and the picker), kept in step by a test rather than by
 * construction. Adding a family is now a single file in themes/.
 */
export const THEME_FAMILIES = THEME_FAMILY_IDS;

/**
 * The theme *family* — orthogonal to light/dark. `parchment` is the warm
 * default; `alma-mater` is the UCSF brand palette; `roche-limit` is the
 * JupyterLab-inspired white/grey/orange palette. Written to `data-theme` on
 * <html>; main.css re-colours the same semantic tokens per family.
 */
export type ThemeFamily = (typeof THEME_FAMILIES)[number];

function isThemeFamily(value: unknown): value is ThemeFamily {
  return THEME_FAMILIES.includes(value as ThemeFamily);
}

interface ThemeContextValue {
  userThemePreference: ThemePreference;
  setUserThemePreference: (pref: ThemePreference) => void;
  resolvedTheme: ResolvedTheme;
  themeFamily: ThemeFamily;
  setThemeFamily: (family: ThemeFamily) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

function getSystemTheme(): ResolvedTheme {
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function resolveTheme(preference: ThemePreference): ResolvedTheme {
  if (preference === 'system') {
    return getSystemTheme();
  }
  return preference;
}

/** Exported for tests only — the default-theme rules must stay pinned, and
 *  must stay in lockstep with index.html's pre-hydration script. */
export const loadThemePreferenceForTest = (): ThemePreference => loadThemePreference();

function loadThemePreference(): ThemePreference {
  const useSystemTheme = localStorage.getItem('use_system_theme');
  if (useSystemTheme === 'true') {
    return 'system';
  }
  // An explicit choice of light or dark. `saveThemePreference` writes both keys
  // together, and older builds wrote `theme` alone — so a stored `theme` means
  // the user picked one. Honour it; never drag someone back to auto on upgrade.
  const savedTheme = localStorage.getItem('theme');
  if (savedTheme === 'dark' || savedTheme === 'light') {
    return savedTheme;
  }

  // Nothing stored: a fresh install. Follow the OS, so the app opens dark at
  // night and light by day.
  //
  // This branch used to `return 'light'`, which ALSO made it a bug rather than
  // just a default: index.html's pre-hydration script already resolves an
  // unset preference to the system theme (`savedTheme ? … : systemPrefersDark`),
  // so on a dark-mode machine the window painted DARK and then React hydrated
  // and forced it LIGHT — a flash on every launch, because the two disagreed
  // about the same question. This keeps them in lockstep; change them together.
  return 'system';
}

function saveThemePreference(preference: ThemePreference): void {
  if (preference === 'system') {
    localStorage.setItem('use_system_theme', 'true');
  } else {
    localStorage.setItem('use_system_theme', 'false');
    localStorage.setItem('theme', preference);
  }
}

function applyThemeToDocument(theme: ResolvedTheme): void {
  const toRemove = theme === 'dark' ? 'light' : 'dark';
  document.documentElement.classList.add(theme);
  document.documentElement.classList.remove(toRemove);
}

function loadThemeFamily(): ThemeFamily {
  const stored = localStorage.getItem('theme_family');
  return isThemeFamily(stored) ? stored : 'parchment';
}

function saveThemeFamily(family: ThemeFamily): void {
  localStorage.setItem('theme_family', family);
}

function applyFamilyToDocument(family: ThemeFamily): void {
  // `data-theme` sits alongside the `.dark`/`.light` class; main.css scopes the
  // Alma Mater token overrides to `[data-theme='alma-mater']`. Parchment has no
  // matching rules, so the bare `:root`/`.dark` defaults render.
  document.documentElement.setAttribute('data-theme', family);
}

interface ThemeProviderProps {
  children: React.ReactNode;
}

export function ThemeProvider({ children }: ThemeProviderProps) {
  useFontSize();
  const [userThemePreference, setUserThemePreferenceState] =
    useState<ThemePreference>(loadThemePreference);
  const [resolvedTheme, setResolvedTheme] = useState<ResolvedTheme>(() =>
    resolveTheme(loadThemePreference())
  );
  const [themeFamily, setThemeFamilyState] = useState<ThemeFamily>(loadThemeFamily);

  const setUserThemePreference = useCallback(
    (preference: ThemePreference) => {
      setUserThemePreferenceState(preference);
      saveThemePreference(preference);

      const resolved = resolveTheme(preference);
      setResolvedTheme(resolved);

      // Broadcast to other windows via Electron (carry the family so windows converge).
      // ⚠ Optional-called on the METHOD, not only the bridge: a `biorouter serve`
      // browser installs a bridge with no other windows and no such method, and a
      // throw here escaped every Mode and Palette click there (renderer.tsx).
      window.electron?.broadcastThemeChange?.({
        mode: resolved,
        useSystemTheme: preference === 'system',
        theme: resolved,
        themeFamily,
      });
    },
    [themeFamily]
  );

  const setThemeFamily = useCallback(
    (family: ThemeFamily) => {
      setThemeFamilyState(family);
      saveThemeFamily(family);
      applyFamilyToDocument(family);

      window.electron?.broadcastThemeChange?.({
        mode: resolvedTheme,
        useSystemTheme: userThemePreference === 'system',
        theme: resolvedTheme,
        themeFamily: family,
      });
    },
    [resolvedTheme, userThemePreference]
  );

  // Listen for system theme changes when preference is 'system'
  useEffect(() => {
    if (userThemePreference !== 'system') return;

    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');

    const handleChange = () => {
      setResolvedTheme(getSystemTheme());
    };

    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  }, [userThemePreference]);

  // Listen for theme changes from other windows (via Electron IPC)
  useEffect(() => {
    if (!window.electron) return;

    const handleThemeChanged = (_event: unknown, ...args: unknown[]) => {
      const themeData = args[0] as {
        useSystemTheme: boolean;
        theme: string;
        themeFamily?: string;
      };
      const newPreference: ThemePreference = themeData.useSystemTheme
        ? 'system'
        : themeData.theme === 'dark'
          ? 'dark'
          : 'light';

      setUserThemePreferenceState(newPreference);
      saveThemePreference(newPreference);
      setResolvedTheme(resolveTheme(newPreference));

      // The native app menu broadcasts without a family; only converge when present.
      if (isThemeFamily(themeData.themeFamily)) {
        setThemeFamilyState(themeData.themeFamily);
        saveThemeFamily(themeData.themeFamily);
        applyFamilyToDocument(themeData.themeFamily);
      }
    };

    return window.electron.on?.('theme-changed', handleThemeChanged);
  }, []);

  // Apply theme to document whenever resolvedTheme changes — and to the native
  // window behind it. That background is what shows wherever a late frame does
  // not reach during a resize; left at Electron's default it was a white band
  // across a dark app (utils/windowCanvas.ts). On mount too, not only on a
  // change: the window was created with the theme the app last showed, which is
  // not this one when the OS flipped while the app was closed. Optional-called,
  // because a browser surface's bridge has no window to paint.
  useEffect(() => {
    applyThemeToDocument(resolvedTheme);
    window.electron?.setWindowCanvas?.(resolvedTheme);
  }, [resolvedTheme]);

  // Apply the theme family (data-theme) whenever it changes. The pre-hydration
  // script in index.html sets it first so there is no flash on load.
  useEffect(() => {
    applyFamilyToDocument(themeFamily);
  }, [themeFamily]);

  const value: ThemeContextValue = {
    userThemePreference,
    setUserThemePreference,
    resolvedTheme,
    themeFamily,
    setThemeFamily,
  };

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const context = useContext(ThemeContext);
  if (!context) {
    throw new Error('useTheme must be used within a ThemeProvider');
  }
  return context;
}

/// The resolved theme, or `light` when rendered outside a `ThemeProvider`.
///
/// For leaf components (e.g. a syntax-highlighted code block) that want to match
/// the theme but must not force every caller — including tests and standalone
/// renders — to wrap themselves in a provider.
export function useResolvedTheme(): 'light' | 'dark' {
  return useContext(ThemeContext)?.resolvedTheme ?? 'light';
}

/// The active theme family, or `parchment` when rendered outside a `ThemeProvider`.
///
/// Non-throwing (like `useResolvedTheme`) so leaf components — the code block, the
/// artifact preview, and the standalone harness — can match the family without
/// forcing every caller into a provider.
export function useThemeFamily(): ThemeFamily {
  return useContext(ThemeContext)?.themeFamily ?? 'parchment';
}
