// @vitest-environment node
/**
 * The Skills page, the BAAM browse dialogs and Add skill under the whole stylesheet, in a real
 * layout engine (owner message 13, R-27, design.md 9.11: fills before lines).
 *
 * jsdom runs no CSS, and every claim here is about the cascade: a row hairline that a scope class
 * switches off, a button that has to stay visible on a row whose hover wash is the button's own
 * fill, a drop zone whose states are authored rules. So the real components are bundled with
 * esbuild, `styles/main.css` is compiled by Tailwind with the classes the bundle uses (plus this
 * area's two authored sheets, which the bundle's `.css` loader drops), and Chromium paints them.
 * Colours are compared with probes that paint the token itself, so a token value can change
 * without this file changing. Screenshots land in
 * `~/biorouter-runs/redesign-2026-10-09/after/WS-SKILLS/r27-tests/` when that run directory exists.
 *
 * Each assertion was proved by removing the rule it guards (the commit message says which).
 */
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const RUN_DIR = join(homedir(), 'biorouter-runs/redesign-2026-10-09');
const SHOTS = join(RUN_DIR, 'after/WS-SKILLS/r27-tests');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import SkillItem from './SkillItem';
import AddSkillModal from './AddSkillModal';
import CustomSkillModal from './CustomSkillModal';
import { MarketplaceDialog, MarketplaceRow, MarketplaceSection } from '../baam/MarketplaceDialog';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Checkbox } from '../ui/Checkbox';

const state = { machineEnabled: true, session: 'default', sessionViaBundle: false, hiddenContext: false, effective: true };
const skill = (name) => ({
  name, description: name + ' does things, in one line that stops where the row does.', slug: name,
  directory: '/skills/' + name, sourceRoot: '/skills',
  source: { kind: 'biorouter', extension: null, label: 'Biorouter' }, bundle: null, builtin: false, state,
});
const noop = () => {};

function Rows() {
  return (
    <div style={{ width: 700, padding: 24 }}>
      <div id="flat" className="biorouter-list-shell br-rows-flat">
        {['alpha', 'beta', 'gamma'].map((n) => <SkillItem key={n} skill={skill(n)} enabled onOpen={noop} onShare={noop} onToggle={noop} />)}
      </div>
      <div id="ruled" className="biorouter-list-shell" style={{ marginTop: 24 }}>
        {['delta', 'epsilon', 'zeta'].map((n) => <SkillItem key={n} skill={skill(n)} enabled onOpen={noop} onShare={noop} onToggle={noop} />)}
      </div>
    </div>
  );
}

function Market() {
  return (
    <MarketplaceDialog
      title="Browse extensions" help="Pick one." live search="" onSearchChange={noop}
      searchLabel="Search extensions" status="ready" emptyText="Nothing" onClose={noop}
    >
      <MarketplaceSection label="Core" count={3}>
        {['One', 'Two', 'Three'].map((n) => (
          <MarketplaceRow
            key={n} title={n} leading={n === 'One' ? <Checkbox aria-label={n} checked={false} onChange={noop} /> : undefined}
            badges={<Badge tone="neutral">Public</Badge>} meta="UCSF · v0.1.0"
            description="A description on one line"
            trailing={<Button size="sm" variant="secondary" aria-label={'Add ' + n}>Add</Button>}
          />
        ))}
      </MarketplaceSection>
    </MarketplaceDialog>
  );
}

