// @vitest-environment node
/**
 * Built apps: where the "No matching apps" tile sits, measured in a real layout engine
 * (vision finding V3-R7-2, re-reported in round 8).
 *
 * The same state, a filter that matches nothing, put its tile at y=192 on Built apps and at y=100
 * on Workflows (y=108 on Skills): Built apps drew the FULL empty state, a box with
 * `min-height: clamp(17rem, 48vh, 25rem)` that centres its tile, and the siblings draw the compact
 * one. jsdom lays nothing out, so `ApplicationsView.test.tsx` pins the branch and the class and
 * this file pins the pixels.
 *
 * The real `ApplicationsView` is bundled with esbuild (the daemon's `GET /apps` answered by a
 * stub), `styles/main.css` is compiled by Tailwind with the classes that bundle uses, and
 * Chromium paints it. The reference is the Workflows recipe, built from the same primitives in the
 * same frame (`ReadableContent` + `ScrollArea`, `px-6 pt-2`, then the compact tile), so the case
 * reads "Built apps puts the tile where Workflows does" rather than a number that a token change
 * would move.
 *
 * Set `VIEWS_A_SHOTS` to a directory to also write a screenshot of each state.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.VIEWS_A_SHOTS;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import ApplicationsView from './ApplicationsView';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { PageHeader } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import { ScrollArea } from '../ui/scroll-area';
import { EmptyState } from '../ui/empty-state';
import { ENTITY_ICONS } from '../icons/entity-icons';

/** Workflows' frame, with its no-match tile: the place Built apps has to match. */
function Reference() {
  return (
    <MainPanelLayout removeTopPadding>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <PageHeader title="Workflows" />
        <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6 pt-2">
          <ScrollArea className="h-full">
            <div className="relative h-full pb-6">
              <EmptyState
                icon={ENTITY_ICONS.application}
                title="No matching workflows"
                description="Try another search."
                compact
              />
            </div>
          </ScrollArea>
        </ReadableContent>
      </div>
    </MainPanelLayout>
  );
}

const reference = window.location.hash === '#reference';

