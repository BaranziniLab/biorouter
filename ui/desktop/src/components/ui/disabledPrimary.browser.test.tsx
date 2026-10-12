// @vitest-environment node
/**
 * A primary that cannot act is neutral (vision V4-R7-4, coordinator ruling 22, design.md 9.7), in
 * a real layout engine.
 *
 * The claim is about the cascade: `default` and `destructive` set their fill and ink through
 * utilities, `.br-button-inert` (main.css, unlayered) has to beat them while the button is
 * disabled, and not when it is working (`aria-busy`). jsdom runs no stylesheet, so the real
 * `Button` is bundled with esbuild, `styles/main.css` is compiled by Tailwind with the classes the
 * bundle uses (the app's own stylesheet), and Chromium paints it. Colours are compared with probes
 * that paint the token itself, so a token value can change without this file changing.
 *
 * Before the rule, Tool permissions > Developer "Save changes", Make workflow "Create and run" and
 * Reset data's "Reset" drew a coral or danger button at 50% opacity while the catalog dialogs drew
 * a secondary for the same state ("not yet"): two looks.
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
import { Card, CardContent } from './card';

function Row({ id, children }: { id: string; children: React.ReactNode }) {
  return <div id={id} style={{ display: 'flex', gap: 8, alignItems: 'center' }}>{children}</div>;
}

function App() {
  return (
    <div style={{ display: 'grid', gap: 12, padding: 24, width: 560 }}>
      <Row id="enabled">
        <Button id="on-default">Save changes</Button>
        <Button id="on-destructive" variant="destructive">Reset</Button>
        <Button id="on-secondary" variant="secondary">Cancel</Button>
      </Row>
      <Row id="disabled">
        <Button id="off-default" disabled>Save changes</Button>
        <Button id="off-destructive" variant="destructive" disabled>Reset</Button>
        <Button id="off-secondary" variant="secondary" disabled>Cancel</Button>
        <Button id="off-outline" variant="outline" disabled>Outline</Button>
        <Button id="off-ghost" variant="ghost" disabled>Ghost</Button>
      </Row>
      <Row id="busy">
        <Button id="busy-default" disabled aria-busy="true">Saving…</Button>
        <Button id="busy-destructive" variant="destructive" disabled aria-busy="true">Resetting…</Button>
        <Button id="busy-false" disabled aria-busy="false">Not busy</Button>
        <Button id="aria-off" aria-disabled="true">Check</Button>
        <Button id="aria-busy-off" aria-disabled="true" aria-busy="true">Checking</Button>
      </Row>
      <Row id="sizes">
        <Button id="size-on" size="sm">Create and run</Button>
        <Button id="size-off" size="sm" disabled>Create and run</Button>
      </Row>
      <Card id="card">
        <CardContent>
          <Row id="in-card">
            <Button id="card-secondary" variant="secondary" disabled>Create</Button>
            <Button id="card-default" disabled>Create and run</Button>
            <Button id="card-destructive" variant="destructive" disabled>Reset</Button>
          </Row>
        </CardContent>
      </Card>
      <div id="recessed" className="br-ground-recessed bg-background-muted" style={{ padding: 8 }}>
        <Row id="in-recessed">
          <Button id="recessed-secondary" variant="secondary" disabled>Create</Button>
          <Button id="recessed-default" disabled>Create and run</Button>
        </Row>
      </div>
      <div id="popover" className="biorouter-popover-surface" style={{ padding: 8 }}>
        <Row id="in-popover">
          <Button id="popover-secondary" variant="secondary" disabled>Create</Button>
          <Button id="popover-default" disabled>Create and run</Button>
        </Row>
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

describe('a primary that cannot act is neutral (V4-R7-4)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the disabled primary tests.');
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
    const tab = await browser!.newPage({ viewport: { width: 700, height: 700 } });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#popover-default');
    // Let every colour transition settle before reading a colour.
    await tab.waitForTimeout(300);
    return tab;
  }

  /** The computed colour of `property` that a probe paints for the token `value`. */
  const paint = (
    tab: Page,
    property: 'backgroundColor' | 'color',
    value: string,
    within = 'body'
  ) =>
    tab.evaluate(
      ([p, v, w]) => {
        const probe = document.createElement('div');
        (probe.style as unknown as Record<string, string>)[p] = v;
        document.querySelector(w)!.append(probe);
        const out = getComputedStyle(probe)[p as 'color'];
        probe.remove();
        return out;
      },
      [property, value, within] as const
    );

  const look = (tab: Page, id: string) =>
    tab.evaluate((s) => {
      const el = document.getElementById(s)!;
      const cs = getComputedStyle(el);
      const box = el.getBoundingClientRect();
      return {
        ground: cs.backgroundColor,
        ink: cs.color,
        opacity: cs.opacity,
        edge: cs.borderTopColor,
        edgeWidth: cs.borderTopWidth,
        size: [box.width, box.height],
      };
    }, id);

  for (const mode of ['light', 'dark'] as const) {
    it(`${mode}: an enabled primary keeps its colour`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        const accent = await paint(tab, 'backgroundColor', 'var(--background-accent)');
        const onAccent = await paint(tab, 'color', 'var(--text-on-accent)');
        const danger = await look(tab, 'on-destructive');
        const onDefault = await look(tab, 'on-default');
        expect(onDefault.ground).toBe(accent);
        expect(onDefault.ink).toBe(onAccent);
        expect(onDefault.opacity).toBe('1');
        expect(danger.ground, 'the danger wash is not the neutral fill').not.toBe(
          await paint(tab, 'backgroundColor', 'var(--background-fill)')
        );
        expect(danger.ink).toBe(await paint(tab, 'color', 'var(--text-danger)'));
      } finally {
        await tab.close();
      }
    });

    it(`${mode}: a disabled default and a disabled destructive draw what a disabled secondary draws`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        const fill = await paint(tab, 'backgroundColor', 'var(--background-fill)');
        const ink = await paint(tab, 'color', 'var(--text-default)');
        const secondary = await look(tab, 'off-secondary');
        expect(secondary.ground).toBe(fill);
        expect(secondary.opacity).toBe('0.5');
        for (const id of ['off-default', 'off-destructive']) {
          const inert = await look(tab, id);
          expect(inert.ground, `${id} ground`).toBe(secondary.ground);
          expect(inert.ink, `${id} ink`).toBe(ink);
          expect(inert.ink, `${id} ink`).toBe(secondary.ink);
          expect(inert.opacity, `${id} fade`).toBe('0.5');
          expect(inert.edgeWidth, `${id} keeps no border`).toBe(secondary.edgeWidth);
        }
        // Nothing moves between the two states: the button is the same box.
        const on = await look(tab, 'size-on');
        const off = await look(tab, 'size-off');
        expect(off.size).toEqual(on.size);
      } finally {
        await tab.close();
      }
    });

    it(`${mode}: the other variants are not touched`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        const outline = await look(tab, 'off-outline');
        expect(outline.ground).toBe(await paint(tab, 'backgroundColor', 'var(--background-fill)'));
        const ghost = await look(tab, 'off-ghost');
        expect(ghost.ground, 'a disabled ghost stays transparent').toBe('rgba(0, 0, 0, 0)');
        expect(ghost.opacity).toBe('0.5');
      } finally {
        await tab.close();
      }
    });

    it(`${mode}: a button that is working keeps its colour`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        const accent = await paint(tab, 'backgroundColor', 'var(--background-accent)');
        const neutral = await paint(tab, 'backgroundColor', 'var(--background-fill)');
        expect((await look(tab, 'busy-default')).ground).toBe(accent);
        expect((await look(tab, 'busy-destructive')).ground).not.toBe(neutral);
        expect(
          (await look(tab, 'busy-destructive')).ink,
          'a working destructive keeps its danger ink'
        ).toBe(await paint(tab, 'color', 'var(--text-danger)'));
        // `aria-busy="false"` is the attribute some call sites write while idle: not busy.
        expect((await look(tab, 'busy-false')).ground).toBe(neutral);
        // `aria-disabled` fades without the attribute (the Copilot buttons) and is covered too.
        expect((await look(tab, 'aria-off')).ground).toBe(neutral);
        expect((await look(tab, 'aria-busy-off')).ground).toBe(accent);
      } finally {
        await tab.close();
      }
    });

    it(`${mode}: it steps to the ground a secondary rests on in a filled card, on a recessed ground and in a popover`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(mode);
      try {
        const pageGround = await paint(tab, 'backgroundColor', 'var(--background-default)');
        const card = await look(tab, 'card-secondary');
        expect(card.ground, 'a fill never sits on a fill').toBe(pageGround);
        expect((await look(tab, 'card-default')).ground).toBe(card.ground);
        expect((await look(tab, 'card-destructive')).ground).toBe(card.ground);

        const recessed = await look(tab, 'recessed-secondary');
        expect(recessed.ground).toBe(
          await paint(tab, 'backgroundColor', 'var(--background-fill-strong)')
        );
        expect((await look(tab, 'recessed-default')).ground).toBe(recessed.ground);

        const popover = await look(tab, 'popover-secondary');
        expect((await look(tab, 'popover-default')).ground).toBe(popover.ground);
      } finally {
        await tab.close();
      }
    });
  }

  it('keeps the system button edge on a disabled primary under forced colours', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light', true);
    try {
      for (const id of ['off-default', 'off-destructive', 'off-secondary']) {
        const l = await look(tab, id);
        expect(l.edgeWidth, `${id} border`).toBe('1px');
        expect(l.edge, `${id} edge is not transparent`).not.toBe('rgba(0, 0, 0, 0)');
      }
    } finally {
      await tab.close();
    }
  });
});
