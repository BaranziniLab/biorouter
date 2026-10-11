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
 * A neutral Badge rests on the strong step everywhere, and a washed row steps it back to the page
 * ground with its controls (`--br-badge-ground`, V3-R2-5), the privacy pill beside it with it.
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
import { Badge } from './badge';
import { PrivacyBadge } from './PrivacyBadge';
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
      <Badge className="kit-badge">Built-in</Badge>
    </div>
  );
}

function Rows({ id, flat }: { id: string; flat?: boolean }) {
  return (
    <div id={id} className={'biorouter-list-shell' + (flat ? ' br-rows-flat' : '')} style={{ width: 360 }}>
      {[1, 2, 3].map((n) => (
        <div key={n} className="biorouter-list-row" data-row={n} style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <span style={{ flex: 1 }}>Row {n}</span>
          <Badge className="row-badge">Built-in</Badge>
          <Badge tone="danger" className="row-tint">Flag</Badge>
          <span className="br-privacy-pill bg-background-muted row-pill">Public</span>
          <PrivacyBadge tier="private" enforcementOff={false} className="row-real-pill" />
          <Progress label={'row ' + n} value={50} size="thin" style={{ width: 60 }} />
          <Button className="row-button" variant="secondary" size="sm">Configure</Button>
        </div>
      ))}
    </div>
  );
}

