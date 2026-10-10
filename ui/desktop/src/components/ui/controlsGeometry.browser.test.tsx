// @vitest-environment node
/**
 * The shared controls measured in a real layout engine (spec 2.6 and 5.7).
 *
 * jsdom lays nothing out, runs no CSS and fires no ResizeObserver, so the component tests can only
 * pin class names and rules at the source. The acceptance numbers are geometry: the Switch is
 * 32×20 with a 16px knob inset 2px in both states, a menu row is 32px, the segmented thumb sits
 * under the selected segment and its first placement does not animate, and an InfoTip adds 14px
 * to the line. So the real components are bundled with esbuild, rendered by React in Chromium,
 * and styled by the authored `@ws WS-PRIMITIVES` blocks of `styles/main.css` with the token
 * values read from that file.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

const TOKEN_NAMES = [
  '--control-md',
  '--control-sm',
  '--radius-full',
  '--radius-element',
  '--radius-inner',
  '--text-secondary',
  '--text-secondary--line-height',
  '--text-label',
  '--text-label--line-height',
  '--dur-fast-min',
  '--dur-fast',
  '--dur-fast-max',
  '--ease-out',
  '--ease-spring',
  '--row-height',
  '--radius-md',
];

/** The first declaration of each token in main.css, so a token change reaches this test. */
function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