const page = location.hash.slice(1);
createRoot(document.getElementById('root')).render(
  page === 'rows' ? <Rows /> : page === 'market' ? <Market /> : page === 'write' ? <CustomSkillModal onClose={noop} onSaved={noop} /> : <AddSkillModal onClose={noop} onSaved={noop} />
);
`;

const PREVIEW = {
  status: 'needsChoice',
  planId: 'plan-1',
  preview: {
    kind: 'bundle',
    id: 'hyperframes',
    displayName: 'HyperFrames',
    version: '0.8.12',
    entryPoint: 'hyperframes',
    groups: {},
    components: Array.from({ length: 12 }, (_, i) => ({
      name: `skill-${i}`,
      description: 'A component',
      directory: `skill-${i}`,
      group: 'core',
      entryPoint: i === 0,
    })),
    evidence: 'codexPlugin',
    ambiguity: null,
    source: { url: null, reference: null, resolvedCommit: null, installer: null },
    shadows: [],
    fileCount: 12,
  },
};

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

// 90s a test: each one launches a page in a Chromium other suites are loading at the same time.
describe('Skills and BAAM on the fill rule (R-27)', { timeout: 90_000 }, () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Skills fill tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"', 'process.env.ALPHA': 'false' },
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
    // The area's own sheets are imported by the components, which the bundle's loader drops.
    css += readFileSync(resolve(here, 'skills.css'), 'utf8');
    css += readFileSync(resolve(here, '../baam/marketplace.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(
    hash: 'rows' | 'market' | 'add' | 'write',
    mode: 'light' | 'dark',
    options: { forcedColors?: boolean; ready: string }
  ): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    if (options.forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    // The later route wins in Playwright, so the page goes first and the lookup answer second.
    await tab.route('http://skills.test/**', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'text/html',
        body:
          `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
          `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`,
      })
    );
    await tab.route('**/skills/packages/preview', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(PREVIEW),
      })
    );
    await tab.goto(`http://skills.test/#${hash}`);
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector(options.ready);
    // Let every colour transition settle before reading a colour.
    await tab.waitForTimeout(300);
    return tab;
  }

  /** The computed colour a probe paints for `value` (a token, in that place). */
  const paint = (tab: Page, value: string) =>
    tab.evaluate((v) => {
      const probe = document.createElement('div');
      probe.style.backgroundColor = v;
      document.body.append(probe);
      const out = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return out;
    }, value);

  const style = (tab: Page, selector: string, pseudo?: string) =>
    tab.evaluate(
      ([s, p]) => {
        const el = document.querySelector(s as string)!;
        const cs = getComputedStyle(el, (p as string) || null);
        const box = el.getBoundingClientRect();
        return {
          ground: cs.backgroundColor,
          edge: cs.borderTopColor,
          edgeWidth: cs.borderTopWidth,
          edgeStyle: cs.borderTopStyle,
          bottomWidth: cs.borderBottomWidth,
          bottomColor: cs.borderBottomColor,
          radius: cs.borderTopLeftRadius,
          shadow: cs.boxShadow,
          content: cs.content,
          overflowY: cs.overflowY,
          maxHeight: cs.maxHeight,
          size: [box.width, box.height],
        };
      },
      [selector, pseudo ?? ''] as const
    );

  const shoot = async (tab: Page, name: string) => {
    if (!existsSync(RUN_DIR)) return;
    mkdirSync(SHOTS, { recursive: true });
    await tab.screenshot({ path: join(SHOTS, `${name}.png`) });
  };

  it('draws no line under a flat list’s rows, keeps the edge as room, and moves nothing', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open('rows', mode, { ready: '#ruled [data-skill-row]' });
      try {
        const flat = '#flat [data-skill-row]:first-child';
        const ruled = '#ruled [data-skill-row]:first-child';
        // The control: the shared recipe still draws its straight line on a list that has not
        // opted out, so the rule below is the opt-out's doing and not a missing recipe.
        expect((await style(tab, ruled, '::after')).content).not.toBe('none');
        expect((await style(tab, flat, '::after')).content, `${mode} flat first row`).toBe('none');
        expect((await style(tab, '#flat [data-skill-row]:nth-child(2)', '::after')).content).toBe(
          'none'
        );
        // Only the line goes: the 1px transparent edge stays, so no row is a pixel shorter.
        const a = await style(tab, flat);
        const b = await style(tab, ruled);
        expect(a.bottomWidth).toBe('1px');
        expect(a.bottomColor).toBe(await paint(tab, 'transparent'));
        expect(a.size[1]).toBe(b.size[1]);
        await shoot(tab, `rows-${mode}`);
      } finally {
        await tab.close();
      }
    }
  });

  it('gives the line back under forced colours, where the hover wash that told rows apart is gone', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('rows', 'light', {
      forcedColors: true,
      ready: '#flat [data-skill-row]',
    });
    try {
      expect((await style(tab, '#flat [data-skill-row]:first-child', '::after')).content).not.toBe(
        'none'
      );
      // The last row of a list never had one.
      expect((await style(tab, '#flat [data-skill-row]:last-child', '::after')).content).toBe(
        'none'
      );
    } finally {
      await tab.close();
    }
  });

  it('keeps the marketplace list’s rows flat and its scroll edge, in both modes', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open('market', mode, { ready: '[data-marketplace-row]' });
      try {
        const row = '[data-marketplace-row]:first-of-type';
        expect((await style(tab, row, '::after')).content, `${mode} row`).toBe('none');
        // The list's top edge is structural: rows scroll under the search and the chips there.
        const list = await style(tab, '[data-testid="marketplace-list"]');
        expect(list.edgeWidth).toBe('1px');
        expect(list.edgeStyle).toBe('solid');
        expect(list.edge).not.toBe(await paint(tab, 'transparent'));
        await shoot(tab, `market-${mode}`);
      } finally {
        await tab.close();
      }
    }
  });

  it('keeps a row’s Add button visible when the row washes under the pointer', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open('market', mode, { ready: '[data-marketplace-row] button' });
      try {
        const fill = await paint(tab, 'var(--background-fill)');
        const strong = await paint(tab, 'var(--background-fill-strong)');
        const deeper = await paint(tab, 'var(--background-strong)');
        const button = '[data-marketplace-row]:nth-of-type(2) button';
        // At rest a column of Add buttons is the quiet fill, not a column of heavier boxes.
        expect((await style(tab, button)).ground, `${mode} rest`).toBe(fill);

        // The row's wash is the rest fill's value to within a shade, so on it the button's box
        // would have vanished (1.00:1). It steps one deeper while its row is washed.
        await tab.hover('[data-marketplace-row]:nth-of-type(2) [data-marketplace-title]');
        await tab.waitForTimeout(250);
        expect((await style(tab, button)).ground, `${mode} row hovered`).toBe(strong);
        expect((await style(tab, button)).ground).not.toBe(fill);
        await shoot(tab, `market-row-hover-${mode}`);

        // And its own hover and press go one deeper again, so it still answers the pointer.
        await tab.hover(button);
        await tab.waitForTimeout(250);
        expect((await style(tab, button)).ground, `${mode} button hovered`).toBe(deeper);

        // A row the pointer is not on keeps the quiet fill.
        await tab.mouse.move(5, 5);
        await tab.waitForTimeout(250);
        expect((await style(tab, '[data-marketplace-row]:nth-of-type(3) button')).ground).toBe(
          fill
        );
      } finally {
        await tab.close();
      }
    }
  });

  it('starts a leading checkbox’s visible box on the column edge the search, the chips and the group label share', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('market', 'light', { ready: '.br-checkbox-box' });
    try {
      const edges = await tab.evaluate(() => {
        const left = (el: Element | null) => el!.getBoundingClientRect().left;
        const label = document.querySelector('.br-marketplace-section-label')!;
        return {
          search: left(document.querySelector('input[aria-label="Search extensions"]')),
          // The label's text starts after its own 12px of padding.
          label: left(label) + parseFloat(getComputedStyle(label).paddingLeft),
          box: left(document.querySelector('.br-checkbox-box')),
          title: left(document.querySelector('[data-marketplace-row] [data-marketplace-title]')),
          boxRight: document.querySelector('.br-checkbox-box')!.getBoundingClientRect().right,
        };
      });
      // The 24px hit target's 4px inset used to leave the box at 421 against 417.
      expect(edges.box).toBe(edges.search);
      expect(edges.box).toBe(edges.label);
      // And the gap from the box to the name is what it was.
      expect(edges.title - edges.boxRight).toBe(16);
    } finally {
      await tab.close();
    }
  });

  it('draws the Add skill drop zone as the one plated fill, with no resting outline', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open('add', mode, { ready: '.br-skill-dropzone' });
      try {
        const zone = await style(tab, '.br-skill-dropzone');
        expect(zone.ground, `${mode} rests on the fill`).toBe(
          await paint(tab, 'var(--background-fill)')
        );
        // The edge stays a border, transparent: forced colours draw it, and nothing moves.
        expect(zone.edgeWidth).toBe('1px');
        expect(zone.edge).toBe(await paint(tab, 'transparent'));
        expect(zone.radius).toBe('12px');
        expect(zone.shadow).toBe('none');
        // 24px of air above and below a 32px plate and one 20px line, 8px apart, plus the edge.
        expect(zone.size[1]).toBe(24 + 32 + 8 + 20 + 24 + 2);

        // The plate is the page ground, one step back from the fill, and not an outlined box.
        const plate = await style(tab, '.br-skill-drop-plate');
        expect(plate.size).toEqual([32, 32]);
        expect(plate.radius).toBe('8px');
        expect(plate.ground).toBe(await paint(tab, 'var(--background-default)'));
        expect(plate.ground).not.toBe(zone.ground);
        expect(plate.edgeWidth).toBe('0px');

        await tab.hover('.br-skill-dropzone');
        await tab.waitForTimeout(250);
        expect((await style(tab, '.br-skill-dropzone')).ground, `${mode} hover`).toBe(
          await paint(tab, 'var(--background-fill-strong)')
        );
        await shoot(tab, `add-${mode}`);
      } finally {
        await tab.close();
      }
    }
  });

  it('firms the drop zone’s edge for a file held over it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('add', 'light', { ready: '.br-skill-dropzone' });
    try {
      await tab.mouse.move(5, 5);
      await tab.dispatchEvent('.br-skill-dropzone', 'dragover');
      await tab.waitForTimeout(250);
      const held = await style(tab, '.br-skill-dropzone');
      expect(held.ground).toBe(await paint(tab, 'var(--background-fill-strong)'));
      const strongEdge = await tab.evaluate(() => {
        const probe = document.createElement('div');
        probe.style.borderTop = '1px solid var(--border-strong)';
        document.body.append(probe);
        return getComputedStyle(probe).borderTopColor;
      });
      expect(held.shadow).toContain(strongEdge);
      expect(held.shadow).toContain('inset');
      // An inset ring draws on the box, so the edge never moves the plate.
      expect((await style(tab, '.br-skill-drop-plate')).size).toEqual([32, 32]);
      expect(held.size[1]).toBe(24 + 32 + 8 + 20 + 24 + 2);
    } finally {
      await tab.close();
    }
  });

  it('keeps the drop zone’s outline under forced colours through its transparent edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('add', 'light', { forcedColors: true, ready: '.br-skill-dropzone' });
    try {
      const zone = await style(tab, '.br-skill-dropzone');
      // A transparent border is drawn in the system ink there, and a `border-0` would not be.
      expect(zone.edgeWidth).toBe('1px');
      expect(zone.edge).not.toBe('rgba(0, 0, 0, 0)');
      // The held-over state cannot lean on a box-shadow there (forced colours drop it), so the
      // edge itself changes: dashed, in a colour, at the same 1px so nothing moves.
      expect(zone.edgeStyle).toBe('solid');
      await tab.dispatchEvent('.br-skill-dropzone', 'dragover');
      await tab.waitForTimeout(150);
      const held = await style(tab, '.br-skill-dropzone');
      expect(held.edgeStyle).toBe('dashed');
      expect(held.edgeWidth).toBe('1px');
      expect(held.size).toEqual(zone.size);
    } finally {
      await tab.close();
    }
  });

  it('sets the help glyph the same distance from its label in both skill dialogs', async (ctx) => {
    if (!browser) return ctx.skip();
    const gaps: number[] = [];
    for (const [hash, ready] of [
      ['add', '#skill-source-url'],
      ['write', '#custom-skill-md'],
    ] as const) {
      const tab = await open(hash, 'light', { ready });
      try {
        gaps.push(
          await tab.evaluate(() => {
            const label = document.querySelector('[role="dialog"] label')!;
            const range = document.createRange();
            range.selectNodeContents(label);
            const glyph = document.querySelector('[role="dialog"] button[aria-label^="About"]')!;
            return glyph.getBoundingClientRect().left - range.getBoundingClientRect().right;
          })
        );
      } finally {
        await tab.close();
      }
    }
    // "From a repository" used to sit 4px further from its glyph than "SKILL.md" (`gap-1`).
    expect(gaps[0]).toBe(gaps[1]);
  });

  it('tells the lookup result from the drop zone by space, and holds its skills in one scrolling well', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open('add', mode, { ready: '#skill-source-url' });
      try {
        await tab.fill('#skill-source-url', 'https://github.com/heygen-com/hyperframes');
        await tab.getByRole('button', { name: 'Look up' }).click();
        await tab.waitForSelector('.br-skill-preview');
        await tab.waitForTimeout(250);

        const section = await style(tab, '.br-skill-preview');
        // No rule above it: the old section drew a hairline between itself and the zone.
        expect(section.edgeWidth).toBe('0px');
        const gap = await tab.evaluate(
          () =>
            document.querySelector('.br-skill-preview')!.getBoundingClientRect().top -
            document.querySelector('.br-skill-dropzone')!.getBoundingClientRect().bottom
        );
        // The body's own 16px gap and nothing else (the old `pt-3` after a rule made it 12 plus 1).
        expect(gap).toBe(16);

        const well = await style(tab, '.br-skill-components');
        expect(well.ground, `${mode} well`).toBe(await paint(tab, 'var(--background-fill)'));
        expect(well.edgeWidth).toBe('1px');
        expect(well.edge).toBe(await paint(tab, 'transparent'));
        expect(well.radius).toBe('8px');
        // Twelve skills overflow 140px, and the scroller says so with an edge to scroll inside.
        expect(well.overflowY).toBe('auto');
        expect(well.maxHeight).toBe('140px');
        expect(well.size[1]).toBe(140);
        await shoot(tab, `add-preview-${mode}`);
      } finally {
        await tab.close();
      }
    }
  });
});