/** Settings rows, the shape Crew's workspace settings use for a member and its Host badge. */
function SettingsRows() {
  return (
    <div id="settings" style={{ width: 360 }}>
      {[1, 2].map((n) => (
        <div key={n} className="biorouter-settings-row" data-row={n} style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <span style={{ flex: 1 }}>Member {n}</span>
          <Badge className="srow-badge">Host</Badge>
          <PrivacyBadge tier="private" enforcementOff={false} className="srow-pill" />
          <Button className="srow-button" variant="secondary" size="sm">Remove</Button>
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
      <SettingsRows />
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
    badge: await ground(tab, `#${name} .kit-badge`),
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
          // The neutral badge is the strong step, a step off the page and off a field beside it.
          badge: await paint(tab, 'var(--background-fill-strong)'),
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
          // A badge is not a control: it stays on the strong step, a step off the card's fill.
          badge: strong,
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
          badge: strong,
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
        const badge = '#lined [data-row="2"] .row-badge';
        const tint = '#lined [data-row="2"] .row-tint';
        expect(await ground(tab, button), `${mode} at rest`).toBe(fill);
        expect(await ground(tab, pill), `${mode} the privacy pill at rest`).toBe(
          await paint(tab, 'var(--background-muted)')
        );
        // The real PrivacyBadge is a neutral Badge with the muted utility: as built, the utility
        // out-ranks the badge's layered class, so it rests on muted, not the strong step.
        const realPill = '#lined [data-row="2"] .row-real-pill';
        expect(await ground(tab, realPill), `${mode} the real pill at rest`).toBe(
          await paint(tab, 'var(--background-muted)')
        );
        expect(await ground(tab, badge), `${mode} the badge at rest`).toBe(
          await paint(tab, 'var(--background-fill-strong)', '#lined')
        );
        const tintAtRest = await ground(tab, tint);
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
        // The neutral badge steps back to the page ground with the controls: the wash lands within
        // 1.08:1 of the strong step it rests on (V3-R2-5). A tinted badge keeps its own wash.
        expect(await ground(tab, badge), `${mode} the badge on a washed row`).toBe(page);
        expect(await ground(tab, tint), `${mode} a tinted badge is not moved`).toBe(tintAtRest);
        // The privacy pill keeps its box (the wash composites to its muted rest, WS-SKILLS-1) on the
        // badge's ground, so one row is one ground (WS-KNOWLEDGE-3).
        expect(await ground(tab, pill), `${mode} the privacy pill on a washed row`).toBe(page);
        expect(await ground(tab, realPill), `${mode} the real pill on a washed row`).toBe(page);
        await tab.hover(button);
        await tab.waitForTimeout(250);
        expect(await ground(tab, button), `${mode} the button under the pointer`).toBe(
          await paint(tab, 'var(--background-fill-strong)', '#lined')
        );
        // The neighbouring row is not washed and its button still rests on the fill.
        expect(await ground(tab, '#lined [data-row="3"] .row-button')).toBe(fill);
        expect(await ground(tab, '#lined [data-row="3"] .row-badge')).toBe(
          await paint(tab, 'var(--background-fill-strong)', '#lined')
        );
      } finally {
        await tab.close();
      }
    }
  });

  /** WCAG relative luminance of a pixel, and the contrast ratio of two. */
  const ratio = (a: number[], b: number[]) => {
    const lum = ([r, g, bl]: number[]) => {
      const c = [r, g, bl].map((v) => {
        const x = v / 255;
        return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    };
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };

  it('never stands the badge closer to a washed row than the strong step did, and further in dark', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const row = (await tab.$('#lined [data-row="2"]'))!;
        const box = (await row.boundingBox())!;
        await tab.mouse.move(box.x + 12, box.y + box.height / 2);
        await tab.waitForTimeout(250);
        // A probe on the strong step in the same washed row, where the badge used to rest.
        await tab.evaluate(() => {
          const probe = document.createElement('span');
          probe.id = 'strong-probe';
          probe.style.cssText =
            'display:inline-block;flex:none;width:12px;height:12px;background:var(--background-fill-strong)';
          const row = document.querySelector('#lined [data-row="2"]')!;
          row.insertBefore(probe, row.children[1]);
        });
        // The pixels: the wash (the row's left gutter), the badge's padding, the probe.
        const at = async (x: number, y: number) => {
          const shot = await tab.screenshot({ clip: { x, y, width: 1, height: 1 } });
          const png = await tab.evaluate(async (b64) => {
            const img = new Image();
            img.src = 'data:image/png;base64,' + b64;
            await img.decode();
            const c = document.createElement('canvas');
            c.width = c.height = 1;
            const g = c.getContext('2d')!;
            g.drawImage(img, 0, 0);
            return Array.from(g.getImageData(0, 0, 1, 1).data.slice(0, 3));
          }, shot.toString('base64'));
          return png;
        };
        const badgeBox = (await (await tab.$('#lined [data-row="2"] .row-badge'))!.boundingBox())!;
        const probeBox = (await (await tab.$('#strong-probe'))!.boundingBox())!;
        const wash = await at(box.x + 2, box.y + box.height / 2);
        const badge = await at(badgeBox.x + 2, badgeBox.y + badgeBox.height / 2);
        const strong = await at(probeBox.x + 6, probeBox.y + 6);
        const now = ratio(badge, wash);
        const before = ratio(strong, wash);
        // Measured 2026-10-11: the wash is #f4f4f4 light, #262623 dark; the strong step stood
        // 1.10:1 and 1.08:1 off it, the page ground stands 1.10:1 and 1.14:1 off it, lighter than
        // the wash in light and darker in dark, as the row's button does.
        expect(before, `${mode}: the strong step on the wash (the defect)`).toBeLessThan(1.1);
        expect(
          now,
          `${mode}: the badge on the wash, never closer than before`
        ).toBeGreaterThanOrEqual(before);
        if (mode === 'dark')
          expect(now - before, 'dark: the step the reviewer saw vanish grows').toBeGreaterThan(
            0.04
          );
      } finally {
        await tab.close();
      }
    }
  });

  it('steps a washed settings row’s badge back too, and leaves its privacy pill on the muted rest', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const strong = await paint(tab, 'var(--background-fill-strong)', '#settings');
        const page = await paint(tab, 'var(--background-default)', '#settings');
        const muted = await paint(tab, 'var(--background-muted)', '#settings');
        const badge = '#settings [data-row="1"] .srow-badge';
        const pill = '#settings [data-row="1"] .srow-pill';
        expect(await ground(tab, badge), `${mode} at rest`).toBe(strong);
        // Keyboard focus on a control in the row washes it, as the pointer does.
        await tab.focus('#settings [data-row="1"] .srow-button');
        expect(
          await tab.evaluate(() => document.activeElement!.matches(':focus-visible')),
          'the instrument: the button holds visible focus'
        ).toBe(true);
        expect(await ground(tab, badge), `${mode} with focus in the row`).toBe(page);
        await tab.evaluate(() => (document.activeElement as HTMLElement).blur());
        const row = (await tab.$('#settings [data-row="1"]'))!;
        const box = (await row.boundingBox())!;
        await tab.mouse.move(box.x + 12, box.y + box.height / 2);
        await tab.waitForTimeout(250);
        expect(await ground(tab, badge), `${mode} under the pointer`).toBe(page);
        // Crew draws "Private · UCSF" as one chip in a settings row: the pill is left alone there.
        expect(await ground(tab, pill), `${mode} the pill on a settings row`).toBe(muted);
        expect(await ground(tab, '#settings [data-row="2"] .srow-badge')).toBe(strong);
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
