/**
 * The on-screen dictation indicator: a small capsule at the bottom center of the active display,
 * shown only while a dictation records or finishes, in Biorouter and in other apps.
 *
 * - It wears the armed Send button's colours (DD-99): the accent fill with the on-accent ink, for
 *   the theme family and the mode the person chose, and it follows a change while it shows. The
 *   fill is opaque, so the desktop behind it never tints it.
 * - While recording, four thin bars follow the microphone's level (the composer's level, or the
 *   hidden Dictate to clipboard page's), so a person can see that sound is arriving.
 * - While the text is being finished, the bars settle into a slow pulse.
 * - Under Reduce motion, one still dot stands in for both: solid while recording, a ring of the
 *   same full ink while finishing, so the two read apart by shape and not by a fainter colour.
 * - It is never red, in any state (design.md 9.14).
 *
 * It is a transparent, frameless window that never takes focus, ignores the mouse (clicks go to
 * whatever is under it), stays out of the Dock and Mission Control, and floats over every Space
 * and full-screen app. It is created on the first dictation and then hidden, not closed, between
 * dictations. Its page is a fixed string with no preload and no Node; main updates it with two
 * calls built from validated values: the mode and level, and the palette.
 *
 * ⚠ The colours are never chosen here and never come from a renderer. `indicatorPalette` reads
 * them from the theme generator's output (`surface.accent`, `surface.onAccent`, `surface.shadow`
 * in `styles/themes.generated.ts`), keyed by a family id and a mode that main checked
 * (`utils/themeAppearance.ts`), and every value is checked again before it reaches the page.
 */
import { GENERATED_THEMES, type GeneratedTheme } from '../styles/themes.generated';
import { WINDOW_CANVAS, type WindowCanvasMode } from './windowCanvas';

export type IndicatorMode = 'hidden' | 'recording' | 'finishing';
export type DictationActivity = 'idle' | 'arming' | 'recording' | 'transcribing';

export interface DictationActivityReport {
  activity: DictationActivity;
  /** The microphone level on the 0 to 1 meter scale (`levelToMeter`). */
  level: number;
}

const ACTIVITIES: readonly DictationActivity[] = ['idle', 'arming', 'recording', 'transcribing'];

/** A report from a renderer (`dictation:indicator`), or null when it is not one. */
export function parseIndicatorReport(raw: unknown): DictationActivityReport | null {
  if (!raw || typeof raw !== 'object') return null;
  const row = raw as Record<string, unknown>;
  if (!ACTIVITIES.includes(row.activity as DictationActivity)) return null;
  const level = typeof row.level === 'number' && Number.isFinite(row.level) ? row.level : 0;
  return {
    activity: row.activity as DictationActivity,
    level: Math.max(0, Math.min(1, level)),
  };
}

/**
 * What the indicator shows for every dictation that reports: recording wins over finishing, and
 * the level is the loudest recording's. Arming shows nothing, so a start that is cancelled before
 * the microphone opens never flashes it.
 */
export function indicatorModeFor(reports: Iterable<DictationActivityReport>): {
  mode: IndicatorMode;
  level: number;
} {
  let recording = false;
  let finishing = false;
  let level = 0;
  for (const report of reports) {
    if (report.activity === 'recording') {
      recording = true;
      level = Math.max(level, report.level);
    } else if (report.activity === 'transcribing') {
      finishing = true;
    }
  }
  if (recording) return { mode: 'recording', level };
  return { mode: finishing ? 'finishing' : 'hidden', level: 0 };
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The window: the capsule plus room for its shadow. */
export const INDICATOR_SIZE = { width: 72, height: 36 } as const;
/** Space between the window and the bottom of the work area (above the Dock when it shows). */
export const INDICATOR_MARGIN = 6;

/** Bottom center of a display's work area. */
export function indicatorBounds(workArea: Rect): Rect {
  return {
    x: Math.round(workArea.x + (workArea.width - INDICATOR_SIZE.width) / 2),
    y: Math.round(workArea.y + workArea.height - INDICATOR_SIZE.height - INDICATOR_MARGIN),
    width: INDICATOR_SIZE.width,
    height: INDICATOR_SIZE.height,
  };
}

/** The indicator's three colours: the capsule's fill, the ink of its bars and dot, its shadow. */
export interface IndicatorPalette {
  fill: string;
  ink: string;
  shadow: string;
}

const HEX = /^#[0-9a-f]{6}$/i;
// One shadow layer: two to four lengths in px (or a bare 0), then an rgba() colour. The generator
// holds every family's `surface.shadow` to the same shape (`scripts/generate-themes.mjs`).
const SHADOW_LAYER =
  /^(?:-?\d*\.?\d+(?:px)?\s+){2,4}rgba\(\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}\s*,\s*(?:0|1|0?\.\d+)\s*\)$/;
