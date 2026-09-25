// @vitest-environment node
/**
 * Let in's saved-code view, measured in a real layout engine (QA Q3-35).
 *
 * jsdom lays nothing out, so `LetInDialog.test.tsx` can only say which blocks stay when the
 * joiner arrives. The defect this file exists for was geometry: the status note went from "Code
 * saved. Gina is in as soon as Gina's Crew checks in; you can close this." (two lines in the
 * 400px dialog) to "Gina joined ito-lab" (one), and the hint row under the teams went from "You
 * can add Gina to a team once Gina joins." to nothing when no team offers a choice of channels.
 * The top-anchored dialog shrank, and "Add to {team}" moved out from under the host's pointer
 * just as it enabled. Every jsdom assertion passed.
 *
 * The real `SavedCodeStatus` and `NextStepHint` are rendered to markup (react-dom/server) and laid
 * out in Chromium with the real `dialogs.css`, in the 400px dialog's body column. Tailwind does
 * not run here, so the utilities they use are written out below at `main.css`'s values — and a
 * class the table does not know fails the suite, so a new layout utility cannot slip in
 * unmeasured.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { letInCopy } from './copy';
import { nextStepReserve, NextStepHint, SavedCodeStatus } from './LetInDialog';

const dialogsDir = dirname(fileURLToPath(import.meta.url));

/** `ModalShell`'s `sm` width, less its body's `px-4` on each side. */
const BODY = 400 - 2 * 16;

const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, BlinkMacSystemFont,
    'Segoe UI', Roboto, sans-serif;
}
body { margin: 0; font: 14px/20px var(--font-body); }
* { box-sizing: border-box; }
p { margin: 0; }`;

/**
 * The Tailwind utilities that move a box, at `main.css`'s values (spacing 4px; text-supporting
 * 12/16; `border` is Tailwind's 1px with preflight's solid style).
 */
const UTILITIES: Record<string, string> = {
  flex: 'display: flex',
  'items-start': 'align-items: flex-start',
  'gap-2': 'gap: 8px',
  'px-3': 'padding-inline: 12px',
  'py-2.5': 'padding-block: 10px',
  border: 'border-width: 1px; border-style: solid',
  'mt-0.5': 'margin-top: 2px',
  'h-4': 'height: 16px',
  'w-4': 'width: 16px',
  'shrink-0': 'flex-shrink: 0',
  'min-w-0': 'min-width: 0',
  'flex-1': 'flex: 1 1 0%',
  '[overflow-wrap:anywhere]': 'overflow-wrap: anywhere',
  'text-supporting': 'font-size: 12px; line-height: 16px; font-weight: 400',
};

/** Classes that only paint, or that `dialogs.css` itself lays out. */
const PAINT_ONLY = [/^crew-(steady|reserve)/, /^(bg|text-text|border-border|rounded)-/, /^lucide/];

const escapeClass = (name: string) => name.replace(/[.:[\]&]/g, (c) => `\\${c}`);
const UTILITY_CSS = Object.entries(UTILITIES)
  .map(([name, rule]) => `.${escapeClass(name)} { ${rule}; }`)
  .join('\n');

/** Every class the markup carries that neither the table nor the paint-only list accounts for. */
function unmeasuredClasses(html: string): string[] {
  const names = new Set<string>();
  for (const match of html.matchAll(/class="([^"]*)"/g)) {
    for (const name of match[1].split(/\s+/)) if (name) names.add(name);
  }
  return [...names].filter(
    (name) => !(name in UTILITIES) && !PAINT_ONLY.some((pattern) => pattern.test(name))
  );
}

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

interface Measured {
  /** The block's height, as the dialog's column lays it out. */
  block: number;
  /** How many lines the sentence it shows really takes: one rect per line of its text. */
  lines: number;
}

describe('Let in keeps its height when the joiner arrives, in a real layout engine (Q3-35)', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome) — skipping the Let in geometry tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(dialogsDir, 'dialogs.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Each element laid out alone in the dialog's body column. */
  async function layOut(blocks: Record<string, ReactElement>): Promise<Record<string, Measured>> {
    const html = Object.entries(blocks)
      .map(
        ([key, block]) =>
          `<section data-block="${key}" style="width: ${BODY}px; margin-bottom: 40px">` +
          renderToStaticMarkup(block) +
          '</section>'
      )
      .join('');
    expect(unmeasuredClasses(html), 'a class with no measured rule').toEqual([]);
    const tab: Page = await browser!.newPage({ viewport: { width: 800, height: 1200 } });
    try {
      await tab.setContent(
        '<!doctype html><html><head><style>' +
          TOKENS +
          UTILITY_CSS +
          css +
          '</style></head><body>' +
          html +
          '</body></html>'
      );
      return await tab.evaluate(() => {
        const out: Record<string, { block: number; lines: number }> = {};
        for (const section of Array.from(document.querySelectorAll('section[data-block]'))) {
          const block = section.firstElementChild!;
          // The live note's words, or the hint's own sentence.
          const shown =
            section.querySelector('[role="status"] span') ??
            section.querySelector('.crew-reserve-shown');
          const words = document.createRange();
          if (shown) words.selectNodeContents(shown);
          out[(section as HTMLElement).dataset.block!] = {
            block: block.getBoundingClientRect().height,
            lines: shown && shown.textContent ? words.getClientRects().length : 0,
          };
        }
        return out;
      });
    } finally {
      await tab.close();
    }
  }

  for (const first of ['Gina', '@crew_gina']) {
    it(`keeps the status note as tall once ${first} joins`, async (ctx) => {
      if (!browser) return ctx.skip();
      const status = (joined: boolean) => (
        <SavedCodeStatus joined={joined} first={first} workspace="ito-lab" steady />
      );
      const measured = await layOut({ waiting: status(false), joined: status(true) });
      // The words really do shrink: two lines or more become one.
      expect(measured.waiting.lines).toBeGreaterThanOrEqual(2);
      expect(measured.joined.lines).toBe(1);
      // The note does not.
      expect(measured.joined.block).toBe(measured.waiting.block);
    });

    it(`keeps the hint row as tall once ${first} joins, even with nothing to say`, async (ctx) => {
      if (!browser) return ctx.skip();
      const reserve = nextStepReserve(first);
      const measured = await layOut({
        waiting: <NextStepHint text={letInCopy.addAfterJoin(first)} reserve={reserve} />,
        choices: <NextStepHint text={letInCopy.channelsWithTeam} reserve={reserve} />,
        nothing: <NextStepHint text={null} reserve={reserve} />,
      });
      expect(measured.waiting.lines).toBeGreaterThanOrEqual(1);
      expect(measured.nothing.lines).toBe(0);
      expect(measured.nothing.block).toBeGreaterThanOrEqual(16);
      expect(measured.choices.block).toBe(measured.waiting.block);
      expect(measured.nothing.block).toBe(measured.waiting.block);
    });
  }

  it('is exactly as tall as its tallest form, the bordered "Code saved" note', async (ctx) => {
    if (!browser) return ctx.skip();
    const measured = await layOut({
      tallest: <SavedCodeStatus joined={false} first="Gina" workspace="ito-lab" steady={false} />,
      joinedAlone: <SavedCodeStatus joined first="Gina" workspace="ito-lab" steady={false} />,
      joined: <SavedCodeStatus joined first="Gina" workspace="ito-lab" steady />,
    });
    // Alone, the joined note is shorter: the height the footer used to follow up the screen.
    expect(measured.joinedAlone.block).toBeLessThan(measured.tallest.block);
    expect(measured.joined.block).toBe(measured.tallest.block);
  });
});
