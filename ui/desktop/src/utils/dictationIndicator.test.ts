import { runInNewContext } from 'node:vm';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DictationIndicator,
  INDICATOR_FADE_MS,
  INDICATOR_LEVEL_INTERVAL_MS,
  INDICATOR_SIZE,
  indicatorBounds,
  indicatorModeFor,
  indicatorPageHtml,
  indicatorScript,
  parseIndicatorReport,
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
    const html = indicatorPageHtml();
    expect(html).toContain(
      `content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'"`
    );
    expect(html.match(/class="bar"/g)).toHaveLength(4);
    expect(html).toContain('prefers-reduced-motion: reduce');
    expect(html).toContain('body.still .dot{display:block}');
    expect(html).not.toMatch(/https?:/);
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
});
