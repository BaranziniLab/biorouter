// @vitest-environment node
/**
 * A settings row under a keyboard and a pointer, in Chromium with the whole stylesheet.
 *
 * V4-R3-1: Tab onto an InfoTip drew its focus edge over the label's last letter (the 24px hit box
 * starts 1px before the label's end). The mark is now a 20px box inside the hit box, so focusing
 * the tip must not change one pixel of the label, in normal colours, under `prefers-contrast:
 * more` and in forced colours, at standard and Large text.
 *
 * V4-R3-5: a status line ("Restart to apply") grew the row by one line and the control, centred on
 * the grown row, slid 8px down under the pointer that had just clicked it. The value and the
 * control now sit on the label's line, so a status line moves nothing in its row.
 *
 * jsdom lays nothing out and paints nothing, so the real components are bundled with esbuild,
 * `styles/main.css` is compiled by Tailwind with the classes the bundle uses and then by the
 * renderer's own text-size plugin (`vite-plugins/fontScale.mjs`), and Chromium paints them. Each
 * group starts with an instrument case: the old rule put back, and the defect seen.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { SettingRow, SettingSection } from './setting-row';
import { Switch } from './switch';
import { Button } from './button';

function Spellcheck() {
  const [on, setOn] = React.useState(true);
  const [restart, setRestart] = React.useState(false);
  return (
    <SettingRow label="Spellcheck" help="Checks spelling as you type." data-testid="spell"
      status={restart ? 'Restart to apply' : undefined}>
      <Switch checked={on} onCheckedChange={(v) => { setOn(v); setRestart(true); }} />
    </SettingRow>
  );
}

function Version() {
  const [checking, setChecking] = React.useState(false);
  return (
    <SettingRow label="Version" help="The installed version." value="1.92.1" valueMono
      status={checking ? 'Checking for updates…' : undefined} data-testid="version">
      <Button variant="secondary" size="sm" onClick={() => setChecking(true)}>Check for updates</Button>
    </SettingRow>
  );
}

function App() {
  return (
    <div style={{ padding: 24, width: 640 }}>
      <SettingSection title="General">
        <SettingRow label="Prevent sleep while running" data-testid="sleep"
          help="Keeps your computer awake while Biorouter runs a task. The screen can still lock.">
          <Switch checked={false} onCheckedChange={() => {}} />
        </SettingRow>
        <Spellcheck />
        <Version />
        <SettingRow label="Tool call details" data-testid="below">
          <Switch checked onCheckedChange={() => {}} />
        </SettingRow>
      </SettingSection>
      {/* A hand-built row with a permanent second line (Local models): not a SettingRow, so its
          control stays centred on both lines. */}
      <div className="biorouter-settings-row br-setting-row" data-testid="hand">
        <div className="br-setting-row-text">
          <div className="br-setting-row-head">
            <span className="br-setting-row-label text-label text-text-default">Gemma 4</span>
          </div>
          <span className="text-supporting text-text-muted">9.6 GB · 128K context</span>
        </div>
        <div className="br-setting-row-control">
          <Button variant="secondary" size="sm">Install</Button>
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
`;

/** The pre-fix InfoTip focus: fill and edge on the 24px hit box itself. */
const OLD_TIP_FOCUS = `
.br-info-tip:focus-visible { background-color: var(--background-focus) !important;
  box-shadow: inset 0 0 0 2px var(--border-focus) !important; }
.br-info-tip:focus-visible::before { content: none !important; }
@media (prefers-contrast: more), (forced-colors: active) {
  button.br-info-tip.br-info-tip:focus-visible { outline-offset: 2px !important; } }`;

/** Every focus mark an InfoTip can draw, taken off (outranks the OS-ring rules). */
const NO_MARK = `
button.br-info-tip.br-info-tip:focus-visible,
button.br-info-tip.br-info-tip:focus-visible::before {
  background-color: transparent !important; box-shadow: none !important; outline: none !important; }`;

/** The pre-fix row: value and control centred on the whole row. */
const OLD_ROW_CENTRE = `
.br-setting-row[data-slot='setting-row'] > .br-setting-row-control,
.br-setting-row[data-slot='setting-row'] > .br-setting-row-value {
  align-self: center !important; min-block-size: 0 !important; margin-block-start: 0 !important; }
