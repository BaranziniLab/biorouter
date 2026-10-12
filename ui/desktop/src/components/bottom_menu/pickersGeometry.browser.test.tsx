// @vitest-environment node
/**
 * The composer's pickers measured in a real layout engine (spec 3.7 and 5.7).
 *
 * jsdom lays nothing out and never reads `pickers.css`, so the component tests
 * can only pin class names. The acceptance numbers are geometry: the Tools and
 * model chips are 28px ghost chips in 13px type, the model chip ellipsizes
 * inside its 260px cap, a list row is the 32px menu row with the switch at its
 * trailing edge and grows only for a visible second line, the popover is 320px
 * wide, and the footer line is 12px with 24px hit areas. So the real
 * `PickerPanel` is bundled with esbuild, rendered by React in Chromium, and
 * styled by `pickers.css` plus the `@ws WS-PRIMITIVES` blocks of `main.css`
 * (the switch and the menu row), with token values read from `main.css`.
 *
 * ⚠ Tailwind does not run here. What this measures is the AUTHORED CSS, which is
 * the point: under `BIOROUTER_NO_HMR` a newly written utility can silently fail
 * to generate, and the pickers' load-bearing geometry must not depend on one.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const PICKERS_CSS = readFileSync(resolve(here, 'pickers.css'), 'utf8');

const TOKEN_NAMES = [
  '--control-compact',
  '--control-sm',
  '--control-md',
  '--radius-full',
  '--radius-element',
  '--radius-inner',
  '--icon-chip',
  '--text-supporting',
  '--text-supporting--line-height',
  '--text-secondary',
  '--text-secondary--line-height',
  '--text-label',
  '--text-label--line-height',
  '--dur-fast-min',
  '--dur-fast',
  '--dur-fast-max',
  '--ease-out',
  '--ease-spring',
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

/** Every `@ws WS-PRIMITIVES` block of main.css (the switch, the menu row, the InfoTip). */
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
button, input { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
svg { display: block; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --background-muted: #f2f2f0; --border-focus: #555; --background-focus: #ddd;
  --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1); --inset-hairline: inset 0 0 0 1px #ddd;
  --overlay-hover: rgb(0 0 0 / 0.05); --overlay-pressed: rgb(0 0 0 / 0.1); --border-subtle: #ccc; }
`;

/**
 * The base layer's D-15 focus rule, as main.css declares it. It sits in a cascade
 * layer, so a control that paints its own ground with unlayered CSS (both chips
 * here do) loses it outright; `pickers.css` carries the rule the chips need.
 */
const BASE_FOCUS = `@layer base {
  :where(button):focus-visible {
    outline: none;
    background-color: var(--background-focus);
    color: var(--text-default);
    box-shadow: inset 0 0 0 2px var(--border-focus);
  }
}`;

/**
 * The Tailwind utilities these measurements depend on, in their own cascade layer:
 * the one the dimming rule has to beat, and `CommandList`'s `p-1`, which puts the
 * rows' text 16px in, and the bulk action's `px-2`. Without them the rows sat 12px
 * in here and 16px in the real build, and a scope line 4px out of line with them
 * passed as aligned.
 */
const UTILITIES = `@layer utilities {
  .opacity-50 { opacity: 0.5; }
  .p-1 { padding: 4px; }
}
/* Unlayered, because this file's preflight is: its \`button { padding: 0 }\` would beat a
   layered utility, where the real build layers both (base under utilities). */
.px-2 { padding-inline: 8px; }`;

const LONG_MODEL = 'gpt-5.6-sol-with-a-name-long-enough-to-need-an-ellipsis-on-any-composer';

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { PickerPanel, PickerRow } from './PickerPanel';
import { ChevronDown, Folder } from '../icons/app-icons';

function App() {
  const [query, setQuery] = React.useState('');
  return (
    <div style={{ padding: 16 }}>
      <div style={{ display: 'flex', gap: 8, width: 600 }}>
        <button type="button" className="br-picker-chip" id="tools">
          <span>Tools</span>
          <span className="br-picker-chip__count">15</span>
          <ChevronDown className="br-picker-chip__chevron" aria-hidden="true" id="tools-chevron" />
        </button>
        <button type="button" className="br-picker-chip" data-kind="model" id="model">
          <span className="br-picker-chip__name" id="model-name">${LONG_MODEL}</span>
          <ChevronDown className="br-picker-chip__chevron" aria-hidden="true" />
        </button>
        <button type="button" className="br-picker-chip" id="open-chip" aria-expanded="true">
          <span>Tools</span>
          <ChevronDown className="br-picker-chip__chevron" aria-hidden="true" id="open-chevron" />
        </button>
      </div>
      <div className="br-picker-popover" id="popover" style={{ marginTop: 16 }}>
        <PickerPanel
          listLabel="Extensions"
          searchPlaceholder="Search extensions…"
          query={query}
          onQueryChange={setQuery}
          scopeLabel="This chat"
          scopeHelp="Changes apply to this chat only."
          bulk={{ label: 'Disable all (2)', onClick: () => {} }}
          emptyText="No extensions found"
        >
          <PickerRow name="spoke" checked onToggle={() => {}} data-testid="row-on" description="Spoke" />
          <PickerRow name="example" checked={false} onToggle={() => {}} data-testid="row-off" />
          <PickerRow
            name="ucsfomopagent"
            checked={false}
            disabled
            onToggle={() => {}}
            note="Unavailable in this chat (public model)"
            data-testid="row-note"
          />
        </PickerPanel>
      </div>
      <div className="br-footline" id="footline" style={{ width: 600, marginTop: 16 }}>
        <div className="br-footline__side">
          <button type="button" className="br-footline__item" id="folder">
            <Folder aria-hidden="true" id="folder-glyph" />
            <span className="br-footline__name">Desktop</span>
          </button>
        </div>
        <div className="br-footline__side" data-side="end">
          <button type="button" className="br-footline__item" id="cost">$0.42</button>
        </div>
      </div>
      {/* A chat that is still loading: its folder is not known (V2-R7-3) and the ring and the
          cost have nothing to say yet, so ComposerFooter renders two empty sides. */}
      <div className="br-footline" id="footline-bare" style={{ width: 600, marginTop: 16 }}>
        <div className="br-footline__side" />
        <div className="br-footline__side" data-side="end" />
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

describe('the composer pickers in a real layout engine (spec 3.7, 5.7)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the pickers geometry tests.'
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
      // The stylesheets are injected below, in a known order; a CSS import in a
      // module would otherwise need an output file.
      loader: { '.css': 'empty' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + BASE_FOCUS + UTILITIES + readTokens() + primitiveBlocks() + PICKERS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 900 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#footline');
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    return tab;
  }

  it('draws both chips as 28px ghost chips in 13px type, the chevron at 14px', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const box = (id: string) => document.getElementById(id)!.getBoundingClientRect();
        const style = (id: string) => getComputedStyle(document.getElementById(id)!);
        return {
          tools: box('tools').height,
          model: box('model').height,
          size: style('tools').fontSize,
          weight: style('tools').fontWeight,
          ground: style('tools').backgroundColor,
          chevron: { w: box('tools-chevron').width, h: box('tools-chevron').height },
        };
      });
      near(m.tools, 28, 'Tools chip height');
      near(m.model, 28, 'model chip height');
      expect(m.size).toBe('13px');
      expect(m.weight).toBe('400');
      // Ghost: no ground at rest (principle 7, hover is neutral and only on hover).
      expect(m.ground).toBe('rgba(0, 0, 0, 0)');
      near(m.chevron.w, 14, 'chevron width');
      near(m.chevron.h, 14, 'chevron height');
    } finally {
      await tab.close();
    }
  });

  it('caps the model chip at 260px and ellipsizes the name inside it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const chip = document.getElementById('model')!;
        const name = document.getElementById('model-name')!;
        return {
          width: chip.getBoundingClientRect().width,
          overflowing: name.scrollWidth > name.clientWidth,
          ellipsis: getComputedStyle(name).textOverflow,
        };
      });
      expect(m.width).toBeLessThanOrEqual(260.5);
      expect(m.overflowing).toBe(true);
      expect(m.ellipsis).toBe('ellipsis');
    } finally {
      await tab.close();
    }
  });

  it('draws a list row as the 32px menu row with the switch on its trailing edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const row = (id: string) => document.querySelector(`[data-testid="${id}"]`)!;
        const out: Record<string, unknown> = {};
        for (const id of ['row-on', 'row-off', 'row-note']) {
          const el = row(id);
          const sw = el.querySelector('.br-switch')!.getBoundingClientRect();
          const r = el.getBoundingClientRect();
          out[id] = {
            height: r.height,
            size: getComputedStyle(el).fontSize,
            switchW: sw.width,
            switchH: sw.height,
            trailing: r.right - sw.right,
            centred: Math.abs(r.top + r.height / 2 - (sw.top + sw.height / 2)),
          };
        }
        const note = row('row-note').querySelector('.br-picker-row__note')!;
        out.noteSize = getComputedStyle(note).fontSize;
        const opacity = (el: Element | null) => getComputedStyle(el!).opacity;
        const refused = row('row-note');
        out.refused = {
          row: opacity(refused),
          note: opacity(note),
          name: opacity(refused.querySelector('.br-picker-row__title')),
          switch: opacity(refused.querySelector('.br-picker-row__switch')),
        };
        return JSON.parse(JSON.stringify(out));
      });
      for (const id of ['row-on', 'row-off'] as const) {
        near(m[id].height, 32, `${id} height`);
        expect(m[id].size).toBe('13px');
        near(m[id].switchW, 32, `${id} switch width`);
        near(m[id].switchH, 20, `${id} switch height`);
        near(m[id].trailing, 8, `${id} switch to the trailing edge`);
        near(m[id].centred, 0, `${id} switch centred on the row`);
      }
      // A refusal is a visible second line (never tooltip-only), so that row grows.
      expect(m['row-note'].height).toBeGreaterThan(32);
      expect(m.noteSize).toBe('12px');
      // ...and it is read at full strength: the row dims what is unavailable, not why.
      expect(m.refused).toEqual({ row: '1', note: '1', name: '0.5', switch: '0.5' });
    } finally {
      await tab.close();
    }
  });

  it('opens a 320px popover whose scope line is 12px, on the rows’ text edges', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const popover = document.getElementById('popover')!.getBoundingClientRect();
        const bar = document.querySelector('.br-picker-panel__bar')!;
        const scope = bar.querySelector('.br-picker-panel__scope')!.getBoundingClientRect();
        const bulk = bar.querySelector(':scope > button')!;
        const bulkText = document.createRange();
        bulkText.selectNodeContents(bulk);
        const rowName = document.querySelector('[data-testid="row-on"] .br-picker-row__name')!;
        const rowSwitch = document
          .querySelector('[data-testid="row-on"] .br-picker-row__switch')!
          .getBoundingClientRect();
        const b = bar.getBoundingClientRect();
        return {
          width: popover.width,
          barHeight: b.height,
          size: getComputedStyle(bar).fontSize,
          scopeLeft: scope.left - popover.left,
          nameLeft: rowName.getBoundingClientRect().left - popover.left,
          bulkTextRight: popover.right - bulkText.getBoundingClientRect().right,
          switchRight: popover.right - rowSwitch.right,
        };
      });
      near(m.width, 320, 'popover width');
      expect(m.barHeight).toBeGreaterThanOrEqual(28);
      expect(m.size).toBe('12px');
      // One left edge for the words: the scope label starts where every row's
      // name does, 16px in (the menu recipe's edge), not 4px short of it.
      near(m.nameLeft, 16, 'row name 16px in');
      near(m.scopeLeft, m.nameLeft, 'scope word on the rows’ text edge');
      // One right edge for the trailing controls: the bulk action's words end
      // where the switches do, 12px in.
      near(m.switchRight, 12, 'switch 12px from the trailing edge');
      near(m.bulkTextRight, m.switchRight, 'bulk action on the switches’ trailing edge');
    } finally {
      await tab.close();
    }
  });

  it('draws the footer line in 12px with 24px hit areas and a 14px folder glyph', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const box = (id: string) => document.getElementById(id)!.getBoundingClientRect();
        const footline = box('footline');
        return {
          folder: box('folder').height,
          cost: box('cost').height,
          size: getComputedStyle(document.getElementById('folder')!).fontSize,
          family: getComputedStyle(document.getElementById('cost')!).fontFamily,
          glyph: box('folder-glyph').width,
          costRight: footline.right - box('cost').right,
        };
      });
      near(m.folder, 24, 'folder hit area');
      near(m.cost, 24, 'cost hit area');
      expect(m.size).toBe('12px');
      expect(m.family).not.toMatch(/mono/i);
      near(m.glyph, 14, 'folder glyph');
      near(m.costRight, 0, 'cost on the trailing edge');
    } finally {
      await tab.close();
    }
  });

  it('keeps the footer line its 24px while a chat has no folder, ring or cost yet', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const m = await tab.evaluate(() => {
        const box = (id: string) => document.getElementById(id)!.getBoundingClientRect();
        return { bare: box('footline-bare').height, full: box('footline').height };
      });
      // The composer must not grow or shrink under the person's eyes when the
      // folder arrives: the empty line already holds the height it will keep.
      near(m.bare, 24, 'empty footer line');
      near(m.bare, m.full, 'empty line against the full one');
    } finally {
      await tab.close();
    }
  });

  it('holds the ground and turns the chevron of a chip whose popover or menu is open', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Open is `aria-expanded`: the tooltip around each trigger overwrites
      // `data-state`, so a `[data-state='open']` rule never matched.
      await tab.waitForTimeout(250);
      const m = await tab.evaluate(() => {
        const style = (id: string) => getComputedStyle(document.getElementById(id)!);
        return {
          ground: style('open-chip').backgroundColor,
          restingGround: style('tools').backgroundColor,
          turned: style('open-chevron').transform,
          restingChevron: style('tools-chevron').transform,
        };
      });
      expect(m.ground).toBe('rgba(0, 0, 0, 0.05)');
      expect(m.restingGround).toBe('rgba(0, 0, 0, 0)');
      expect(m.turned).toBe('matrix(-1, 0, 0, -1, 0, 0)');
      expect(m.restingChevron).toBe('none');
    } finally {
      await tab.close();
    }
  });

  it('gives a Tab-focused chip and footer item the focus surface, though each paints its own ground', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const reach = async (id: string) => {
        for (let presses = 0; presses < 30; presses++) {
          await tab.keyboard.press('Tab');
          if (await tab.evaluate((want) => document.activeElement?.id === want, id)) break;
        }
        // The ground and ink transition over `--dur-fast-min`: read them at rest.
        await tab.waitForTimeout(250);
        return tab.evaluate((want) => {
          const el = document.getElementById(want)!;
          const style = getComputedStyle(el);
          return {
            focused: document.activeElement === el,
            visible: el.matches(':focus-visible'),
            ground: style.backgroundColor,
            ink: style.color,
            edge: style.boxShadow,
          };
        }, id);
      };
      for (const id of ['tools', 'model', 'folder', 'cost']) {
        const m = await reach(id);
        expect(m.focused, `${id} was reached with Tab`).toBe(true);
        expect(m.visible, `${id} matches :focus-visible`).toBe(true);
        // --background-focus (#ddd) and --text-default (#111), not the resting
        // transparent ground and muted ink the unlayered rules would keep.
        expect(m.ground, `${id} focus ground`).toBe('rgb(221, 221, 221)');
        expect(m.ink, `${id} focus ink`).toBe('rgb(17, 17, 17)');
        expect(m.edge, `${id} inset edge`).toMatch(/rgb\(85, 85, 85\) 0px 0px 0px 2px inset/);
      }
    } finally {
      await tab.close();
    }
  });
});
