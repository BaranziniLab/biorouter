/**
 * The on-screen dictation indicator: a small dark capsule at the bottom center of the active
 * display, shown only while a dictation records or finishes, in Biorouter and in other apps.
 *
 * - While recording, four thin bars follow the microphone's level (the composer's level, or the
 *   hidden Dictate to clipboard page's), so a person can see that sound is arriving.
 * - While the text is being finished, the bars settle into a slow pulse.
 * - Under Reduce motion, one still dot stands in for both.
 *
 * It is a transparent, frameless window that never takes focus, ignores the mouse (clicks go to
 * whatever is under it), stays out of the Dock and Mission Control, and floats over every Space
 * and full-screen app. It is created on the first dictation and then hidden, not closed, between
 * dictations. Its page is a fixed string with no preload and no Node; main updates it with one
 * call built from two validated values.
 */

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

/** The indicator's page. Fixed text: nothing in it comes from a person, a model or a file. */
export function indicatorPageHtml(): string {
  return `<!doctype html>
<html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'">
<style>
html,body{margin:0;height:100%;background:transparent;overflow:hidden;user-select:none;cursor:default}
body{display:flex;align-items:center;justify-content:center}
.pill{display:flex;align-items:center;justify-content:center;gap:3px;width:42px;height:16px;border-radius:8px;
background:rgba(22,22,21,.86);box-shadow:0 0 0 .5px rgba(255,255,255,.2),0 1px 6px rgba(0,0,0,.28);
opacity:0;transform:translateY(2px) scale(.94);transition:opacity .16s ease,transform .16s ease}
body.on .pill{opacity:1;transform:none}
.bar{display:block;width:2px;height:2px;border-radius:1px;background:rgba(255,255,255,.9);transition:height .1s linear}
body.finishing .bar{height:2px;animation:settle 1.2s ease-in-out infinite}
body.finishing .bar:nth-child(2){animation-delay:.15s}
body.finishing .bar:nth-child(3){animation-delay:.3s}
body.finishing .bar:nth-child(4){animation-delay:.45s}
@keyframes settle{0%,100%{opacity:.35}50%{opacity:.95}}
.dot{display:none;width:5px;height:5px;border-radius:50%;background:rgba(255,255,255,.9)}
body.still .pill{transition:none;transform:none}
body.still .bar{display:none}
body.still .dot{display:block}
body.still.finishing .dot{opacity:.5}
</style></head>
<body><div class="pill" aria-hidden="true"><i class="bar"></i><i class="bar"></i><i class="bar"></i><i class="bar"></i><span class="dot"></span></div>
<script>
(() => {
  const body = document.body;
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
})();
</script></body></html>`;
}

/** The one call main makes into the page. Both values are checked before they are written. */
export function indicatorScript(mode: IndicatorMode, level: number): string {
  const safeMode: IndicatorMode = mode === 'recording' || mode === 'finishing' ? mode : 'hidden';
  const safeLevel = Number.isFinite(level) ? Math.max(0, Math.min(1, level)) : 0;
  return `window.__brIndicator && window.__brIndicator(${JSON.stringify(safeMode)}, ${safeLevel.toFixed(3)})`;
}

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
  private lastLevelAt = -Infinity;
  private hideTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly now: () => number;

  constructor(private readonly options: DictationIndicatorOptions) {
    this.now = options.now ?? (() => Date.now());
  }

  get shownMode(): IndicatorMode {
    return this.mode;
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
    const win = this.ensureWindow();
    if (!win) return;
    if (appearing) {
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
    this.loaded = this.win
      .loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(indicatorPageHtml())}`)
      .catch(() => undefined);
    return this.win;
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
