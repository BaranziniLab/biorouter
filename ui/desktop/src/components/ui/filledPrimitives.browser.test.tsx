// @vitest-environment node
/**
 * The filled primitives under the whole stylesheet, in a real layout engine (owner message 13,
 * R-27, design.md 9.11; docs/desktop-ui/shared-controls.md "Fills before lines").
 *
 * jsdom runs no CSS, and the claims here are about the cascade: a layered field rule against the
 * `@layer base` focus rule, a private ground variable inherited from a filled box, a hover that has
 * to beat nothing and lose to a caller's utility, forced colours drawing a transparent border. So
 * the real components are bundled with esbuild, `styles/main.css` is compiled by Tailwind with the
 * classes the bundle uses (the app's own stylesheet, not a hand-copied subset), and Chromium paints
 * them. Colours are compared with probes that paint the token itself, so a token value can change
 * without this file changing.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Button } from './button';
import { Input } from './input';
import { Card, CardContent } from './card';
import { Note } from './note';
import { Badge } from './badge';

function Pair({ prefix }: { prefix: string }) {
  return (
    <>
      <Input id={prefix + '-field'} aria-label={prefix + ' field'} placeholder="Value" />
      <Button id={prefix + '-button'} variant="secondary" size="sm">Skip</Button>
    </>
  );
}

function App() {
  return (
    <div style={{ display: 'grid', gap: 16, padding: 24, width: 420 }}>
      <div id="page"><Pair prefix="page" /></div>
      <Input id="invalid" aria-label="Invalid" aria-invalid defaultValue="not a url" />
      <div style={{ display: 'flex', alignItems: 'start', gap: 8 }}>
        <Button id="outline" variant="outline" size="sm">Outline</Button>
        <Button id="secondary-width" variant="secondary" size="sm">Outline</Button>
        <Button id="caller-ground" variant="secondary" size="sm" className="bg-background-default">Mine</Button>
      </div>
      <Card id="card"><CardContent><Pair prefix="card" /></CardContent></Card>
      <Note testId="note" action={<Button id="note-button" variant="secondary" size="sm">Retry</Button>}>
        <span id="note-words">The extension stopped responding.</span>
      </Note>
      <Note testId="bare-note"><span>Saved.</span></Note>
      <div id="panel" className="biorouter-modal-panel rounded-container"><div><Pair prefix="panel" /></div></div>
      <div className="br-fill-surface bg-background-fill" id="host"><div><Pair prefix="host" /></div></div>
      <div className="biorouter-list-row" id="row"><Badge id="badge">Built-in</Badge></div>
      <div className="biorouter-popover-surface" id="popover" style={{ padding: 8 }}>
        <Button id="popover-button" variant="secondary" size="sm">Disable all</Button>
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

describe('the filled primitives under the whole stylesheet (R-27)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the filled primitives tests.');
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
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark', forcedColors = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 600, height: 1400 } });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#popover-button');
    // Let every colour transition settle before reading a colour.
    await tab.waitForTimeout(300);
    return tab;
  }

  /** The computed colour a probe inside `within` paints for `value` (a token, in that place). */
  const paint = (tab: Page, value: string, within = 'body') =>
    tab.evaluate(
      ([v, w]) => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = v;
        document.querySelector(w)!.append(probe);
        const out = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return out;
      },
      [value, within] as const
    );
  const style = (tab: Page, selector: string) =>
    tab.evaluate((s) => {
      const el = document.querySelector(s)!;
      const cs = getComputedStyle(el);
      const box = el.getBoundingClientRect();
      return {
        ground: cs.backgroundColor,
        edge: cs.borderTopColor,
        edgeWidth: cs.borderTopWidth,
        size: [box.width, box.height],
      };
    }, selector);

  it('rests a field and a secondary button on the fill and steps them to the strong fill under the pointer', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const fill = await paint(tab, 'var(--background-fill)');
        const strong = await paint(tab, 'var(--background-fill-strong)');
        const clear = await paint(tab, 'transparent');
        for (const id of ['#page-field', '#page-button']) {
          const rest = await style(tab, id);
          expect(rest.ground, `${mode} ${id} rests on the fill`).toBe(fill);
          await tab.hover(id);
          await tab.waitForTimeout(250);
          expect((await style(tab, id)).ground, `${mode} ${id} hovers on the strong fill`).toBe(
            strong
          );
          await tab.mouse.move(590, 1390);
        }
        // The field keeps its border as transparent room: no line, no reflow when one appears.
        const field = await style(tab, '#page-field');
        expect(field.edgeWidth).toBe('1px');
        expect(field.edge).toBe(clear);
        // `outline` is the same box, with the old outline's width kept as transparent room.
        const outline = await style(tab, '#outline');
        expect(outline.ground).toBe(fill);
        expect(outline.edge).toBe(clear);
        expect(outline.size[0]).toBe((await style(tab, '#secondary-width')).size[0] + 2);
        // A caller's own ground still wins over the layered rest.
        expect((await style(tab, '#caller-ground')).ground).toBe(
          await paint(tab, 'var(--background-default)')
        );
      } finally {
        await tab.close();
      }
    }
  });

  it('firms the field’s edge on focus, keeps a focused field’s fill under the pointer, and keeps the danger edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const fill = await paint(tab, 'var(--background-fill)');
      await tab.focus('#page-field');
      await tab.hover('#page-field');
      await tab.waitForTimeout(250);
      const focused = await style(tab, '#page-field');
      expect(focused.edge).toBe(
        await tab.evaluate(() => {
          const probe = document.createElement('div');
          probe.style.borderTop = '1px solid var(--border-focus)';
          document.body.append(probe);
          return getComputedStyle(probe).borderTopColor;
        })
      );
      expect(focused.ground, 'the box being typed in does not change under the pointer').toBe(fill);
      const danger = await tab.evaluate(() => {
        const probe = document.createElement('div');
        probe.style.borderTop = '1px solid var(--border-danger)';
        document.body.append(probe);
        return getComputedStyle(probe).borderTopColor;
      });
      expect((await style(tab, '#invalid')).edge).toBe(danger);
      await tab.focus('#invalid');
      await tab.waitForTimeout(250);
      expect((await style(tab, '#invalid')).edge, 'invalid wins over focus').toBe(danger);
    } finally {
      await tab.close();
    }
  });

  it('steps the controls inside a filled box back to the page ground (a fill never sits on a fill)', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const page = await paint(tab, 'var(--background-default)');
        const fill = await paint(tab, 'var(--background-fill)');
        expect((await style(tab, '#card')).ground, `${mode} the card is filled`).toBe(fill);
        expect((await style(tab, '[data-testid="note"]')).ground).toBe(fill);
        expect((await style(tab, '#panel')).ground).toBe(fill);
        for (const prefix of ['card', 'panel', 'host']) {
          for (const kind of ['field', 'button']) {
            expect(
              (await style(tab, `#${prefix}-${kind}`)).ground,
              `${mode} ${kind} inside ${prefix}`
            ).toBe(page);
          }
        }
        expect((await style(tab, '#note-button')).ground).toBe(page);
        await tab.hover('#card-button');
        await tab.waitForTimeout(250);
        expect((await style(tab, '#card-button')).ground).toBe(
          await paint(tab, 'var(--background-fill-strong)')
        );
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps a neutral badge a step off a hovered row, and a hovered button a step off a dark popover', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        await tab.hover('#row');
        await tab.waitForTimeout(250);
        const row = await style(tab, '#row');
        expect((await style(tab, '#badge')).ground, `${mode} badge on a hovered row`).not.toBe(
          row.ground
        );
        const ground = await paint(tab, 'var(--background-default)', '#popover');
        await tab.hover('#popover-button');
        await tab.waitForTimeout(250);
        expect(
          (await style(tab, '#popover-button')).ground,
          `${mode} a hovered button inside a popover is not the popover's own ground`
        ).not.toBe(ground);
      } finally {
        await tab.close();
      }
    }
  });

  it('draws every filled box’s edge in forced colours, at the same size', async (ctx) => {
    if (!browser) return ctx.skip();
    const rest = await open('light');
    const forced = await open('light', true);
    try {
      const canvas = await paint(forced, 'Canvas');
      for (const id of ['#page-field', '#card', '[data-testid="note"]', '#panel', '#outline']) {
        const before = await style(rest, id);
        const after = await style(forced, id);
        expect(after.edgeWidth, `${id} keeps a 1px edge`).toBe('1px');
        expect(after.edge, `${id}'s edge is drawn`).not.toBe(canvas);
        expect(after.edge).not.toMatch(/rgba\(.*,\s*0\)$/);
        expect(after.size, `${id} costs no geometry`).toEqual(before.size);
      }
    } finally {
      await rest.close();
      await forced.close();
    }
  });

  it('centres a note’s action on its first line and keeps a one-line note as tall as one without', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const m = await tab.evaluate(() => {
        const centre = (r: DOMRect) => r.top + r.height / 2;
        const words = document.getElementById('note-words')!.getClientRects()[0];
        const button = document.getElementById('note-button')!.getBoundingClientRect();
        const note = document.querySelector('[data-testid="note"]')!.getBoundingClientRect();
        const bare = document.querySelector('[data-testid="bare-note"]')!.getBoundingClientRect();
        return {
          offset: centre(button) - centre(words),
          height: note.height,
          bare: bare.height,
          inside: button.top >= note.top && button.bottom <= note.bottom,
        };
      });
      expect(Math.abs(m.offset), 'button centre vs first line centre').toBeLessThanOrEqual(0.5);
      expect(m.height).toBe(m.bare);
      expect(m.inside, 'the button stays inside the note').toBe(true);
    } finally {
      await tab.close();
    }
  });
});
