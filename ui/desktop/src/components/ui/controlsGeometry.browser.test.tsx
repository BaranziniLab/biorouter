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
function primitiveBlocks(without?: string): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) if (match[1] !== without) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/**
 * The base D-15 rule that gives every `[tabindex]` element a focus fill and an inset edge,
 * lifted out of main.css and put back in `@layer base`, so a focus test sees what the app sees.
 */
function baseFocusRule(): string {
  const match = MAIN_CSS.match(
    /:where\(\s*:is\(\s*a,\s*button,[\s\S]*?\)\s*:focus-visible\s*\{[^}]*\}/
  );
  if (!match) throw new Error('main.css has no base focus rule');
  return `@layer base { ${match[0]} }`;
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
import { RowActions } from './row-actions';

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
          <SettingRow label="Show tips" controlId="row-switch-last">
            <Switch id="row-switch-last" checked={false} onCheckedChange={() => {}} />
          </SettingRow>
        </SettingSection>
      </div>
      <div className="biorouter-settings-list" style={{ width: 560, padding: 16 }}>
        <button id="row-button" type="button" className="biorouter-settings-row br-setting-row" style={{ width: '100%', textAlign: 'left' }}>
          <span>Provider</span>
        </button>
        <button type="button" className="biorouter-settings-row br-setting-row" style={{ width: '100%', textAlign: 'left' }}>
          <span>Another provider</span>
        </button>
      </div>
      <div style={{ padding: 16 }}>
        <Tabs defaultValue="general">
          <TabsList aria-label="Sections" data-testid="tabs" style={{ display: 'flex', gap: 20 }}>
            <TabsTrigger value="general" style={{ padding: '8px 0' }}>General</TabsTrigger>
            <TabsTrigger value="models" style={{ padding: '8px 0' }}>Models</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      <div id="scaled" style={{ transform: 'scale(0.98)', transformOrigin: '50% 50%', width: 300, padding: 16 }}>
        <Tabs defaultValue="models">
          <TabsList aria-label="Scaled sections" data-testid="scaled-tabs" style={{ display: 'flex', gap: 20 }}>
            <TabsTrigger value="general" style={{ padding: '8px 0' }}>General</TabsTrigger>
            <TabsTrigger value="models" style={{ padding: '8px 0' }}>Models</TabsTrigger>
            <TabsTrigger value="privacy" style={{ padding: '8px 0' }}>Privacy</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      <div id="centred" style={{ width: 400, padding: 16 }}>
        <Tabs defaultValue="models">
          <TabsList aria-label="Centred sections" data-testid="centred-tabs" style={{ display: 'flex', gap: 20, justifyContent: 'center' }}>
            <TabsTrigger value="general" style={{ padding: '8px 0' }}>General</TabsTrigger>
            <TabsTrigger value="models" style={{ padding: '8px 0' }}>Models</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      <div id="squeeze" style={{ width: 400, padding: 16 }}>
        <SegmentedControl aria-label="Squeezed" options={OPTIONS} value="system" onValueChange={() => {}} data-testid="squeezed" />
      </div>
      <div data-row-actions-host="" id="meta-row" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', width: 360, padding: 8 }}>
        <span>Nightly cohort</span>
        <RowActions
          meta={<time>2h ago</time>}
          menu={[{ label: 'Rename', onSelect: () => {} }]}
          menuTestId="meta-more"
        />
      </div>
      <p style={{ margin: 0, padding: 16 }}>
        <InfoTip label="Legacy" help="Kept for older bases." asChild>
          <span id="as-child-target" data-testid="as-child-target">Legacy</span>
        </InfoTip>
      </p>
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
    css = PREFLIGHT + readTokens() + baseFocusRule() + primitiveBlocks();
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** The page, with every transition sampled on its first frame (no reduced motion). */
  async function open(styles: string = css): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 900 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${styles}</style></head><body><div id="root"></div></body></html>`
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
  /** Hover the row, then ask which element sits at the ⋯ button's centre, right and left edges. */
  async function hitTestMore(tab: Page) {
    await tab.hover('#meta-row');
    await tab.waitForTimeout(250);
    return tab.evaluate(() => {
      const more = document.querySelector('[data-testid="meta-more"]')!;
      const box = more.getBoundingClientRect();
      const at = (x: number, y: number) => {
        const top = document.elementFromPoint(x, y);
        return top === more || more.contains(top);
      };
      return {
        centre: at(box.left + box.width / 2, box.top + box.height / 2),
        right: at(box.right - 2, box.top + box.height / 2),
        left: at(box.left + 2, box.top + box.height / 2),
      };
    });
  }

  it('keeps the invisible meta off the ⋯ button, so the click lands on it (WS-VIEWS-A-3)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const hit = await hitTestMore(tab);
      expect(hit.centre, 'the ⋯ centre').toBe(true);
      expect(hit.right, 'the ⋯ right edge').toBe(true);
      expect(hit.left, 'the ⋯ left edge').toBe(true);
    } finally {
      await tab.close();
    }

    // The instrument: without the rule the time paints over the button's centre and right edge.
    const bare = await open(
      css.replace(/(\.br-row-actions-meta \{[^}]*?)pointer-events: none;/, '$1')
    );
    try {
      const hit = await hitTestMore(bare);
      expect(hit.centre || hit.right, 'the bug this guards').toBe(false);
    } finally {
      await bare.close();
    }
  });

  /** The row's wash and the firmed line under it, as the browser resolved them. */
  async function rowLook(tab: Page) {
    return tab.evaluate(() => {
      const row = document.getElementById('row-switch')!.closest('.biorouter-settings-row')!;
      return {
        wash: getComputedStyle(row).backgroundColor,
        shadow: getComputedStyle(row).boxShadow,
        line: getComputedStyle(row, '::after').borderTopColor,
        within: row.matches(':focus-within'),
        visible: row.matches(':has(:focus-visible)'),
      };
    });
  }

  /** Press Tab from the top of a fresh page until the keyboard reaches the element with this id. */
  async function tabTo(tab: Page, id: string) {
    for (let step = 0; step < 60; step += 1) {
      await tab.keyboard.press('Tab');
      if (await tab.evaluate((wanted) => document.activeElement?.id === wanted, id)) return;
    }
    throw new Error(`Tab never reached #${id}`);
  }

  it('leaves a clicked switch row at rest, and washes and firms it for a Tab (WS-SETTINGS-B-2)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const rest = await rowLook(tab);
      expect(rest.within).toBe(false);

      // A real mouse click, then the pointer leaves: nothing of the click may remain.
      await tab.click('#row-switch');
      await tab.mouse.move(790, 5);
      await tab.waitForTimeout(300);
      const clicked = await rowLook(tab);
      expect(clicked.within, 'the click left focus inside the row').toBe(true);
      expect(clicked.visible, 'a click is not visible focus').toBe(false);
      expect(clicked.wash, 'the row washed after a click').toBe(rest.wash);
      expect(clicked.line, 'the hairline firmed after a click').toBe(rest.line);
    } finally {
      await tab.close();
    }

    // Tab from the top of a fresh page until the keyboard reaches the same switch.
    const keys = await open();
    try {
      const rest = await rowLook(keys);
      await tabTo(keys, 'row-switch');
      await keys.mouse.move(790, 5);
      await keys.waitForTimeout(300);
      const keyboard = await rowLook(keys);
      expect(keyboard.visible).toBe(true);
      expect(keyboard.wash, 'Tab washes the row').not.toBe(rest.wash);
      expect(keyboard.line, 'Tab firms the line').toBe('rgb(85, 85, 85)');
      // The row only CONTAINS the focus: D-15's inset edge is the control's, not the row's.
      expect(keyboard.shadow, 'a row that merely contains the focus').toBe('none');
    } finally {
      await keys.close();
    }

    // The instrument: keyed on `:focus-within`, as the rows were, the click does leave its mark.
    const old = await open(css.replace(/:has\(:focus-visible\)/g, ':focus-within'));
    try {
      const rest = await rowLook(old);
      await old.click('#row-switch');
      await old.mouse.move(790, 5);
      await old.waitForTimeout(300);
      const clicked = await rowLook(old);
      expect(clicked.wash !== rest.wash || clicked.line !== rest.line, 'the bug this guards').toBe(
        true
      );
    } finally {
      await old.close();
    }
  });

  it('draws D-15 on a row that is itself the focus target, and keeps it under the pointer', async (ctx) => {
    if (!browser) return ctx.skip();
    const look = (tab: Page) =>
      tab.evaluate(() => {
        const row = document.getElementById('row-button')!;
        const cs = getComputedStyle(row);
        return { wash: cs.backgroundColor, shadow: cs.boxShadow };
      });
    const own = await open();
    try {
      await tabTo(own, 'row-button');
      await own.mouse.move(790, 5);
      await own.waitForTimeout(300);
      const focused = await look(own);
      expect(focused.wash, 'D-15 fill (--background-focus)').toBe('rgb(221, 221, 221)');
      expect(focused.shadow, 'D-15 inset 2px edge').toBe('rgb(85, 85, 85) 0px 0px 0px 2px inset');
      // Pointing at the focused row does not take the fill back.
      await own.hover('#row-button');
      await own.waitForTimeout(300);
      expect((await look(own)).wash).toBe('rgb(221, 221, 221)');
    } finally {
      await own.close();
    }

    // The instrument: without the block, a focused full-width row had a 5% wash and no edge,
    // the same as a hover, which is what a keyboard user got.
    const bare = await open(
      PREFLIGHT + readTokens() + baseFocusRule() + primitiveBlocks('row focus')
    );
    try {
      await tabTo(bare, 'row-button');
      const lost = await look(bare);
      expect(lost.shadow, 'the bug this guards').toBe('none');
      expect(lost.wash, 'the bug this guards').not.toBe('rgb(221, 221, 221)');
    } finally {
      await bare.close();
    }
  });

  it('measures the tab underline in the list’s own pixels while an ancestor is scaled', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const list = document.querySelector('[data-testid="scaled-tabs"]') as HTMLElement;
        const indicator = list.querySelector('.br-tabs-indicator') as HTMLElement;
        const active = list.querySelector('[role="tab"][data-state="active"]') as HTMLElement;
        const box = active.getBoundingClientRect();
        return {
          x: parseFloat(indicator.style.translate),
          width: parseFloat(indicator.style.width),
          // Layout pixels, which the scale does not touch.
          left: active.offsetLeft,
          layoutWidth: box.width / 0.98,
          scaled: list.getBoundingClientRect().width / list.offsetWidth,
        };
      });
      expect(m.scaled, 'the list really is scaled').toBeLessThan(0.99);
      near(m.x, m.left, 'underline x against the tab’s layout position');
      near(m.width, m.layoutWidth, 'underline width against the tab’s layout width');
    } finally {
      await tab.close();
    }
  });

  /** Record the indicator's or thumb's transition duration at every moment its box is rewritten. */
  async function recordPlacements(tab: Page, selector: string) {
    await tab.evaluate((sel) => {
      const w = window as unknown as { placements: Array<{ duration: string; box: string }> };
      w.placements = [];
      const node = document.querySelector(sel) as HTMLElement;
      new MutationObserver(() => {
        w.placements.push({
          duration: getComputedStyle(node).transitionDuration,
          box: `${node.style.translate}|${node.style.width}`,
        });
      }).observe(node, { attributes: true, attributeFilter: ['style'] });
    }, selector);
  }

  const placements = (tab: Page) =>
    tab.evaluate(
      () =>
        (window as unknown as { placements: Array<{ duration: string; box: string }> }).placements
    );

  it('snaps the tab underline when the layout moves it, and glides when the person picks', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await recordPlacements(tab, '[data-testid="centred-tabs"] .br-tabs-indicator');
      // The list is centred: narrowing its container moves the active tab.
      await tab.evaluate(() => {
        (document.getElementById('centred') as HTMLElement).style.width = '200px';
      });
      await tab.waitForTimeout(250);
      const moved = await placements(tab);
      expect(moved.length, 'the observer rewrote the underline').toBeGreaterThan(0);
      for (const entry of moved) {
        expect(parseFloat(entry.duration), 'a layout move is at rest').toBe(0);
      }

      await tab.evaluate(() => {
        (window as unknown as { placements: unknown[] }).placements.length = 0;
      });
      await tab.click('[data-testid="centred-tabs"] [role="tab"]:not([data-state="active"])');
      await tab.waitForTimeout(250);
      const picked = await placements(tab);
      expect(picked.length).toBeGreaterThan(0);
      expect(parseFloat(picked[0].duration), 'a pick glides').toBeGreaterThan(0);
    } finally {
      await tab.close();
    }
  });

  it('snaps the segmented thumb when a squeeze resizes its segments', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await recordPlacements(tab, '[data-testid="squeezed"] .br-segmented-thumb');
      await tab.evaluate(() => {
        (document.getElementById('squeeze') as HTMLElement).style.width = '120px';
      });
      await tab.waitForTimeout(250);
      const squeezed = await placements(tab);
      expect(squeezed.length, 'the observer rewrote the thumb').toBeGreaterThan(0);
      for (const entry of squeezed) {
        expect(parseFloat(entry.duration), 'a squeeze is at rest').toBe(0);
      }
      const m = await tab.evaluate(() => {
        const track = document.querySelector('[data-testid="squeezed"]')!;
        const thumb = track.querySelector('.br-segmented-thumb')!.getBoundingClientRect();
        const selected = track
          .querySelector('.br-segmented-item[data-state="checked"]')!
          .getBoundingClientRect();
        return { thumb, selected };
      });
      near(m.thumb.left, m.selected.left, 'thumb left after the squeeze');
      near(m.thumb.width, m.selected.width, 'thumb width after the squeeze');
    } finally {
      await tab.close();
    }
  });

  it('gives a focused asChild InfoTip target one indicator, its outline (WS-HISTORY-2)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Keyboard modality first, so a script's focus() counts as :focus-visible.
      await tab.keyboard.press('Tab');
      const style = await tab.evaluate(() => {
        const target = document.getElementById('as-child-target') as HTMLElement;
        target.focus();
        const s = getComputedStyle(target);
        return {
          focused: target.matches(':focus-visible'),
          outlineStyle: s.outlineStyle,
          outlineWidth: s.outlineWidth,
          boxShadow: s.boxShadow,
          background: s.backgroundColor,
        };
      });
      expect(style.focused).toBe(true);
      expect(style.outlineStyle).toBe('solid');
      expect(style.outlineWidth).toBe('2px');
      expect(style.boxShadow, 'no inset edge inside the outline').toBe('none');
      expect(style.background, 'no focus fill on the chip').toBe('rgba(0, 0, 0, 0)');
    } finally {
      await tab.close();
    }
  });
  it('opens an InfoTip on hover after 200ms, on Tab at once, closes on Escape and when the pointer leaves', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const content = () => tab.locator('[data-slot="info-tip-content"]');
      const glyph = () => tab.locator('[data-testid="tip"]');

      // The words are in the page before anything opens, and the trigger points at them.
      const wiring = await tab.evaluate(() => {
        const tip = document.querySelector('[data-testid="tip"]')!;
        const described = document.getElementById(tip.getAttribute('aria-describedby')!);
        return {
          name: tip.getAttribute('aria-label'),
          text: described?.textContent,
          hidden: described ? getComputedStyle(described).clip : null,
        };
      });
      expect(wiring.name).toBe('About Prevent sleep');
      expect(wiring.text).toBe('The screen can still lock.');

      // Hover: shut at 100ms, open by 350ms.
      await glyph().hover();
      await tab.waitForTimeout(100);
      expect(await content().count(), 'not before 200ms').toBe(0);
      await tab.waitForTimeout(250);
      expect(await content().count(), 'after 200ms').toBe(1);

      // Above its trigger with an 8px gap, aligned to the trigger's start.
      const place = await tab.evaluate(() => {
        const box = document.querySelector('[data-testid="tip"]')!.getBoundingClientRect();
        const tip = document
          .querySelector('[data-slot="info-tip-content"]')!
          .getBoundingClientRect();
        return { gap: box.top - tip.bottom, startDelta: tip.left - box.left };
      });
      near(place.gap, 8, 'gap above the trigger');
      expect(Math.abs(place.startDelta), 'start-aligned').toBeLessThanOrEqual(1);

      // Crossing onto the tip keeps it open; leaving closes it.
      const box = await content().boundingBox();
      await tab.mouse.move(box!.x + 4, box!.y + 4);
      await tab.waitForTimeout(250);
      expect(await content().count(), 'the pointer is on the tip').toBe(1);
      await tab.mouse.move(700, 800);
      await tab.waitForTimeout(250);
      expect(await content().count(), 'the pointer left').toBe(0);

      // Tab opens it at once; Escape closes it.
      for (let i = 0; i < 40; i++) {
        await tab.keyboard.press('Tab');
        const onTip = await tab.evaluate(
          () => document.activeElement?.getAttribute('data-testid') === 'tip'
        );
        if (onTip) break;
      }
      await tab.waitForTimeout(30);
      expect(await content().count(), 'opens on Tab with no delay').toBe(1);
      await tab.keyboard.press('Escape');
      await tab.waitForTimeout(60);
      expect(await content().count(), 'Escape closes it').toBe(0);

      // A click toggles it, and a program's focus() does not open it.
      await glyph().click();
      await tab.waitForTimeout(60);
      expect(await content().count(), 'a click opens it').toBe(1);
      await glyph().click();
      await tab.waitForTimeout(60);
      expect(await content().count(), 'a second click closes it').toBe(0);
    } finally {
      await tab.close();
    }
  });
  it('keeps the switch, the segmented choice, the tab underline and the row divider visible in forced colours', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.emulateMedia({ forcedColors: 'active' });
      // The track and knob ease their colour: read them once the transition has settled.
      await tab.waitForTimeout(400);
      const m = await tab.evaluate(() => {
        const cs = (el: Element) => getComputedStyle(el);
        const probe = document.createElement('div');
        probe.style.backgroundColor = 'Canvas';
        document.body.append(probe);
        const canvas = cs(probe).backgroundColor;
        const on = document.getElementById('on')!;
        const off = document.getElementById('off')!;
        const knob = (el: Element) => el.querySelector('.br-switch-thumb')!;
        const seg = document.querySelector(
          '[data-testid="fit"] .br-segmented-item[data-state="checked"]'
        )!;
        const indicator = document.querySelector('[data-testid="tabs"] .br-tabs-indicator')!;
        const row = document.querySelector('.br-setting-row')!;
        const box = (el: Element) => el.getBoundingClientRect();
        return {
          canvas,
          onTrack: cs(on).backgroundColor,
          offTrack: cs(off).backgroundColor,
          onEdge: cs(on).borderTopColor,
          offEdge: cs(off).borderTopColor,
          onKnob: cs(knob(on)).backgroundColor,
          offKnob: cs(knob(off)).backgroundColor,
          offInset: box(knob(off)).left - box(off).left,
          onInset: box(on).right - box(knob(on)).right,
          thumbShown: cs(document.querySelector('.br-segmented-thumb')!).display,
          segOutline: `${cs(seg).outlineStyle} ${cs(seg).outlineWidth}`,
          segOutlineColor: cs(seg).outlineColor,
          indicator: cs(indicator).backgroundColor,
          rowBorder: cs(row).borderBottomColor,
          switchSize: [box(on).width, box(on).height],
        };
      });
      expect(m.onTrack, 'an on switch is filled').not.toBe(m.canvas);
      expect(m.offTrack, 'an off switch is hollow').toBe(m.canvas);
      expect(m.offEdge, 'an off switch has an edge').not.toBe(m.canvas);
      expect(m.onKnob, 'the knob changes with the state').not.toBe(m.offKnob);
      near(m.offInset, 2, 'off knob start inset');
      near(m.onInset, 2, 'on knob end inset');
      near(m.switchSize[0], 32, 'switch width');
      near(m.switchSize[1], 20, 'switch height');
      expect(m.thumbShown, 'the thumb stands down').toBe('none');
      expect(m.segOutline).toBe('solid 2px');
      expect(m.segOutlineColor, 'the chosen segment is outlined').not.toBe(m.canvas);
      expect(m.indicator, 'the underline is a visible fill').not.toBe(m.canvas);
      expect(m.rowBorder, 'the row’s border room is invisible, so it cannot curl').toBe(m.canvas);
    } finally {
      await tab.close();
    }
  });
});