.br-setting-row[data-slot='setting-row'] > .br-setting-row-control { margin-block-start: -6px !important; }`;

const launchChromium = async (): Promise<Browser | null> => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      return await chromium.launch(options);
    } catch {
      // try the next build
    }
  }
  return null;
};

type Look = {
  mode?: 'light' | 'dark';
  scale?: '1' | '1.07';
  forcedColors?: boolean;
  contrastMore?: boolean;
  extraCss?: string;
};

describe('a settings row under a keyboard and a pointer', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the settings row stability tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
    );
    // The renderer's own text-size pass, so Large text is the app's Large text.
    css = (await postcss([fontScale()]).process(tailwind, { from: undefined })).css;
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(look: Look = {}): Promise<Page> {
    const tab = await browser!.newPage({
      viewport: { width: 720, height: 520 },
      deviceScaleFactor: 2,
    });
    if (look.forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    if (look.contrastMore) await tab.emulateMedia({ contrast: 'more' });
    await tab.setContent(
      `<!doctype html><html class="${look.mode === 'dark' ? 'dark' : ''}" ` +
        `style="--app-font-scale: ${look.scale ?? '1'}"><head><style>${css}</style>` +
        `<style>${look.extraCss ?? ''}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="hand"]');
    await tab.waitForTimeout(200);
    return tab;
  }

  describe('the InfoTip focus mark (V4-R3-1)', () => {
    const TIP = 'About Prevent sleep while running';

    /** The label's painted line, from its start to the end of its text, as PNG bytes. */
    async function labelPixels(tab: Page): Promise<Buffer> {
      const box = await tab.evaluate(() => {
        const label = document.querySelector('[data-testid="sleep"] label')!;
        const range = document.createRange();
        range.selectNodeContents(label);
        const text = range.getBoundingClientRect();
        return { x: text.left, y: text.top, right: text.right, height: text.height };
      });
      return tab.screenshot({
        clip: { x: box.x, y: box.y, width: Math.floor(box.right) - box.x, height: box.height },
      });
    }

    async function tabTo(tab: Page, name: string) {
      for (let i = 0; i < 12; i++) {
        await tab.keyboard.press('Tab');
        const current = await tab.evaluate(() =>
          document.activeElement?.getAttribute('aria-label')
        );
        if (current === name) return;
      }
      throw new Error(`Tab never reached ${name}`);
    }

    /**
     * Whether the focus MARK paints on the label and on the tip: the focused row is read once
     * with its mark and once with every mark taken off (the row's own focus wash and the open
     * tooltip are in both), so only the mark can differ.
     */
    async function markTouches(look: Look): Promise<{ label: boolean; tip: boolean }> {
      const tab = await open(look);
      try {
        const tipBox = async () =>
          tab.screenshot({
            clip: await tab.evaluate((name) => {
              const r = document.querySelector(`[aria-label="${name}"]`)!.getBoundingClientRect();
              return { x: r.left, y: r.top, width: r.width, height: r.height };
            }, TIP),
          });
        await tabTo(tab, TIP);
        await tab.waitForTimeout(400);
        const markedLabel = await labelPixels(tab);
        const markedTip = await tipBox();
        await tab.addStyleTag({ content: NO_MARK });
        await tab.waitForTimeout(50);
        const bareLabel = await labelPixels(tab);
        const bareTip = await tipBox();
        return { label: !markedLabel.equals(bareLabel), tip: !markedTip.equals(bareTip) };
      } finally {
        await tab.close();
      }
    }

    it('instrument: with the old mark on the hit box, focus paints over the label', async (ctx) => {
      if (!browser) return ctx.skip();
      for (const look of [
        { extraCss: OLD_TIP_FOCUS },
        { extraCss: OLD_TIP_FOCUS, forcedColors: true },
      ] as Look[]) {
        expect(await markTouches(look), JSON.stringify(look)).toEqual({
          label: true,
          tip: true,
        });
      }
    });

    it('marks the tip and leaves every pixel of the label alone', async (ctx) => {
      if (!browser) return ctx.skip();
      const looks: Look[] = [
        { mode: 'light' },
        { mode: 'dark' },
        { mode: 'light', scale: '1.07' },
        { mode: 'dark', scale: '1.07' },
        { mode: 'light', contrastMore: true },
        { mode: 'light', forcedColors: true },
        { mode: 'dark', forcedColors: true },
      ];
      for (const look of looks) {
        // A mark is drawn on the tip, and none of it on the label.
        expect(await markTouches(look), JSON.stringify(look)).toEqual({
          label: false,
          tip: true,
        });
      }
    });

    it('draws the mark on a 20px box past the label, and keeps the glyph 4px after it', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open();
      try {
        await tabTo(tab, TIP);
        const m = await tab.evaluate((name) => {
          const tip = document.querySelector(`[aria-label="${name}"]`)!;
          const label = tip.parentElement!.querySelector('label')!;
          const range = document.createRange();
          range.selectNodeContents(label);
          const textEnd = range.getBoundingClientRect().right;
          const box = tip.getBoundingClientRect();
          const glyph = tip.querySelector('svg')!.getBoundingClientRect();
          const mark = getComputedStyle(tip, '::before');
          const own = getComputedStyle(tip);
          return {
            glyphGap: glyph.left - textEnd,
            hitBox: [box.width, box.height],
            markStart: box.left + parseFloat(mark.left) - textEnd,
            markSize: [mark.width, mark.height],
            markEdge: mark.boxShadow,
            ownShadow: own.boxShadow,
            ownFill: own.backgroundColor,
          };
        }, TIP);
        expect(m.glyphGap).toBeCloseTo(4, 1);
        expect(m.hitBox).toEqual([24, 24]);
        expect(m.markStart).toBeGreaterThanOrEqual(1);
        expect(m.markSize).toEqual(['20px', '20px']);
        expect(m.markEdge).toMatch(/inset/);
        expect(m.ownShadow).toBe('none');
        expect(m.ownFill).toBe('rgba(0, 0, 0, 0)');
      } finally {
        await tab.close();
      }
    });
  });

  describe('a status line in a settings row (V4-R3-5)', () => {
    type Rect = { top: number; height: number; mid: number };
    const rect = (tab: Page, selector: string) =>
      tab.evaluate((s) => {
        const r = document.querySelector(s)!.getBoundingClientRect();
        return { top: r.top, height: r.height, mid: r.top + r.height / 2 } as Rect;
      }, selector);

    /** Click the control by pointer, the way a person does, and read what moved. */
    async function clickAndMeasure(look: Look) {
      const tab = await open(look);
      try {
        const parts = {
          spellRow: '[data-testid="spell"]',
          spellSwitch: '[data-testid="spell"] [role="switch"]',
          versionRow: '[data-testid="version"]',
          versionValue: '[data-testid="version"] .br-setting-row-value',
          versionButton: '[data-testid="version"] .br-setting-row-control button',
          below: '[data-testid="below"]',
        };
        const read = async () =>
          Object.fromEntries(
            await Promise.all(
              Object.entries(parts).map(async ([k, s]) => [k, await rect(tab, s)] as const)
            )
          ) as Record<keyof typeof parts, Rect>;
        const before = await read();
        const s = before.spellSwitch;
        const switchBox = await tab.locator(parts.spellSwitch).boundingBox();
        await tab.mouse.click(switchBox!.x + switchBox!.width / 2, s.mid);
        await tab.waitForSelector('text=Restart to apply');
        const afterSpell = await read();
        const buttonBox = await tab.locator(parts.versionButton).boundingBox();
        await tab.mouse.click(
          buttonBox!.x + buttonBox!.width / 2,
          buttonBox!.y + buttonBox!.height / 2
        );
        await tab.waitForSelector('text=Checking for updates…');
        const afterVersion = await read();
        return { before, afterSpell, afterVersion };
      } finally {
        await tab.close();
      }
    }

    it('instrument: centred on the grown row, the clicked switch slides down', async (ctx) => {
      if (!browser) return ctx.skip();
      const { before, afterSpell } = await clickAndMeasure({ extraCss: OLD_ROW_CENTRE });
      expect(afterSpell.spellSwitch.top - before.spellSwitch.top).toBeGreaterThanOrEqual(7);
    });

    it('keeps the clicked control, the value and the row where they were; only the space below grows', async (ctx) => {
      if (!browser) return ctx.skip();
      for (const look of [
        { mode: 'light', scale: '1' },
        { mode: 'light', scale: '1.07' },
        { mode: 'dark', scale: '1.07' },
      ] as Look[]) {
        const { before, afterSpell, afterVersion } = await clickAndMeasure(look);
        const tag = JSON.stringify(look);
        // The switch just clicked does not move, nor does its row's top.
        expect(afterSpell.spellSwitch.top, tag).toBeCloseTo(before.spellSwitch.top, 2);
        expect(afterSpell.spellRow.top, tag).toBeCloseTo(before.spellRow.top, 2);
        // The row grew by the status line, downward; the rows below moved by exactly that.
        const grew = afterSpell.spellRow.height - before.spellRow.height;
        expect(grew, tag).toBeGreaterThan(10);
        expect(afterSpell.below.top - before.below.top, tag).toBeCloseTo(grew, 2);
        // Version: its value and button stay put within the row while its status appears.
        const shift = afterSpell.versionRow.top;
        expect(afterVersion.versionRow.top, tag).toBeCloseTo(shift, 2);
        expect(afterVersion.versionButton.top - shift, tag).toBeCloseTo(
          before.versionButton.top - before.versionRow.top,
          2
        );
        expect(afterVersion.versionValue.top - shift, tag).toBeCloseTo(
          before.versionValue.top - before.versionRow.top,
          2
        );
      }
    });

    it('draws a one-line row as before: control and value on the row’s centre', async (ctx) => {
      if (!browser) return ctx.skip();
      for (const scale of ['1', '1.07'] as const) {
        const tab = await open({ scale });
        try {
          const row = await rect(tab, '[data-testid="sleep"]');
          const control = await rect(tab, '[data-testid="sleep"] [role="switch"]');
          const version = await rect(tab, '[data-testid="version"]');
          const value = await rect(tab, '[data-testid="version"] .br-setting-row-value');
          const button = await rect(tab, '[data-testid="version"] .br-setting-row-control button');
          // 40px at standard text, the 40px row scaled at Large; padding 10/9 plus the 1px edge.
          expect(row.height).toBeCloseTo(scale === '1' ? 40 : 42.8, 1);
          // The 1px edge room is part of the bottom 10px, so the line sits on the row's centre.
          expect(control.mid - row.mid, scale).toBeCloseTo(0, 1);
          expect(button.mid - version.mid, scale).toBeCloseTo(0, 1);
          expect(value.mid - button.mid, scale).toBeCloseTo(0, 1);
        } finally {
          await tab.close();
        }
      }
    });

    it('leaves a hand-built row with a permanent second line centred', async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open();
      try {
        const row = await rect(tab, '[data-testid="hand"]');
        const button = await rect(tab, '[data-testid="hand"] .br-setting-row-control button');
        expect(row.height).toBeGreaterThan(50);
        expect(button.mid - row.mid).toBeCloseTo(0, 1);
      } finally {
        await tab.close();
      }
    });
  });
});