createRoot(document.getElementById('root')!).render(
  <div style={{ height: '100vh' }}>
    {reference ? (
      <Reference />
    ) : (
      <MemoryRouter>
        <ApplicationsView />
      </MemoryRouter>
    )}
  </div>
);
`;

const APP = {
  id: 'cohort-explorer',
  title: 'Cohort Explorer',
  description: 'Explore a paired cohort.',
  kind: 'agentic',
  session_id: 'session-1',
  updated_at: 1750000000,
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

describe('Built apps, a filter with no match (V3-R7-2)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Built apps no-match tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      alias: { '@': resolve(here, '../..') },
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

  /** Opens the view (or the reference) at a window size, with the daemon's answer stubbed. */
  async function open(
    options: {
      apps?: unknown[];
      mode?: 'light' | 'dark';
      reference?: boolean;
      size?: { width: number; height: number };
    } = {}
  ): Promise<Page> {
    const { apps = [APP], mode = 'light', size = { width: 1100, height: 760 } } = options;
    const tab = await browser!.newPage({ viewport: size });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-canvas)"><div id="root"></div></body></html>`
    );
    // The daemon and the main process, stubbed on the page the bundle is about to run in.
    await tab.evaluate(
      ([list, asReference]) => {
        (window as unknown as { electron: unknown }).electron = {
          platform: 'darwin',
          getSecretKey: async () => 'test-secret',
          openExternal: async () => undefined,
        };
        window.fetch = (async () => ({
          ok: true,
          status: 200,
          json: async () => list,
          text: async () => '',
        })) as unknown as typeof window.fetch;
        if (asReference) window.location.hash = '#reference';
      },
      [apps, Boolean(options.reference)] as const
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector(options.reference ? '.br-empty-tile' : 'header');
    return tab;
  }

  /** The empty tile's top below the band's bottom edge: where the state's mark sits. */
  const tileBelowBand = (tab: Page) =>
    tab.evaluate(() => {
      const tile = document.querySelector('.br-empty-tile')!.getBoundingClientRect();
      const band = document.querySelector('header')!.getBoundingClientRect();
      return tile.top - band.bottom;
    });
  const emptySection = (tab: Page) =>
    tab.evaluate(() => {
      const box = document.querySelector('.br-empty-tile')!.closest('section')!;
      return {
        height: box.getBoundingClientRect().height,
        minHeight: getComputedStyle(box).minHeight,
      };
    });

  async function filterFor(tab: Page, text: string) {
    await tab.getByRole('searchbox').fill(text);
    await tab.waitForSelector('.br-empty-tile');
    await tab.waitForTimeout(150);
  }

  async function shot(tab: Page, name: string) {
    if (!SHOTS) return;
    mkdirSync(SHOTS, { recursive: true });
    await tab.screenshot({ path: resolve(SHOTS, `built-apps-${name}.png`) });
  }

  it('puts the no-match tile where Workflows puts its own, at two window sizes, light and dark', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      for (const size of [
        { width: 1440, height: 900 },
        { width: 1100, height: 760 },
      ]) {
        const label = `${mode} ${size.width}x${size.height}`;
        const sibling = await open({ reference: true, mode, size });
        const view = await open({ mode, size });
        try {
          await view.waitForSelector('[data-testid="app-row"]');
          await filterFor(view, 'nothing');
          expect(await view.getByRole('heading', { level: 2 }).textContent()).toBe(
            'No matching apps'
          );
          const reference = await tileBelowBand(sibling);
          // The compact tile is the scroll body's `pt-2` plus the section's `py-12`: 56 px under
          // the band. Built apps drew 148 (round 8: tile at 192 under a band that ends at 44).
          expect(reference, `${label} reference`).toBeGreaterThan(40);
          expect(reference, `${label} reference`).toBeLessThan(80);
          expect(await tileBelowBand(view), `${label} the same place as Workflows`).toBeCloseTo(
            reference,
            0
          );
          // Compact has no minimum height, so the section is just its content and padding.
          const { minHeight, height } = await emptySection(view);
          expect(minHeight, `${label} no min-height`).toBe('0px');
          expect(height, `${label} shorter than the full empty state’s 17rem floor`).toBeLessThan(
            272
          );
          if (size.width === 1440) await shot(view, `no-match-${mode}-${size.width}`);
        } finally {
          await view.close();
          await sibling.close();
        }
      }
    }
  }, 180_000);

  it('keeps the full empty state, and its place, when nothing is built and the filter is typed in', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ apps: [] });
    try {
      await tab.waitForSelector('.br-empty-tile');
      expect(await tab.getByRole('heading', { level: 2 }).textContent()).toBe('No apps built yet');
      const before = await tileBelowBand(tab);
      const full = await emptySection(tab);
      // The full state is the tall one: it keeps its 17rem floor and centres its tile in it.
      expect(Number.parseFloat(full.minHeight)).toBeGreaterThanOrEqual(272);
      expect(before).toBeGreaterThan(56);
      await shot(tab, 'empty-light-1100');

      await filterFor(tab, 'nothing');
      // Nothing to miss, so nothing to say "no match" about: same words, same tile, same place.
      expect(await tab.getByRole('heading', { level: 2 }).textContent()).toBe('No apps built yet');
      expect(await tileBelowBand(tab)).toBe(before);
    } finally {
      await tab.close();
    }
  }, 120_000);

  it('tells the two states apart: the full empty state sits lower than the no-match tile', async (ctx) => {
    if (!browser) return ctx.skip();
    const empty = await open({ apps: [] });
    const matched = await open();
    try {
      await empty.waitForSelector('.br-empty-tile');
      await matched.waitForSelector('[data-testid="app-row"]');
      await filterFor(matched, 'nothing');
      // The instrument sees the difference this file is about (84 to 92 px at the window sizes
      // the round measured), so a pass above is not a reading that cannot tell the two apart.
      expect(await tileBelowBand(empty)).toBeGreaterThan((await tileBelowBand(matched)) + 40);
    } finally {
      await empty.close();
      await matched.close();
    }
  }, 120_000);
});