const SHADOW_MAX_LENGTH = 200;

function isShadow(value: unknown): value is string {
  if (typeof value !== 'string' || value.length > SHADOW_MAX_LENGTH) return false;
  if (value === 'none') return true;
  return value.split(/,\s*(?![^()]*\))/).every((layer) => SHADOW_LAYER.test(layer.trim()));
}

/**
 * A palette whose every value is safe to write into the page, or null. The fill and the ink are
 * 6-digit hex, and the shadow is `none` or px lengths with rgba() colours, so nothing else (a
 * `var()`, a `url()`, a quote, a brace) can reach the page's CSS.
 */
export function checkIndicatorPalette(raw: unknown): IndicatorPalette | null {
  if (!raw || typeof raw !== 'object') return null;
  const { fill, ink, shadow } = raw as Record<string, unknown>;
  if (typeof fill !== 'string' || !HEX.test(fill)) return null;
  if (typeof ink !== 'string' || !HEX.test(ink)) return null;
  if (!isShadow(shadow)) return null;
  return { fill, ink, shadow };
}

const THEMES = GENERATED_THEMES as Record<string, GeneratedTheme | undefined>;

function generatedPalette(family: string, mode: WindowCanvasMode): IndicatorPalette | null {
  // An own key only, so a family word such as `constructor` never reaches the object's prototype.
  if (!Object.prototype.hasOwnProperty.call(THEMES, family)) return null;
  const surface = THEMES[family]?.[mode]?.surface;
  if (!surface) return null;
  return checkIndicatorPalette({
    fill: surface.accent,
    ink: surface.onAccent,
    shadow: surface.shadow,
  });
}

/**
 * The armed Send button's colours for a family and a mode (DD-99): `--background-accent` for the
 * fill, `--text-on-accent` for the ink and `--shadow-default` for the edge, as the theme generator
 * resolved them. An unknown family, or a value that does not pass the checks, gives Parchment's.
 *
 * The last line is never reached while the generator holds Parchment's values to the same checks
 * (it refuses to emit otherwise, and `dictationIndicator.test.ts` asserts every family). It is the
 * app's own two canvases with no shadow, so even then the bubble is a plain capsule and the page
 * takes no colour from anywhere else.
 */
export function indicatorPalette(family: string, mode: WindowCanvasMode): IndicatorPalette {
  const safeMode: WindowCanvasMode = mode === 'dark' ? 'dark' : 'light';
  return (
    generatedPalette(family, safeMode) ??
    generatedPalette('parchment', safeMode) ?? {
      fill: WINDOW_CANVAS.dark,
      ink: WINDOW_CANVAS.light,
      shadow: 'none',
    }
  );
}

/** The palette a window is built with before main has said anything (Parchment, light). */
export const DEFAULT_INDICATOR_PALETTE: IndicatorPalette = indicatorPalette('parchment', 'light');

/**
 * The indicator's page. Fixed text apart from the three palette values, which come from
 * `checkIndicatorPalette`: nothing in it comes from a person, a model or a file.
 *
 * The colours are custom properties on `:root`, written once here so the first frame is already
 * right, and replaced by `__brIndicatorTheme` on a change. Nothing transitions a colour: a theme
 * change is a jump in this app (`suspendTransitionsForThemeChange`), so only opacity, transform
 * and height move.
 */
