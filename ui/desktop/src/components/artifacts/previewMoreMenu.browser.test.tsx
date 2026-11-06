// @vitest-environment node
/**
 * The preview panel's `⋯` shows that its menu is open (H4: only a click leaves a mark), measured
 * in a real engine on the real stylesheet.
 *
 * The `⋯` is a Radix menu trigger inside a `PreviewTip` (a Radix label tooltip). Radix merges the
 * TOOLTIP's `data-state` ("closed", "delayed-open", "instant-open") over the menu's own, because
 * the outer `Slot` hands its props to the inner trigger, which spreads them after its own. So
 * `.br-preview-icon-button[data-state='open']` never matched: the open `⋯` looked like a resting
 * one the moment the pointer left it (the trap `pickers.css` records for the composer chips).
 * The open look is keyed on `aria-expanded`, which only the menu sets, and paints the step every
 * ghost trigger whose surface a click opened wears: `--background-fill-strong` with
 * `--text-default`, held whether or not the pointer is on it.
 *
 * ⚠ `aria-haspopup` is part of the selector. The fold toggle beside the `⋯` is a
 * `.br-preview-icon-button` too and carries `aria-expanded` for the preview it folds, which is
 * open nearly all the time; keyed on `aria-expanded` alone, it would have sat on the fill for
 * good.
 *
 * The header is drawn with the real `PreviewTip`, `PreviewIconButton`, `DropdownMenu` and
 * `buttonVariants`, composed as `ArtifactViewer` composes them (the viewer itself is too large to
 * bundle here). `styles/main.css` is compiled by Tailwind with the classes the bundle uses, so the
 * ghost button's own layered utilities (`bg-transparent`, `tint-interactive`) are in the cascade
 * the authored rule has to beat, and `preview-panel.css` is appended as Vite would. Colours are
 * compared with probes that paint the token itself.
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
import { cn } from '../../utils';
import { ChevronUp, MoreHorizontal } from '../icons/app-icons';
import { AppTooltipProvider } from '../ui/Tooltip';
import { buttonVariants } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { PreviewIconButton, PreviewTip } from './PreviewBar';

function App() {
  return (
    <AppTooltipProvider>
      <div className="br-preview-actions" style={{ position: 'fixed', top: 40, right: 40 }}>
        {/* The header's ⋯, composed as ArtifactViewer composes it. */}
        <DropdownMenu>
          <PreviewTip content="More actions">
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label="More actions"
                data-testid="artifact-more-actions"
                className={cn(
                  buttonVariants({ variant: 'ghost', shape: 'round' }),
                  'br-preview-icon-button no-drag'
                )}
              >
                <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
              </button>
            </DropdownMenuTrigger>
          </PreviewTip>
          <DropdownMenuContent align="end" className="min-w-[180px] max-w-[320px]">
            <DropdownMenuItem>Reveal in Finder</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <PreviewIconButton
          data-testid="artifact-fold-toggle"
          aria-expanded={true}
          className="br-preview-fold-toggle"
          label="Fold preview"
        >
          <ChevronUp className="h-4 w-4" aria-hidden="true" />
        </PreviewIconButton>
      </div>
    </AppTooltipProvider>
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

const MORE = '[data-testid="artifact-more-actions"]';
const FOLD = '[data-testid="artifact-fold-toggle"]';
const AWAY = { x: 20, y: 380 };

describe('the preview panel’s ⋯ shows its open menu (H4)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the preview ⋯ tests.');
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
    css =
      compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }])) +
      readFileSync(resolve(here, 'preview-panel.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Every finite animation and transition ended, so a reading is the control at rest. */
  const settle = (page: Page) =>
    page.evaluate(() =>
      Promise.all(
        document
          .getAnimations()
          .filter((a) => Number.isFinite(a.effect?.getComputedTiming().endTime))
          .map((a) => a.finished.catch(() => undefined))
      ).then(() => undefined)
    );

  async function load(mode: 'light' | 'dark'): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 600, height: 400 } });
    await page.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--sidebar)"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    await page.waitForSelector(MORE);
    await page.mouse.move(AWAY.x, AWAY.y);
    await settle(page);
    return page;
  }

  /** The computed value a probe paints for `value` as a background, or as an ink. */
  const paint = (page: Page, value: string, as: 'background' | 'ink' = 'background') =>
    page.evaluate(
      ([v, kind]) => {
        const probe = document.createElement('div');
        if (kind === 'ink') probe.style.color = v;
        else probe.style.backgroundColor = v;
        document.body.append(probe);
        const cs = getComputedStyle(probe);
        const out = kind === 'ink' ? cs.color : cs.backgroundColor;
        probe.remove();
        return out;
      },
      [value, as] as const
    );

  const look = (page: Page, selector: string) =>
    page.evaluate((s) => {
      const el = document.querySelector(s)!;
      const cs = getComputedStyle(el);
      return {
        ground: cs.backgroundColor,
        image: cs.backgroundImage,
        ink: cs.color,
        state: el.getAttribute('data-state'),
        expanded: el.getAttribute('aria-expanded'),
      };
    }, selector);

  it('records the trap: the tooltip’s data-state, not the menu’s, is on the open ⋯', async (ctx) => {
    if (!browser) return ctx.skip();
    const page = await load('light');
    try {
      await page.click(MORE);
      await page.waitForSelector('[role="menu"]');
      const open = await look(page, MORE);
      expect(open.expanded, 'the menu says it is open').toBe('true');
      expect(open.state, 'the data-state is the tooltip’s').not.toBe('open');
    } finally {
      await page.close();
    }
  });

  for (const mode of ['light', 'dark'] as const) {
    it(`${mode}: rests transparent and muted, and holds the kept fill while its menu is open`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await load(mode);
      try {
        const clear = await paint(page, 'transparent');
        const strong = await paint(page, 'var(--background-fill-strong)');
        const muted = await paint(page, 'var(--text-muted)', 'ink');
        const ink = await paint(page, 'var(--text-default)', 'ink');

        const rest = await look(page, MORE);
        expect(rest.ground, 'at rest: no ground').toBe(clear);
        expect(rest.ink, 'at rest: the muted ink').toBe(muted);

        await page.click(MORE);
        await page.waitForSelector('[role="menu"]');
        // On the open ⋯, then off it: the kept look holds either way.
        await settle(page);
        const pointed = await look(page, MORE);
        expect(pointed.ground, 'open, pointer on it: the kept fill').toBe(strong);
        expect(pointed.image, 'open, pointer on it: no hover tint over the fill').toBe('none');
        expect(pointed.ink).toBe(ink);
        await page.mouse.move(AWAY.x, AWAY.y);
        await settle(page);
        const away = await look(page, MORE);
        expect(away.ground, 'open, pointer elsewhere: the kept fill').toBe(strong);
        expect(away.ink).toBe(ink);

        // The fold toggle says `aria-expanded` about its preview and opens no surface.
        const fold = await look(page, FOLD);
        expect(fold.expanded).toBe('true');
        expect(fold.ground, 'the fold toggle is not a kept trigger').toBe(clear);

        await page.keyboard.press('Escape');
        await page.waitForSelector('[role="menu"]', { state: 'detached' });
        await page.mouse.move(AWAY.x, AWAY.y);
        await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
        await settle(page);
        const closed = await look(page, MORE);
        expect(closed.ground, 'closed again: no ground').toBe(clear);
        expect(closed.ink, 'closed again: the muted ink').toBe(muted);
      } finally {
        await page.close();
      }
    });
  }
});
