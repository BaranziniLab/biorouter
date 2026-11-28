/**
 * The theme a person sees, as words main can keep (DD-99).
 *
 * Surfaces outside the renderer, the dictation indicator above all, have no stylesheet and no
 * `localStorage` of their own, yet they should wear the family and the mode the person chose. So a
 * chat window reports its appearance to main over `theme:appearance`, and main resolves the
 * appearance those surfaces use from that report, its saved settings and the system.
 *
 * A renderer tells main a WORD, never a colour (`windowCanvas.ts`): a family id from the generated
 * list, a mode, and whether the mode follows the system. Main maps the words onto colours the theme
 * generator resolved (`styles/themes.generated.ts`), so no renderer can paint a window that floats
 * over other apps.
 *
 * No Electron and no DOM here, so main and the renderer both import it.
 */
import { THEME_FAMILY_IDS } from '../styles/themes.generated';
import type { Settings } from './settings';
import { isWindowCanvasMode, type WindowCanvasMode } from './windowCanvas';

/** What a chat window reports: its family, its mode, and whether the mode follows the system. */
export type ThemeAppearance = { family: string; mode: WindowCanvasMode; followsSystem: boolean };

/** What main resolved and hands to a surface with no theme of its own. */
export type ThemeAppearanceView = { family: string; mode: WindowCanvasMode };

/** The family every fallback lands on, as `sandboxedSurface` and `BioRouterMark` do. */
export const DEFAULT_THEME_FAMILY = 'parchment';

/** One of the generated family ids, and nothing else (a new family is accepted with no edit here). */
export function isThemeFamilyId(value: unknown): value is string {
  return typeof value === 'string' && (THEME_FAMILY_IDS as readonly string[]).includes(value);
}

/**
 * A renderer's report, or null when it is not exactly one: a known family, one of the two modes,
 * and a boolean. Extra fields are ignored; a wrong type or an unknown word in any of the three
 * refuses the whole report, so main never keeps half of one.
 */
export function parseThemeAppearance(raw: unknown): ThemeAppearance | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const row = raw as Record<string, unknown>;
  if (!isThemeFamilyId(row.family)) return null;
  if (!isWindowCanvasMode(row.mode)) return null;
  if (typeof row.followsSystem !== 'boolean') return null;
  return { family: row.family, mode: row.mode, followsSystem: row.followsSystem };
}

/**
 * The appearance main hands out (`theme:appearance:get` and `theme:appearance:changed`), checked
 * the same way on the receiving side: a known family and a mode, or null.
 */
export function parseThemeAppearanceView(raw: unknown): ThemeAppearanceView | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const row = raw as Record<string, unknown>;
  if (!isThemeFamilyId(row.family) || !isWindowCanvasMode(row.mode)) return null;
  return { family: row.family, mode: row.mode };
}

/** The saved settings this reads: the two theme words and the canvas a window was last given. */
export type SavedAppearance = Pick<
  Settings,
  'themeFamily' | 'themeFollowsSystem' | 'windowCanvasMode'
>;

/**
 * The appearance main uses when nothing better is known, in this order:
 *
 * 1. the last report a chat window sent (checked again, so a caller cannot hand in a bad one);
 * 2. the persisted settings, so the bubble is right while no chat window is open (fn in other
 *    apps and the global shortcut need none);
 * 3. for the mode, `initialWindowCanvas`'s rule: the mode the app last showed, else the system;
 * 4. for the family, Parchment.
 *
 * A mode that follows the system is read from `nativeDark` (main's
 * `nativeTheme.shouldUseDarkColors`), not from the report, because the system can flip while no
 * renderer is alive to say so.
 */
export function resolveAppearance(
  last: ThemeAppearance | null,
  saved: SavedAppearance | null | undefined,
  nativeDark: boolean
): ThemeAppearanceView {
  const report = parseThemeAppearance(last);
  const system: WindowCanvasMode = nativeDark === true ? 'dark' : 'light';
  const savedFamily = saved?.themeFamily;
  const savedCanvas = saved?.windowCanvasMode;
  const family = report
    ? report.family
    : isThemeFamilyId(savedFamily)
      ? savedFamily
      : DEFAULT_THEME_FAMILY;
  let mode: WindowCanvasMode;
  if (report) {
    mode = report.followsSystem ? system : report.mode;
  } else if (saved?.themeFollowsSystem === true) {
    mode = system;
  } else if (isWindowCanvasMode(savedCanvas)) {
    mode = savedCanvas;
  } else {
    mode = system;
  }
  return { family, mode };
}
