import { runInNewContext } from 'node:vm';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from '../styles/themes.generated';
import {
  checkIndicatorPalette,
  DEFAULT_INDICATOR_PALETTE,
  DictationIndicator,
  INDICATOR_FADE_MS,
  INDICATOR_LEVEL_INTERVAL_MS,
  INDICATOR_SIZE,
  indicatorBounds,
  indicatorModeFor,
  indicatorPageHtml,
  indicatorPalette,
  indicatorScript,
  indicatorThemeScript,
  parseIndicatorReport,
  type IndicatorPalette,
  type IndicatorWindow,
} from './dictationIndicator';

const last = <T>(items: T[]): T | undefined => items[items.length - 1];

describe('what the indicator shows', () => {
  it('reads a renderer report and refuses anything else', () => {
    expect(parseIndicatorReport({ activity: 'recording', level: 0.4 })).toEqual({
      activity: 'recording',
      level: 0.4,
    });
    expect(parseIndicatorReport({ activity: 'transcribing', level: 7 })).toEqual({
      activity: 'transcribing',
      level: 1,
    });
    expect(parseIndicatorReport({ activity: 'idle', level: Number.NaN })).toEqual({
      activity: 'idle',
      level: 0,
    });
    for (const raw of [null, 'recording', {}, { activity: 'error' }, { activity: 'speaking' }]) {
      expect(parseIndicatorReport(raw)).toBeNull();
    }
  });

  it('shows only while a dictation records or finishes, never while it arms', () => {
    expect(indicatorModeFor([])).toEqual({ mode: 'hidden', level: 0 });
    expect(indicatorModeFor([{ activity: 'idle', level: 0 }])).toEqual({
      mode: 'hidden',
      level: 0,
    });
    expect(indicatorModeFor([{ activity: 'arming', level: 0 }]).mode).toBe('hidden');
    expect(indicatorModeFor([{ activity: 'transcribing', level: 0 }]).mode).toBe('finishing');
    expect(
      indicatorModeFor([
        { activity: 'transcribing', level: 0 },
        { activity: 'recording', level: 0.3 },
        { activity: 'recording', level: 0.6 },
      ])
    ).toEqual({ mode: 'recording', level: 0.6 });
  });

  it('sits at the bottom center of the work area, above the Dock', () => {
    const bounds = indicatorBounds({ x: 0, y: 25, width: 1512, height: 870 });
    expect(bounds.width).toBe(INDICATOR_SIZE.width);
    expect(bounds.height).toBe(INDICATOR_SIZE.height);
    expect(bounds.x + bounds.width / 2).toBe(756);
    expect(bounds.y + bounds.height).toBeLessThan(25 + 870);
    expect(bounds.y + bounds.height).toBeGreaterThan(25 + 870 - 20);
    // A second display to the left, with a negative origin.
    const left = indicatorBounds({ x: -1920, y: 0, width: 1920, height: 1080 });
    expect(left.x + left.width / 2).toBe(-960);
  });

  it('the script main runs in the page carries only a known mode and a clamped number', () => {
    expect(indicatorScript('recording', 0.5)).toBe(
      'window.__brIndicator && window.__brIndicator("recording", 0.500)'
    );
    expect(indicatorScript('finishing', 4)).toContain('"finishing", 1.000');
    expect(indicatorScript('hidden', Number.NaN)).toContain('"hidden", 0.000');
    expect(indicatorScript('</script>' as never, 0)).toContain('"hidden"');
  });

  it('the page locks itself down and draws bars, a pulse, or one still dot', () => {
    const html = indicatorPageHtml(indicatorPalette('parchment', 'light'));
    expect(html).toContain(
      `content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'"`
    );
    expect(html.match(/class="bar"/g)).toHaveLength(4);
    expect(html).toContain('prefers-reduced-motion: reduce');
    expect(html).toContain('body.still .dot{display:block}');
    expect(html).not.toMatch(/https?:/);
  });

  // Reduce motion: the finishing dot used to fade to half opacity, which measures 2.29 to
  // 2.72:1 on the accent fill. It keeps full ink and says "finishing" by its shape instead.
  it('the Reduce motion finishing dot keeps full ink and changes shape', () => {
    const html = indicatorPageHtml(indicatorPalette('alma-mater', 'light'));
    expect(html).toContain(
      'body.still.finishing .dot{background:transparent;box-shadow:inset 0 0 0 1.5px var(--br-ink)}'
    );
    const dotRules = html.match(/[^{}\n]*\.dot[^{}]*\{[^}]*\}/g) ?? [];
    expect(dotRules.length).toBeGreaterThan(0);
    for (const rule of dotRules) expect(rule, rule).not.toMatch(/opacity/);
    expect(html).toMatch(/\.dot\{[^}]*background:var\(--br-ink\)/);
  });

  it('the page script sets the classes and bar heights main asks for', () => {
    const script = indicatorPageHtml().match(/<script>([\s\S]*)<\/script>/)![1];
    const classes = new Set<string>();
    const bars = Array.from({ length: 4 }, () => ({ style: { height: '' } }));
    const context = {
      window: {} as Record<string, (mode: string, level: number) => void>,
      document: {
        body: {
          classList: {
            toggle: (name: string, on: boolean) => (on ? classes.add(name) : classes.delete(name)),
          },
        },
        documentElement: { style: { setProperty: () => undefined } },
        querySelectorAll: () => bars,
      },
      matchMedia: () => ({ matches: false, addEventListener: () => undefined }),
      Array,
      Math,
    };
    runInNewContext(script, context);
    context.window.__brIndicator('recording', 1);
    expect([...classes]).toEqual(['on']);
    for (const bar of bars) expect(parseInt(bar.style.height, 10)).toBeGreaterThan(2);
    context.window.__brIndicator('finishing', 0);
    expect(classes.has('finishing')).toBe(true);
    for (const bar of bars) expect(bar.style.height).toBe('');
    context.window.__brIndicator('hidden', 0);
    expect(classes.has('on')).toBe(false);
  });

  it('the page script repaints the three colours main sends, and nothing else', () => {
    const script = indicatorPageHtml().match(/<script>([\s\S]*)<\/script>/)![1];
    const props = new Map<string, string>();
    const context = {
      window: {} as Record<string, (...args: string[]) => void>,
      document: {
        body: { classList: { toggle: () => undefined } },
        documentElement: {
          style: { setProperty: (name: string, value: string) => props.set(name, value) },
        },
        querySelectorAll: () => [],
      },
      matchMedia: () => ({ matches: false, addEventListener: () => undefined }),
      Array,
      Math,
    };
    runInNewContext(script, context);
    const palette = indicatorPalette('roche-limit', 'dark');
    const call = indicatorThemeScript(palette)!;
    runInNewContext(call, context);
    expect(Object.fromEntries(props)).toEqual({
      '--br-fill': palette.fill,
      '--br-ink': palette.ink,
      '--br-shadow': palette.shadow,
    });
  });
});