export function indicatorPageHtml(palette: IndicatorPalette = DEFAULT_INDICATOR_PALETTE): string {
  const colours = checkIndicatorPalette(palette) ?? DEFAULT_INDICATOR_PALETTE;
  return `<!doctype html>
<html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'">
<style>
:root{--br-fill:${colours.fill};--br-ink:${colours.ink};--br-shadow:${colours.shadow}}
html,body{margin:0;height:100%;background:transparent;overflow:hidden;user-select:none;cursor:default}
body{display:flex;align-items:center;justify-content:center}
.pill{display:flex;align-items:center;justify-content:center;gap:3px;width:42px;height:16px;border-radius:8px;
background:var(--br-fill);box-shadow:var(--br-shadow);
opacity:0;transform:translateY(2px) scale(.94);transition:opacity .16s ease,transform .16s ease}
body.on .pill{opacity:1;transform:none}
.bar{display:block;width:2px;height:2px;border-radius:1px;background:var(--br-ink);transition:height .1s linear}
body.finishing .bar{height:2px;animation:settle 1.2s ease-in-out infinite}
body.finishing .bar:nth-child(2){animation-delay:.15s}
body.finishing .bar:nth-child(3){animation-delay:.3s}
body.finishing .bar:nth-child(4){animation-delay:.45s}
@keyframes settle{0%,100%{opacity:.35}50%{opacity:.95}}
.dot{display:none;box-sizing:border-box;width:5px;height:5px;border-radius:50%;background:var(--br-ink)}
body.still .pill{transition:none;transform:none}
body.still .bar{display:none}
body.still .dot{display:block}
body.still.finishing .dot{background:transparent;box-shadow:inset 0 0 0 1.5px var(--br-ink)}
</style></head>
<body><div class="pill" aria-hidden="true"><i class="bar"></i><i class="bar"></i><i class="bar"></i><i class="bar"></i><span class="dot"></span></div>
<script>
(() => {
  const body = document.body;
  const root = document.documentElement;
  const bars = Array.from(document.querySelectorAll('.bar'));
  const shape = [0.6, 1, 0.85, 0.7];
  const still = matchMedia('(prefers-reduced-motion: reduce)');
  const sync = () => body.classList.toggle('still', still.matches);
  sync();
  still.addEventListener('change', sync);
  window.__brIndicator = (mode, level) => {
    body.classList.toggle('on', mode !== 'hidden');
    body.classList.toggle('finishing', mode === 'finishing');
    bars.forEach((bar, i) => {
      const lift = mode === 'recording' ? level * shape[i] * (0.8 + Math.random() * 0.4) : 0;
      bar.style.height = mode === 'recording' ? (2 + Math.round(10 * Math.min(1, lift))) + 'px' : '';
    });
  };
  window.__brIndicatorTheme = (fill, ink, shadow) => {
    root.style.setProperty('--br-fill', fill);
    root.style.setProperty('--br-ink', ink);
    root.style.setProperty('--br-shadow', shadow);
  };
})();
</script></body></html>`;
}

/** The call main makes into the page for the mode. Both values are checked before they are written. */
export function indicatorScript(mode: IndicatorMode, level: number): string {
  const safeMode: IndicatorMode = mode === 'recording' || mode === 'finishing' ? mode : 'hidden';
  const safeLevel = Number.isFinite(level) ? Math.max(0, Math.min(1, level)) : 0;
  return `window.__brIndicator && window.__brIndicator(${JSON.stringify(safeMode)}, ${safeLevel.toFixed(3)})`;
}

/**
 * The call main makes into the page for the colours, or null when the palette does not pass
 * `checkIndicatorPalette` (a value that is not a 6-digit hex, or a shadow of another shape). The
 * values are written as JSON strings, as the mode is.
 */
export function indicatorThemeScript(palette: IndicatorPalette): string | null {
  const safe = checkIndicatorPalette(palette);
  if (!safe) return null;
  const args = [safe.fill, safe.ink, safe.shadow].map((value) => JSON.stringify(value)).join(', ');
  return `window.__brIndicatorTheme && window.__brIndicatorTheme(${args})`;
}

const samePalette = (a: IndicatorPalette, b: IndicatorPalette): boolean =>
  a.fill === b.fill && a.ink === b.ink && a.shadow === b.shadow;

/** The parts of a `BrowserWindow` the indicator uses, so a test can hand it a fake. */
export interface IndicatorWindow {
  isDestroyed(): boolean;
  setBounds(bounds: Rect): void;
  showInactive(): void;
  hide(): void;
  destroy(): void;
  loadURL(url: string): Promise<unknown>;
  webContents: { executeJavaScript(code: string): Promise<unknown> };
}

export interface DictationIndicatorOptions {
  /** A new hidden window with the indicator's window options (main.ts). */
  createWindow: () => IndicatorWindow;
  /** The work area of the display the person is on. */
  workArea: () => Rect;
  /**
   * The palette for the appearance main resolved (`indicatorPalette(family, mode)`), read each
   * time a window is built so its first frame is already right. A change while a window exists
   * goes through `setPalette`. Without it the window starts in Parchment light.
   */
  palette?: () => IndicatorPalette;
  now?: () => number;
}

/** The fastest a renderer reports its level: ten times a second, one 100 ms chunk each. */
export const INDICATOR_REPORT_INTERVAL_MS = 100;

/** The fastest the bars are redrawn: about 12 times a second. */
export const INDICATOR_LEVEL_INTERVAL_MS = 80;
/** How long the fade out runs before the window hides. */
export const INDICATOR_FADE_MS = 180;

export class DictationIndicator {
  private win: IndicatorWindow | null = null;
  private loaded: Promise<unknown> | null = null;
  private mode: IndicatorMode = 'hidden';
  private palette: IndicatorPalette = DEFAULT_INDICATOR_PALETTE;
  /** The palette the current window's page shows, or will once its queued call lands. */
  private pagePalette: IndicatorPalette | null = null;
  private lastLevelAt = -Infinity;
  private hideTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly now: () => number;

  constructor(private readonly options: DictationIndicatorOptions) {
    this.now = options.now ?? (() => Date.now());
  }