/** Every `@ws WS-PRIMITIVES` block of main.css, in file order. */
function primitiveBlocks(): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/** Tailwind's preflight, the parts the layout depends on, plus colours that only paint. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
svg { display: block; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --background-muted: #f2f2f0; --border-focus: #555; --background-focus: #ddd;
  --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1); --inset-hairline: inset 0 0 0 1px #ddd;
  --overlay-hover: rgb(0 0 0 / 0.05); --border-subtle: #ccc; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Switch } from './switch';
import { SegmentedControl } from './segmented-control';
import { InfoTip } from './info-tip';
import { MENU_ROW_CLASS_NAME, MENU_ROW_TRAILING_CLASS_NAME } from './dropdown-menu';
import { Tabs, TabsList, TabsTrigger } from './tabs';
import { SettingRow, SettingSection } from './setting-row';

const OPTIONS = [
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
  { value: 'system', label: 'System' },
];

function App() {
  const [value, setValue] = React.useState('dark');
  (window as unknown as { setSegmented: (v: string) => void }).setSegmented = setValue;
  return (
    <div>
      <div style={{ display: 'flex', gap: 16, padding: 16 }}>
        <Switch id="off" checked={false} aria-label="Off" onCheckedChange={() => {}} />
        <Switch id="on" checked aria-label="On" onCheckedChange={() => {}} />
      </div>
      <div style={{ padding: 16 }}>
        <SegmentedControl aria-label="Theme" options={OPTIONS} value={value} onValueChange={setValue} data-testid="fit" />
      </div>
      <div style={{ padding: 16, width: 300 }}>
        <SegmentedControl aria-label="Range" fill options={OPTIONS} value="system" onValueChange={() => {}} data-testid="fill" />
      </div>
      <p id="line" style={{ margin: 0, padding: 16, display: 'flex', alignItems: 'center' }}>
        <span id="label">Prevent sleep</span>
        <InfoTip label="Prevent sleep" help="The screen can still lock." data-testid="tip" />
      </p>
      <p id="bare" style={{ margin: 0, padding: 16 }}>Prevent sleep</p>
      <p id="inline" style={{ margin: 0, padding: 16 }}>
        Prevent sleep<InfoTip label="Prevent sleep" help="The screen can still lock." />
      </p>
      <div style={{ width: 560, padding: 16 }}>
        <SettingSection title="General">
          <SettingRow label="Text size" controlId="row-28">
            <span id="row-28" style={{ display: 'block', width: 120, height: 28 }} />
          </SettingRow>
          <SettingRow label="Max turns" controlId="row-32">
            <span id="row-32" style={{ display: 'block', width: 120, height: 32 }} />
          </SettingRow>
          <SettingRow label="Prevent sleep" controlId="row-switch">
            <Switch id="row-switch" checked={false} onCheckedChange={() => {}} />
          </SettingRow>
        </SettingSection>
      </div>
      <div style={{ padding: 16 }}>
        <Tabs defaultValue="general">
          <TabsList aria-label="Sections" data-testid="tabs" style={{ display: 'flex', gap: 20 }}>
            <TabsTrigger value="general" style={{ padding: '8px 0' }}>General</TabsTrigger>
            <TabsTrigger value="models" style={{ padding: '8px 0' }}>Models</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      <div style={{ width: 240, padding: 4 }}>
        <div role="menuitem" className={MENU_ROW_CLASS_NAME} id="row">Rename</div>
        <div role="menuitemcheckbox" aria-checked="true" className={MENU_ROW_CLASS_NAME} id="checked">
          <span>Show archived</span>
          <span className={MENU_ROW_TRAILING_CLASS_NAME}><svg id="check" width="16" height="16" /></span>
        </div>
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<App />);
`;

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

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

describe('the shared controls in a real layout engine (spec 5.7)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the controls geometry tests.'
      );
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
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + readTokens() + primitiveBlocks();
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The page, with every transition sampled on its first frame (no reduced motion). */
  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 900 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    // Record the thumb's transition at the moment it is first placed, before anything settles.
    await tab.evaluate(() => {
      const w = window as unknown as { firstPlacement: string[] };
      w.firstPlacement = [];
      new MutationObserver((records) => {
        for (const record of records) {
          const node = record.target as HTMLElement;
          if (
            node.classList?.contains('br-segmented-thumb') &&
            node.getAttribute('data-placed') === 'true' &&
            w.firstPlacement.length === 0
          ) {
            w.firstPlacement.push(getComputedStyle(node).transitionDuration);
          }
        }
      }).observe(document.body, {
        attributes: true,
        subtree: true,
        attributeFilter: ['data-placed'],
      });
    });
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#row');
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    return tab;
  }

  it('draws the Switch 32×20 with a 16px knob inset 2px, off and on', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const box = (el: Element) => el.getBoundingClientRect();
        const out: Record<string, { track: DOMRect; knob: DOMRect }> = {};
        for (const id of ['off', 'on']) {
          const track = document.getElementById(id)!;
          out[id] = { track: box(track), knob: box(track.querySelector('.br-switch-thumb')!) };
        }
        return JSON.parse(JSON.stringify(out));
      });
      for (const state of ['off', 'on'] as const) {
        const { track, knob } = m[state];
        near(track.width, 32, `${state} track width`);
        near(track.height, 20, `${state} track height`);
        near(knob.width, 16, `${state} knob width`);
        near(knob.height, 16, `${state} knob height`);
        near(knob.top - track.top, 2, `${state} knob top inset`);
        near(track.bottom - knob.bottom, 2, `${state} knob bottom inset`);
      }
      near(m.off.knob.left - m.off.track.left, 2, 'off knob start inset');
      near(m.on.track.right - m.on.knob.right, 2, 'on knob end inset');
    } finally {
      await tab.close();
    }
  });

  it('draws menu rows at 32px with the check at the trailing edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const r = (id: string) => document.getElementById(id)!.getBoundingClientRect();
        return { row: r('row').height, checked: r('checked'), check: r('check') };
      });
      near(m.row, 32, 'menu row height');
      near(m.checked.height, 32, 'checkbox row height');
      // 12px from the row's trailing edge: the check sits on the right, no left gutter.
      near(m.checked.right - m.check.right, 12, 'check to the trailing edge');
    } finally {
      await tab.close();
    }
  });

  it('places the segmented thumb under the selected segment, at rest the first time', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const measure = () =>
        tab.evaluate(() => {
          const track = document.querySelector('[data-testid="fit"]')!;
          const thumb = track.querySelector('.br-segmented-thumb')!.getBoundingClientRect();
          const selected = track
            .querySelector('.br-segmented-item[data-state="checked"]')!
            .getBoundingClientRect();
          return {
            track: track.getBoundingClientRect().height,
            thumb: { left: thumb.left, width: thumb.width, height: thumb.height },
            selected: { left: selected.left, width: selected.width, height: selected.height },
            first: (window as unknown as { firstPlacement: string[] }).firstPlacement,
          };
        });
      const m = await measure();
      near(m.track, 28, 'track height');
      near(m.selected.height, 24, 'segment height');
      near(m.thumb.left, m.selected.left, 'thumb left');
      near(m.thumb.width, m.selected.width, 'thumb width');
      // The first placement carried data-motion-still: no transition ran from x=0.
      expect(m.first[0]?.split(',').every((d) => parseFloat(d) === 0)).toBe(true);

      // A later move slides: the thumb is mid-way a frame after the value changes, then lands.
      await tab.evaluate(() =>
        (window as unknown as { setSegmented: (v: string) => void }).setSegmented('system')
      );
      const mid = await tab.evaluate(
        () =>
          new Promise<{ thumb: number; target: number }>((done) =>
            requestAnimationFrame(() =>
              requestAnimationFrame(() => {
                const track = document.querySelector('[data-testid="fit"]')!;
                done({
                  thumb: track.querySelector('.br-segmented-thumb')!.getBoundingClientRect().left,
                  target: track
                    .querySelector('.br-segmented-item[data-state="checked"]')!
                    .getBoundingClientRect().left,
                });
              })
            )
          )
      );
      expect(mid.thumb).toBeLessThan(mid.target - 1);
      await tab.waitForTimeout(400);
      const end = await measure();
      near(end.thumb.left, end.selected.left, 'thumb left after the move');
      near(end.thumb.width, end.selected.width, 'thumb width after the move');
    } finally {
      await tab.close();
    }
  });

  it('computes the fill-mode thumb as equal columns under the selected one', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const track = document.querySelector('[data-testid="fill"]')!;
        const items = Array.from(track.querySelectorAll('.br-segmented-item')).map((item) =>
          item.getBoundingClientRect()
        );
        const thumb = track.querySelector('.br-segmented-thumb')!.getBoundingClientRect();
        return { widths: items.map((i) => i.width), last: items[2], thumb };
      });
      near(m.widths[0], m.widths[1], 'equal columns');
      near(m.widths[1], m.widths[2], 'equal columns');
      near(m.thumb.left, m.last.left, 'fill thumb left');
      near(m.thumb.width, m.last.width, 'fill thumb width');
    } finally {
      await tab.close();
    }
  });

  it('sets the InfoTip glyph 4px after its label and adds 14px to the line', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const tip = document.querySelector('[data-testid="tip"]')!;
        const glyph = tip.querySelector('svg')!.getBoundingClientRect();
        const label = document.getElementById('label')!.getBoundingClientRect();
        const style = getComputedStyle(tip);
        const box = tip.getBoundingClientRect();
        return {
          gap: glyph.left - label.right,
          glyph: glyph.width,
          hit: { width: box.width, height: box.height },
          marginBox: box.height + parseFloat(style.marginTop) + parseFloat(style.marginBottom),
          bare: document.getElementById('bare')!.getBoundingClientRect().height,
          inline: document.getElementById('inline')!.getBoundingClientRect().height,
        };
      });
      near(m.glyph, 14, 'glyph size');
      near(m.hit.width, 24, 'hit box width');
      near(m.hit.height, 24, 'hit box height');
      near(m.gap, 4, 'label to glyph');
      near(m.marginBox, 14, 'what the hit box adds to the line');
      // In running text the 24px target does not grow the 20px line.
      near(m.inline, m.bare, 'a line with an InfoTip');
    } finally {
      await tab.close();
    }
  });

  it('puts the tab underline under the active tab, and moves it there when the tab changes', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const measure = () =>
        tab.evaluate(() => {
          const list = document.querySelector('[data-testid="tabs"]')!;
          const indicator = list.querySelector('.br-tabs-indicator')!;
          const active = list.querySelector('[role="tab"][data-state="active"]')!;
          const r = (el: Element) => el.getBoundingClientRect();
          return {
            placed: indicator.getAttribute('data-placed'),
            still: indicator.hasAttribute('data-motion-still'),
            indicator: {
              left: r(indicator).left,
              width: r(indicator).width,
              bottom: r(indicator).bottom,
              height: r(indicator).height,
            },
            active: { left: r(active).left, width: r(active).width },
            list: { bottom: r(list).bottom },
          };
        });
      const m = await measure();
      expect(m.placed).toBe('true');
      expect(m.still).toBe(false);
      near(m.indicator.left, m.active.left, 'indicator left');
      near(m.indicator.width, m.active.width, 'indicator width');
      near(m.indicator.height, 2, 'indicator height');
      near(m.indicator.bottom, m.list.bottom + 1, 'indicator on the list edge');

      await tab.click('[role="tab"]:not([data-state="active"])');
      await tab.waitForTimeout(400);
      const end = await measure();
      near(end.indicator.left, end.active.left, 'indicator left after the switch');
      near(end.indicator.width, end.active.width, 'indicator width after the switch');
    } finally {
      await tab.close();
    }
  });

  it('keeps every one-line SettingRow at 40px, whatever its 20 to 32px control (WS-SETTINGS-A-1)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const heights = await tab.evaluate(() =>
        ['row-28', 'row-32', 'row-switch'].map(
          (id) =>
            document.getElementById(id)!.closest('.br-setting-row')!.getBoundingClientRect().height
        )
      );
      near(heights[0], 40, 'row with a 28px control');
      near(heights[1], 40, 'row with a 32px control');
      near(heights[2], 40, 'row with a switch');
    } finally {
      await tab.close();
    }
  });
});