/** WCAG relative luminance contrast of two 6-digit hexes. */
function contrast(a: string, b: string): number {
  const lum = (hex: string) => {
    const [r, g, bl] = [1, 3, 5].map((i) => {
      const c = parseInt(hex.slice(i, i + 2), 16) / 255;
      return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [x, y] = [lum(a), lum(b)].sort((m, n) => n - m);
  return (x + 0.05) / (y + 0.05);
}

const MODES = ['light', 'dark'] as const;
const COLOUR_LITERAL =
  /#[0-9a-f]{3,8}\b|\brgba?\(|\bhsla?\(|\bhwb\(|\blab\(|\blch\(|oklch\(|oklab\(|color-mix\(|\bcolor\(/gi;
const NAMED_COLOUR =
  /\b(?:red|white|black|green|crimson|tomato|orange|grey|gray|darkred|firebrick)\b/i;

describe('the indicator wears the armed Send button’s colours (DD-99)', () => {
  it.each(THEME_FAMILY_IDS.flatMap((family) => MODES.map((mode) => [family, mode] as const)))(
    '%s %s: the palette is the generated accent, on-accent ink and shadow',
    (family, mode) => {
      const surface = GENERATED_THEMES[family][mode].surface;
      expect(indicatorPalette(family, mode)).toEqual({
        fill: surface.accent,
        ink: surface.onAccent,
        shadow: surface.shadow,
      });
      expect(surface.accent).toMatch(/^#[0-9a-f]{6}$/i);
      expect(surface.onAccent).toMatch(/^#[0-9a-f]{6}$/i);
      // The bars and the dot are graphics: 3:1 at least on the fill, in every family.
      expect(contrast(surface.onAccent, surface.accent)).toBeGreaterThanOrEqual(3);
    }
  );

  it('the families differ, so the bubble really follows the family and the mode', () => {
    const fills = new Set(
      THEME_FAMILY_IDS.flatMap((family) => MODES.map((mode) => indicatorPalette(family, mode).fill))
    );
    expect(fills.size).toBeGreaterThan(1);
    expect(indicatorPalette('parchment', 'light').fill).not.toBe(
      indicatorPalette('parchment', 'dark').fill
    );
  });

  it('an unknown family, or a mode that is not one, gives Parchment', () => {
    for (const mode of MODES) {
      expect(indicatorPalette('neon', mode)).toEqual(indicatorPalette('parchment', mode));
      expect(indicatorPalette('__proto__', mode)).toEqual(indicatorPalette('parchment', mode));
      expect(indicatorPalette('constructor', mode)).toEqual(indicatorPalette('parchment', mode));
      expect(indicatorPalette('', mode)).toEqual(indicatorPalette('parchment', mode));
    }
    expect(indicatorPalette('alma-mater', 'dim' as never)).toEqual(
      indicatorPalette('alma-mater', 'light')
    );
    expect(DEFAULT_INDICATOR_PALETTE).toEqual(indicatorPalette('parchment', 'light'));
  });

  it.each(THEME_FAMILY_IDS.flatMap((family) => MODES.map((mode) => [family, mode] as const)))(
    '%s %s: the page carries no colour but the palette’s, and an opaque fill',
    (family, mode) => {
      const palette = indicatorPalette(family, mode);
      const html = indicatorPageHtml(palette);
      expect(html).toContain(
        `:root{--br-fill:${palette.fill};--br-ink:${palette.ink};--br-shadow:${palette.shadow}}`
      );
      const rest = html
        .replace(`--br-fill:${palette.fill}`, '')
        .replace(`--br-ink:${palette.ink}`, '')
        .replace(`--br-shadow:${palette.shadow}`, '');
      expect(rest.match(COLOUR_LITERAL)).toBeNull();
      expect(rest).not.toMatch(NAMED_COLOUR);
      // Never red: no danger or critical tone, and no state with a colour of its own.
      expect(html).not.toMatch(/danger|critical|error/i);
      // Opaque: a 6-digit fill and no opacity on the pill other than the fade in and out.
      expect(palette.fill).toMatch(/^#[0-9a-f]{6}$/i);
      expect(html).toMatch(
        /\.pill\{[^}]*background:var\(--br-fill\);box-shadow:var\(--br-shadow\)/
      );
      expect(html).toMatch(/\.bar\{[^}]*background:var\(--br-ink\)/);
    }
  );

  it('nothing transitions a colour: a theme change is a jump', () => {
    const html = indicatorPageHtml(indicatorPalette('parchment', 'dark'));
    const transitions = html.match(/transition:[^;}]*/g) ?? [];
    expect(transitions.length).toBeGreaterThan(0);
    for (const transition of transitions) {
      if (transition === 'transition:none') continue;
      for (const part of transition.slice('transition:'.length).split(',')) {
        expect(['opacity', 'transform', 'height'], transition).toContain(part.trim().split(' ')[0]);
      }
    }
  });

  it('builds the theme call from checked values only, and refuses anything else', () => {
    const palette = indicatorPalette('alma-mater', 'dark');
    expect(indicatorThemeScript(palette)).toBe(
      `window.__brIndicatorTheme && window.__brIndicatorTheme(${JSON.stringify(palette.fill)}, ${JSON.stringify(palette.ink)}, ${JSON.stringify(palette.shadow)})`
    );
    expect(indicatorThemeScript({ ...palette, shadow: 'none' })).toContain('"none"');
    const bad: unknown[] = [
      { ...palette, fill: 'red' },
      { ...palette, fill: '#fff' },
      { ...palette, fill: '#b85a3280' },
      { ...palette, fill: 'rgba(0,0,0,.5)' },
      { ...palette, fill: 'var(--background-danger)' },
      { ...palette, fill: '#b85a32"); alert(1); ("' },
      { ...palette, ink: 'white' },
      { ...palette, ink: ' #ffffff' },
      { ...palette, ink: 16777215 },
      { ...palette, shadow: 'var(--shadow-default)' },
      { ...palette, shadow: '0 0 1px red' },
      { ...palette, shadow: '0 0 1px rgba(0,0,0,.2)}body{background:url(x)' },
      { ...palette, shadow: '0 0 1px rgba(0,0,0,.2);--br-fill:#ff0000' },
      { ...palette, shadow: `${'0 0 1px rgba(0,0,0,.2), '.repeat(20)}0 0 1px rgba(0,0,0,.2)` },
      { fill: palette.fill, ink: palette.ink },
      null,
      'alma-mater',
    ];
    for (const raw of bad) {
      expect(checkIndicatorPalette(raw), JSON.stringify(raw)).toBeNull();
      expect(indicatorThemeScript(raw as IndicatorPalette), JSON.stringify(raw)).toBeNull();
    }
  });

  it('a page asked for a palette that fails the checks is built with the default one', () => {
    const html = indicatorPageHtml({ fill: 'red', ink: '#ffffff', shadow: 'none' });
    expect(html).toContain(`--br-fill:${DEFAULT_INDICATOR_PALETTE.fill}`);
    expect(html).not.toMatch(/\bred\b/);
  });
});

class FakeWindow implements IndicatorWindow {
  destroyed = false;
  shown = false;
  shows = 0;
  bounds: unknown = null;
  url = '';
  scripts: string[] = [];
  isDestroyed() {
    return this.destroyed;
  }
  setBounds(bounds: unknown) {
    this.bounds = bounds;
  }
  showInactive() {
    this.shown = true;
    this.shows += 1;
  }
  hide() {
    this.shown = false;
  }
  destroy() {
    this.destroyed = true;
  }
  loadURL(url: string) {
    this.url = url;
    return Promise.resolve();
  }
  webContents = {
    executeJavaScript: (code: string) => {
      this.scripts.push(code);
      return Promise.resolve();
    },
  };
}

describe('the indicator’s window', () => {
  let now = 0;
  let windows: FakeWindow[];
  beforeEach(() => {
    vi.useFakeTimers();
    now = 0;
    windows = [];
  });
  afterEach(() => vi.useRealTimers());

  const indicator = () =>
    new DictationIndicator({
      createWindow: () => {
        const win = new FakeWindow();
        windows.push(win);
        return win;
      },
      workArea: () => ({ x: 0, y: 0, width: 1000, height: 800 }),
      now: () => now,
    });
  const flush = async () => {
    await Promise.resolve();
    await Promise.resolve();
  };

  it('opens nothing until a dictation records', () => {
    const shown = indicator();
    shown.update('hidden');
    expect(windows).toHaveLength(0);
  });

  it('appears without taking focus, at the bottom center, and follows the level', async () => {
    const shown = indicator();
    shown.update('recording', 0.2);
    await flush();
    expect(windows).toHaveLength(1);
    const win = windows[0];
    expect(win.shown).toBe(true);
    expect(win.bounds).toEqual(indicatorBounds({ x: 0, y: 0, width: 1000, height: 800 }));
    expect(win.url.startsWith('data:text/html;charset=utf-8,')).toBe(true);
    expect(last(win.scripts)).toContain('"recording", 0.200');
    // Level updates faster than the redraw interval are skipped.
    now += INDICATOR_LEVEL_INTERVAL_MS - 1;
    shown.update('recording', 0.9);
    await flush();
    expect(win.scripts).toHaveLength(1);
    now += 1;
    shown.update('recording', 0.9);
    await flush();
    expect(last(win.scripts)).toContain('"recording", 0.900');
  });

  it('settles while the text is finished, then fades and hides, and is reused next time', async () => {
    const shown = indicator();
    shown.update('recording', 0.5);
    shown.update('finishing');
    await flush();
    const win = windows[0];
    expect(last(win.scripts)).toContain('"finishing"');
    shown.update('hidden');
    await flush();
    expect(last(win.scripts)).toContain('"hidden"');
    expect(win.shown).toBe(true);
    vi.advanceTimersByTime(INDICATOR_FADE_MS);
    expect(win.shown).toBe(false);
    expect(shown.shownMode).toBe('hidden');
    shown.update('recording', 0.1);
    await flush();
    expect(windows).toHaveLength(1);
    expect(win.shown).toBe(true);
    expect(win.shows).toBe(2);
  });

  it('a new recording during the fade keeps the window', async () => {
    const shown = indicator();
    shown.update('recording', 0.5);
    shown.update('hidden');
    vi.advanceTimersByTime(INDICATOR_FADE_MS - 10);
    shown.update('recording', 0.5);
    vi.advanceTimersByTime(INDICATOR_FADE_MS);
    await flush();
    expect(windows[0].shown).toBe(true);
    expect(last(windows[0].scripts)).toContain('"recording"');
  });

  it('opens a new window when the old one was closed, and dispose destroys it', async () => {
    const shown = indicator();
    shown.update('recording', 0.5);
    windows[0].destroyed = true;
    shown.update('hidden');
    shown.update('recording', 0.5);
    expect(windows).toHaveLength(2);
    shown.dispose();
    expect(windows[1].destroyed).toBe(true);
  });

  const pageOf = (win: FakeWindow) =>
    decodeURIComponent(win.url.slice('data:text/html;charset=utf-8,'.length));
  const themeCalls = (win: FakeWindow) =>
    win.scripts.filter((code) => code.startsWith('window.__brIndicatorTheme'));

  it('builds its page with main’s palette, so the first frame is already right', async () => {
    const palette = indicatorPalette('alma-mater', 'dark');
    const shown = new DictationIndicator({
      createWindow: () => {
        const win = new FakeWindow();
        windows.push(win);
        return win;
      },
      workArea: () => ({ x: 0, y: 0, width: 1000, height: 800 }),
      palette: () => palette,
      now: () => now,
    });
    shown.update('recording', 0.3);
    await flush();
    expect(pageOf(windows[0])).toContain(`--br-fill:${palette.fill};--br-ink:${palette.ink}`);
    expect(themeCalls(windows[0])).toEqual([]);
    expect(shown.currentPalette).toEqual(palette);
  });

  it('without a palette from main, starts in Parchment light; a failing getter changes nothing', async () => {
    const shown = indicator();
    shown.update('recording', 0.3);
    await flush();
    expect(pageOf(windows[0])).toContain(`--br-fill:${DEFAULT_INDICATOR_PALETTE.fill}`);
    const broken = new DictationIndicator({
      createWindow: () => {
        const win = new FakeWindow();
        windows.push(win);
        return win;
      },
      workArea: () => ({ x: 0, y: 0, width: 1000, height: 800 }),
      palette: () => {
        throw new Error('no appearance yet');
      },
    });
    expect(() => broken.update('recording', 0.3)).not.toThrow();
    expect(pageOf(windows[1])).toContain(`--br-fill:${DEFAULT_INDICATOR_PALETTE.fill}`);
  });

  it('setPalette while it shows repaints it once after the page loads', async () => {
    const shown = indicator();
    shown.update('recording', 0.3);
    await flush();
    const win = windows[0];
    const next = indicatorPalette('roche-limit', 'dark');
    shown.setPalette(next);
    shown.setPalette(next);
    await flush();
    expect(themeCalls(win)).toEqual([indicatorThemeScript(next)]);
    expect(win.shows).toBe(1);
    expect(windows).toHaveLength(1);
  });

  it('setPalette before the page loads sends only the latest palette, once it has', async () => {
    let finishLoad: () => void = () => undefined;
    const slow = new FakeWindow();
    slow.loadURL = (url: string) => {
      slow.url = url;
      return new Promise<void>((resolve) => {
        finishLoad = resolve;
      });
    };
    const shown = new DictationIndicator({
      createWindow: () => {
        windows.push(slow);
        return slow;
      },
      workArea: () => ({ x: 0, y: 0, width: 1000, height: 800 }),
      now: () => now,
    });
    shown.update('recording', 0.3);
    shown.setPalette(indicatorPalette('alma-mater', 'light'));
    shown.setPalette(indicatorPalette('alma-mater', 'dark'));
    await flush();
    expect(slow.scripts).toEqual([]);
    finishLoad();
    await flush();
    expect(themeCalls(slow)).toEqual([
      indicatorThemeScript(indicatorPalette('alma-mater', 'dark')),
    ]);
  });

  it('setPalette repaints the hidden window kept for reuse, and never shows it', async () => {
    const shown = indicator();
    shown.update('recording', 0.3);
    shown.update('hidden');
    vi.advanceTimersByTime(INDICATOR_FADE_MS);
    await flush();
    const win = windows[0];
    expect(win.shown).toBe(false);
    const next = indicatorPalette('alma-mater', 'dark');
    shown.setPalette(next);
    await flush();
    expect(themeCalls(win)).toEqual([indicatorThemeScript(next)]);
    expect(win.shown).toBe(false);
    expect(win.shows).toBe(1);
    // The next dictation shows it in the new colours with no second call.
    shown.update('recording', 0.3);
    await flush();
    expect(win.shown).toBe(true);
    expect(themeCalls(win)).toHaveLength(1);
  });

  it('setPalette with no window builds none, and the next window wears it', async () => {
    const shown = indicator();
    const next = indicatorPalette('roche-limit', 'light');
    shown.setPalette(next);
    expect(windows).toHaveLength(0);
    shown.update('recording', 0.3);
    await flush();
    expect(pageOf(windows[0])).toContain(`--br-fill:${next.fill};--br-ink:${next.ink}`);
    expect(themeCalls(windows[0])).toEqual([]);
  });

  it('setPalette ignores a palette that fails the checks', async () => {
    const shown = indicator();
    shown.update('recording', 0.3);
    await flush();
    const before = shown.currentPalette;
    shown.setPalette({ fill: '#ff0000aa', ink: '#ffffff', shadow: 'none' });
    shown.setPalette({ fill: 'red', ink: 'white', shadow: 'none' } as IndicatorPalette);
    await flush();
    expect(themeCalls(windows[0])).toEqual([]);
    expect(shown.currentPalette).toEqual(before);
  });

  it('a reused window takes main’s palette when it appears, if a change was missed', async () => {
    let current = indicatorPalette('parchment', 'light');
    const shown = new DictationIndicator({
      createWindow: () => {
        const win = new FakeWindow();
        windows.push(win);
        return win;
      },
      workArea: () => ({ x: 0, y: 0, width: 1000, height: 800 }),
      palette: () => current,
      now: () => now,
    });
    shown.update('recording', 0.3);
    shown.update('hidden');
    vi.advanceTimersByTime(INDICATOR_FADE_MS);
    await flush();
    current = indicatorPalette('alma-mater', 'dark');
    shown.update('recording', 0.3);
    await flush();
    expect(themeCalls(windows[0])).toEqual([indicatorThemeScript(current)]);
    expect(windows).toHaveLength(1);
  });
});