  get shownMode(): IndicatorMode {
    return this.mode;
  }

  /** The palette the indicator wears now, and every window it builds next. */
  get currentPalette(): IndicatorPalette {
    return { ...this.palette };
  }

  /**
   * Wear a new palette (a family or mode change). A window that exists, shown or hidden for
   * reuse, is repainted once its page has loaded; this never shows a hidden window and never
   * creates one. A palette that does not pass `checkIndicatorPalette` is ignored, and the
   * indicator keeps the one it has.
   */
  setPalette(palette: IndicatorPalette): void {
    const safe = checkIndicatorPalette(palette);
    if (!safe || samePalette(safe, this.palette)) return;
    this.palette = safe;
    this.sendPalette();
  }

  update(mode: IndicatorMode, level = 0): void {
    if (mode === 'hidden') {
      if (this.mode === 'hidden') return;
      this.mode = 'hidden';
      this.send('hidden', 0);
      this.scheduleHide();
      return;
    }
    const changed = mode !== this.mode;
    if (!changed && mode === 'recording') {
      // Level only: at most every INDICATOR_LEVEL_INTERVAL_MS.
      if (this.now() - this.lastLevelAt < INDICATOR_LEVEL_INTERVAL_MS) return;
    }
    if (!changed && mode === 'finishing') return;
    const appearing = this.mode === 'hidden';
    this.mode = mode;
    this.cancelHide();
    const reused = !!this.win && !this.win.isDestroyed();
    const win = this.ensureWindow();
    if (!win) return;
    if (appearing) {
      // A window kept hidden since the last dictation may have missed a change; main's
      // appearance now is the one to show (a new window was just built with it).
      if (reused) this.refreshPalette();
      win.setBounds(indicatorBounds(this.options.workArea()));
      win.showInactive();
    }
    this.lastLevelAt = this.now();
    this.send(mode, level);
  }

  dispose(): void {
    this.cancelHide();
    const win = this.win;
    this.win = null;
    this.loaded = null;
    this.pagePalette = null;
    this.mode = 'hidden';
    if (win && !win.isDestroyed()) win.destroy();
  }

  private ensureWindow(): IndicatorWindow | null {
    if (this.win && !this.win.isDestroyed()) return this.win;
    try {
      this.win = this.options.createWindow();
    } catch {
      this.win = null;
      return null;
    }
    // Main's appearance now, so the first frame is right; a getter that fails, or answers with a
    // palette that does not pass the checks, leaves the last one in place.
    // `loaded` is cleared first so nothing is queued on the previous window's page load.
    this.loaded = null;
    this.pagePalette = null;
    this.refreshPalette();
    this.pagePalette = this.palette;
    this.loaded = this.win
      .loadURL(
        `data:text/html;charset=utf-8,${encodeURIComponent(indicatorPageHtml(this.palette))}`
      )
      .catch(() => undefined);
    return this.win;
  }

  private refreshPalette(): void {
    if (!this.options.palette) return;
    let fresh: IndicatorPalette | null = null;
    try {
      fresh = checkIndicatorPalette(this.options.palette());
    } catch {
      fresh = null;
    }
    if (!fresh || samePalette(fresh, this.palette)) return;
    this.palette = fresh;
    this.sendPalette();
  }

  private sendPalette(): void {
    const win = this.win;
    if (!win || win.isDestroyed() || !this.loaded) return;
    if (this.pagePalette && samePalette(this.pagePalette, this.palette)) return;
    const palette = this.palette;
    const code = indicatorThemeScript(palette);
    if (!code) return;
    this.pagePalette = palette;
    void this.loaded.then(() => {
      if (win.isDestroyed() || this.win !== win) return;
      // The latest palette wins: a call queued behind the page load is dropped once it is stale.
      if (!samePalette(this.palette, palette)) return;
      win.webContents.executeJavaScript(code).catch(() => undefined);
    });
  }

  private send(mode: IndicatorMode, level: number): void {
    const win = this.win;
    if (!win || win.isDestroyed() || !this.loaded) return;
    const code = indicatorScript(mode, level);
    void this.loaded.then(() => {
      if (win.isDestroyed() || this.win !== win) return;
      // The latest mode wins: a call queued behind the page load is dropped once it is stale.
      if (mode !== this.mode) return;
      win.webContents.executeJavaScript(code).catch(() => undefined);
    });
  }

  private scheduleHide(): void {
    this.cancelHide();
    this.hideTimer = setTimeout(() => {
      this.hideTimer = null;
      if (this.mode === 'hidden' && this.win && !this.win.isDestroyed()) this.win.hide();
    }, INDICATOR_FADE_MS);
  }

  private cancelHide(): void {
    if (this.hideTimer) clearTimeout(this.hideTimer);
    this.hideTimer = null;
  }
}
