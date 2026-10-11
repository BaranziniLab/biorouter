// @vitest-environment node
/**
 * The grounds and the rows, said once (WS-SETTINGS-M-3, WS-KNOWLEDGE-2, WS-VIEWS-A-5), under the
 * whole stylesheet in Chromium.
 *
 * Three private variables carry what a ground asks of the primitives on it (main.css, "THE GROUNDS,
 * said once"): where a filled control rests, where a filled button hovers, and the ground of a
 * track. A plain page draws what it drew; a filled box steps controls back to the page and tracks
 * one step in; `.br-ground-recessed` steps everything one step in; a washed row steps its controls
 * back like a filled box. `.br-rows-flat` drops the hairline between rows except in forced colours.
 *
 * jsdom runs no cascade and has no `::after`, so the real components are bundled with esbuild,
 * `styles/main.css` is compiled by Tailwind with the classes the bundle uses, and Chromium paints
 * them. Colours are compared with probes that paint the token itself in the same place.
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
import { SegmentedControl } from './segmented-control';
import { CopyField } from './copy-field';
import { Progress } from './progress';
import { EmptyState } from './empty-state';
import { Database } from '../icons/app-icons';

function Kit({ name }: { name: string }) {
  const [v, setV] = React.useState('a');
  return (
    <div id={name} style={{ display: 'grid', gap: 8, padding: 8 }}>
      <SegmentedControl
        aria-label={name + ' view'}
        options={[{ value: 'a', label: 'One' }, { value: 'b', label: 'Two' }]}
        value={v}
        onValueChange={setV}
      />
      <CopyField label={name + ' command'} value="npx biorouter" />
      <Progress label={name + ' progress'} value={40} />
      <EmptyState compact icon={Database} title="Nothing yet" description="Add a base." />
      <Input aria-label={name + ' field'} placeholder="Value" />
      <Button className="kit-button" variant="secondary" size="sm">Skip</Button>
    </div>
  );
}

function Rows({ id, flat }: { id: string; flat?: boolean }) {
  return (
    <div id={id} className={'biorouter-list-shell' + (flat ? ' br-rows-flat' : '')} style={{ width: 360 }}>
      {[1, 2, 3].map((n) => (
        <div key={n} className="biorouter-list-row" data-row={n} style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <span style={{ flex: 1 }}>Row {n}</span>
          <span className="br-privacy-pill bg-background-muted row-pill">Public</span>
          <Progress label={'row ' + n} value={50} size="thin" style={{ width: 60 }} />
          <Button className="row-button" variant="secondary" size="sm">Configure</Button>
        </div>
      ))}
    </div>
  );
}

function App() {
  return (
    <div style={{ display: 'grid', gap: 16, padding: 24, width: 480 }}>
      <Kit name="page" />
      <Card><CardContent><Kit name="card" /></CardContent></Card>
      <div className="br-ground-recessed bg-background-muted" style={{ padding: 8 }}>
        <Kit name="recessed" />
        <Card><CardContent><Kit name="recessed-card" /></CardContent></Card>
      </div>
      <Rows id="lined" />
      <Rows id="flat" flat />
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

describe('the grounds and the rows, said once', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the grounds and rows tests.');
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
    const tab = await browser!.newPage({ viewport: { width: 600, height: 2600 } });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#flat [data-row="3"]');
    await tab.waitForTimeout(300);
    return tab;
  }

  /** The colour a probe placed inside `within` paints for `value`. */
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
  const ground = (tab: Page, selector: string) =>
    tab.evaluate((s) => getComputedStyle(document.querySelector(s)!).backgroundColor, selector);

  /** Where each primitive in a kit rests. */
  const kit = async (tab: Page, name: string) => ({
    track: await ground(tab, `#${name} .br-segmented`),
    copy: await ground(tab, `#${name} .biorouter-copy-field`),
    progress: await ground(tab, `#${name} .br-progress`),
    tile: await ground(tab, `#${name} .br-empty-tile`),
    field: await ground(tab, `#${name} input`),
    button: await ground(tab, `#${name} .kit-button`),
  });

  it('draws a plain page exactly as before (every fallback is today’s value)', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const fill = await paint(tab, 'var(--background-fill)');
        expect(await kit(tab, 'page'), mode).toEqual({
          track: fill,
          copy: await paint(tab, 'var(--background-well)'),
          progress: await paint(tab, 'var(--background-muted)'),
          tile: fill,
          field: fill,
          button: fill,
        });
      } finally {
        await tab.close();
      }
    }
  });

  it('steps controls back to the page and tracks one step in, inside a filled box', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const page = await paint(tab, 'var(--background-default)', '#card');
        const strong = await paint(tab, 'var(--background-fill-strong)', '#card');
        const box = await paint(tab, 'var(--background-fill)', '#card');
        const k = await kit(tab, 'card');
        expect(k, mode).toEqual({
          track: strong,
          copy: page,
          progress: strong,
          tile: page,
          field: page,
          button: page,
        });
        // Nothing in the box is the box's own colour (the defect: track, copy field and progress
        // track all read 1.00:1 on the card before).
        for (const [part, value] of Object.entries(k))
          expect(value, `${mode} ${part}`).not.toBe(box);
      } finally {
        await tab.close();
      }
    }
  });

  it('rests everything one step in on a recessed ground, and buttons hover one deeper', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const strong = await paint(tab, 'var(--background-fill-strong)', '#recessed');
        const muted = await paint(tab, 'var(--background-muted)');
        const k = await kit(tab, 'recessed');
        expect(k, mode).toEqual({
          track: strong,
          copy: strong,
          progress: strong,
          tile: strong,
          field: strong,
          button: strong,
        });
        for (const [part, value] of Object.entries(k))
          expect(value, `${mode} ${part}`).not.toBe(muted);
        await tab.hover('#recessed .kit-button');
        await tab.waitForTimeout(250);
        expect(await ground(tab, '#recessed .kit-button')).toBe(
          await paint(tab, 'var(--background-strong)', '#recessed')
        );
        // A filled box on the recessed ground sets its own grounds again.
        await tab.mouse.move(590, 2590);
        const page = await paint(tab, 'var(--background-default)', '#recessed-card');
        const inCard = await kit(tab, 'recessed-card');
        expect(inCard.button).toBe(page);
        expect(inCard.copy).toBe(page);
        await tab.hover('#recessed-card .kit-button');
        await tab.waitForTimeout(250);
        expect(await ground(tab, '#recessed-card .kit-button')).toBe(
          await paint(tab, 'var(--background-fill-strong)', '#recessed-card')
        );
      } finally {
        await tab.close();
      }
    }
  });

  it('steps a washed row’s button back to the page ground, so it keeps its box under the pointer', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const fill = await paint(tab, 'var(--background-fill)');
        const button = '#lined [data-row="2"] .row-button';
        const pill = '#lined [data-row="2"] .row-pill';
        expect(await ground(tab, button), `${mode} at rest`).toBe(fill);
        expect(await ground(tab, pill), `${mode} the privacy pill at rest`).toBe(
          await paint(tab, 'var(--background-muted)')
        );
        // Point at the row, not the button.
        const row = await tab.$('#lined [data-row="2"]');
        const box = (await row!.boundingBox())!;
        await tab.mouse.move(box.x + 12, box.y + box.height / 2);
        await tab.waitForTimeout(250);
        const page = await paint(tab, 'var(--background-default)', '#lined');
        expect(await ground(tab, button), `${mode} in a washed row`).toBe(page);
        expect(await ground(tab, button)).not.toBe(await ground(tab, '#lined [data-row="2"]'));
        expect(await ground(tab, '#lined [data-row="2"] .br-progress')).toBe(
          await paint(tab, 'var(--background-fill-strong)', '#lined')
        );
        // The privacy pill keeps its box: the wash composites to its resting value (WS-SKILLS-1).
        expect(await ground(tab, pill), `${mode} the privacy pill on a washed row`).toBe(
          await paint(tab, 'var(--background-fill-strong)', '#lined')
        );
        await tab.hover(button);
        await tab.waitForTimeout(250);
        expect(await ground(tab, button), `${mode} the button under the pointer`).toBe(
          await paint(tab, 'var(--background-fill-strong)', '#lined')
        );
        // The neighbouring row is not washed and its button still rests on the fill.
        expect(await ground(tab, '#lined [data-row="3"] .row-button')).toBe(fill);
      } finally {
        await tab.close();
      }
    }
  });

  it('drops the hairline between flat rows, and draws it again in forced colours', async (ctx) => {
    if (!browser) return ctx.skip();
    const after = (tab: Page, list: string) =>
      tab.evaluate(
        (l) =>
          Array.from(document.querySelectorAll(`#${l} .biorouter-list-row`)).map(
            (row) => getComputedStyle(row, '::after').content
          ),
        list
      );
    const rest = await open('light');
    const forced = await open('light', true);
    try {
      expect(await after(rest, 'lined'), 'the default list keeps its line').toEqual([
        '""',
        '""',
        'none',
      ]);
      expect(await after(rest, 'flat')).toEqual(['none', 'none', 'none']);
      expect(await after(forced, 'flat'), 'forced colours: the line comes back').toEqual([
        '""',
        '""',
        'none',
      ]);
      // Nothing moves: a flat row is as tall as a lined one.
      const height = (tab: Page, l: string) =>
        tab.evaluate(
          (s) => document.querySelector(`#${s} [data-row="1"]`)!.getBoundingClientRect().height,
          l
        );
      expect(await height(rest, 'flat')).toBe(await height(rest, 'lined'));
    } finally {
      await rest.close();
      await forced.close();
    }
  });
});
