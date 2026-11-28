import { describe, expect, it } from 'vitest';
import { THEME_FAMILY_IDS } from '../styles/themes.generated';
import {
  DEFAULT_THEME_FAMILY,
  isThemeFamilyId,
  parseThemeAppearance,
  parseThemeAppearanceView,
  resolveAppearance,
  type ThemeAppearance,
} from './themeAppearance';

/**
 * A chat window tells main its appearance as words (DD-99), and main keeps only words it knows:
 * a generated family id, one of two modes, and a boolean. These pin both halves: what a report
 * may be, and what main resolves when it has no report, or no window at all.
 */

const report = (over: Partial<Record<keyof ThemeAppearance, unknown>> = {}): unknown => ({
  family: 'alma-mater',
  mode: 'dark',
  followsSystem: false,
  ...over,
});

describe('parseThemeAppearance', () => {
  it('accepts every generated family in both modes, with either preference', () => {
    for (const family of THEME_FAMILY_IDS) {
      for (const mode of ['light', 'dark'] as const) {
        for (const followsSystem of [true, false]) {
          expect(parseThemeAppearance({ family, mode, followsSystem })).toEqual({
            family,
            mode,
            followsSystem,
          });
        }
      }
    }
  });

  it('drops extra fields rather than keeping them', () => {
    expect(parseThemeAppearance({ ...(report() as object), colour: '#ff0000' })).toEqual({
      family: 'alma-mater',
      mode: 'dark',
      followsSystem: false,
    });
  });

  it('refuses an unknown family, including one that differs only in case or spacing', () => {
    for (const family of ['neon', 'Parchment', ' parchment', 'parchment ', '', '__proto__']) {
      expect(parseThemeAppearance(report({ family })), JSON.stringify(family)).toBeNull();
    }
  });

  it('refuses an unknown mode', () => {
    for (const mode of ['dim', 'system', 'Dark', '', '#000000']) {
      expect(parseThemeAppearance(report({ mode })), mode).toBeNull();
    }
  });

  it('refuses a field of the wrong type, a missing field, and anything that is not an object', () => {
    for (const bad of [
      report({ family: 7 }),
      report({ family: ['parchment'] }),
      report({ mode: 1 }),
      report({ followsSystem: 'true' }),
      report({ followsSystem: 1 }),
      report({ followsSystem: null }),
      { family: 'parchment', mode: 'light' },
      { mode: 'light', followsSystem: true },
      null,
      undefined,
      'parchment',
      42,
      ['parchment', 'light', true],
    ]) {
      expect(parseThemeAppearance(bad), JSON.stringify(bad)).toBeNull();
    }
  });

  it('knows the families from the generated list only', () => {
    for (const family of THEME_FAMILY_IDS) expect(isThemeFamilyId(family)).toBe(true);
    expect(isThemeFamilyId('toString')).toBe(false);
    expect(isThemeFamilyId(undefined)).toBe(false);
    expect(THEME_FAMILY_IDS as readonly string[]).toContain(DEFAULT_THEME_FAMILY);
  });
});

describe('parseThemeAppearanceView', () => {
  it('accepts a known family and a mode, and nothing else', () => {
    expect(parseThemeAppearanceView({ family: 'roche-limit', mode: 'light' })).toEqual({
      family: 'roche-limit',
      mode: 'light',
    });
    expect(
      parseThemeAppearanceView({ family: 'roche-limit', mode: 'light', followsSystem: true })
    ).toEqual({ family: 'roche-limit', mode: 'light' });
    for (const bad of [
      { family: 'neon', mode: 'light' },
      { family: 'roche-limit', mode: 'dim' },
      { family: 'roche-limit' },
      null,
      'roche-limit',
    ]) {
      expect(parseThemeAppearanceView(bad), JSON.stringify(bad)).toBeNull();
    }
  });
});

describe('resolveAppearance', () => {
  const none = {};

  it('takes the last report first', () => {
    const last: ThemeAppearance = { family: 'roche-limit', mode: 'dark', followsSystem: false };
    expect(
      resolveAppearance(
        last,
        { themeFamily: 'alma-mater', windowCanvasMode: 'light', themeFollowsSystem: true },
        false
      )
    ).toEqual({ family: 'roche-limit', mode: 'dark' });
  });

  it('reads the mode from the system when the last report follows it', () => {
    const last: ThemeAppearance = { family: 'alma-mater', mode: 'light', followsSystem: true };
    expect(resolveAppearance(last, none, true)).toEqual({ family: 'alma-mater', mode: 'dark' });
    expect(resolveAppearance(last, none, false)).toEqual({ family: 'alma-mater', mode: 'light' });
  });

  it('checks the last report again, and falls back past a bad one', () => {
    const bad = { family: 'neon', mode: 'dark', followsSystem: false } as ThemeAppearance;
    expect(
      resolveAppearance(bad, { themeFamily: 'alma-mater', windowCanvasMode: 'light' }, true)
    ).toEqual({ family: 'alma-mater', mode: 'light' });
  });

  it('with no report, takes the persisted settings', () => {
    expect(
      resolveAppearance(null, { themeFamily: 'alma-mater', windowCanvasMode: 'dark' }, false)
    ).toEqual({ family: 'alma-mater', mode: 'dark' });
    expect(
      resolveAppearance(
        null,
        { themeFamily: 'roche-limit', windowCanvasMode: 'light', themeFollowsSystem: true },
        true
      )
    ).toEqual({ family: 'roche-limit', mode: 'dark' });
  });

  it('with nothing remembered, follows the system and wears Parchment', () => {
    expect(resolveAppearance(null, none, true)).toEqual({ family: 'parchment', mode: 'dark' });
    expect(resolveAppearance(null, none, false)).toEqual({ family: 'parchment', mode: 'light' });
    expect(resolveAppearance(null, null, true)).toEqual({ family: 'parchment', mode: 'dark' });
    expect(resolveAppearance(null, undefined, false)).toEqual({
      family: 'parchment',
      mode: 'light',
    });
  });

  it('refuses unknown persisted words: a family, a mode or a preference of the wrong type', () => {
    const saved = {
      themeFamily: 'neon',
      windowCanvasMode: 'dim',
      themeFollowsSystem: 'yes',
    } as unknown as Parameters<typeof resolveAppearance>[1];
    expect(resolveAppearance(null, saved, true)).toEqual({ family: 'parchment', mode: 'dark' });
    expect(resolveAppearance(null, saved, false)).toEqual({ family: 'parchment', mode: 'light' });
  });

  it('keeps a remembered mode over the system, as a new window is created', () => {
    // `initialWindowCanvas`'s rule: the mode the app last showed wins over the system's.
    expect(resolveAppearance(null, { windowCanvasMode: 'light' }, true)).toEqual({
      family: 'parchment',
      mode: 'light',
    });
  });
});
