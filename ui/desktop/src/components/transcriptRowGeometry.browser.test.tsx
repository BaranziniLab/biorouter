// @vitest-environment node
/**
 * The transcript row, measured in a real layout engine (spec 3.6, 5.7).
 *
 * jsdom lays nothing out and never runs Tailwind, so a component test that
 * reads a height passes whether the rule exists or not. `TranscriptRow` is
 * rendered to markup and laid out in Chromium with its own authored stylesheet
 * (`transcript-row.css`) at `main.css`'s token values, inside the 760px chat
 * column. It pins the numbers the acceptance names: a 28px line, the glyph on
 * the column edge with the hover wash bleeding 8px, the label at 24px, and the
 * well's left edge on the label's.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { TranscriptRow, TranscriptRowSection } from './TranscriptRow';
import { Terminal } from './icons/app-icons';

const here = dirname(fileURLToPath(import.meta.url));
const COLUMN = 760;

/** `main.css`'s values for the tokens the row reads. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --text-secondary: 13px;
  --text-secondary--line-height: 18px;
  --text-supporting: 12px;
  --text-supporting--line-height: 16px;
  --radius-element: 8px;
  --text-muted: #635c54;
  --text-default: #1c1b19;
  --text-danger: #a51a14;
  --overlay-hover: rgba(0, 0, 0, 0.05);
  --background-focus: #e0e0dc;
  --border-focus: #6b6963;
  --background-well: #f5f5f3;
  --dur-fast-min: 95ms;
  --dur-fast: 125ms;
  --dur-fast-max: 175ms;
  --dur-slow: 525ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px var(--font-body); }
* { box-sizing: border-box; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }`;

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

interface Geometry {
  head: { left: number; height: number; right: number };
  glyph: { left: number; width: number; height: number };
  label: { left: number; right: number; height: number; clipped: boolean };
  chevron: { left: number; width: number } | null;
  meta: { right: number; clipped: boolean } | null;
  well: { left: number; top: number } | null;
}

describe('TranscriptRow in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the transcript row ' +
          'geometry tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(here, 'transcript-row.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function layOut(element: React.ReactElement, column = COLUMN): Promise<Geometry> {
    const html = renderToStaticMarkup(element);
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 600 } });
    try {
      await tab.setContent(
        `<!doctype html><html><head><style>${TOKENS}${css}</style></head><body>` +
          `<div id="column" style="width:${column}px;margin-left:100px">${html}</div></body></html>`
      );
      return await tab.evaluate(() => {
        const origin = document.getElementById('column')!.getBoundingClientRect().left;
        const q = (selector: string) => document.querySelector(selector);
        const head = q('.br-transcript-row-head')!.getBoundingClientRect();
        const glyph = q('.br-transcript-row-glyph')!.getBoundingClientRect();
        const labelNode = q('.br-transcript-row-label') as HTMLElement;
        const label = labelNode.getBoundingClientRect();
        const chevron = q('.br-transcript-row-chevron')?.getBoundingClientRect() ?? null;
        const metaNode = q('.br-transcript-row-meta') as HTMLElement | null;
        const well = q('.br-transcript-row-well')?.getBoundingClientRect() ?? null;
        return {
          head: { left: head.left - origin, height: head.height, right: head.right - origin },
          glyph: { left: glyph.left - origin, width: glyph.width, height: glyph.height },
          label: {
            left: label.left - origin,
            right: label.right - origin,
            height: label.height,
            clipped: labelNode.scrollWidth > labelNode.clientWidth,
          },
          chevron: chevron ? { left: chevron.left - origin, width: chevron.width } : null,
          meta: metaNode
            ? {
                right: metaNode.getBoundingClientRect().right - origin,
                clipped: metaNode.scrollWidth > metaNode.clientWidth,
              }
            : null,
          well: well ? { left: well.left - origin, top: well.top } : null,
        };
      });
    } finally {
      await tab.close();
    }
  }

  it('is a 28px line: glyph on the column edge, label at 24px, the wash bleeding 8px', async () => {
    if (!browser) return;
    const g = await layOut(
      <TranscriptRow icon={Terminal} label="Ran npm test" defaultOpen>
        <TranscriptRowSection label="Output">
          <pre>ok</pre>
        </TranscriptRowSection>
      </TranscriptRow>
    );
    expect(g.head.height).toBe(28);
    expect(g.head.left).toBe(-8);
    expect(g.glyph).toEqual({ left: 0, width: 16, height: 16 });
    expect(g.label.left).toBe(24);
    expect(g.label.height).toBe(18);
    // The chevron trails the label, 8px after it, at the row's icon size.
    expect(g.chevron!.width).toBe(16);
    expect(g.chevron!.left).toBe(g.label.right + 8);
  });

  it("opens into a well whose left edge is the label's", async () => {
    if (!browser) return;
    const g = await layOut(
      <TranscriptRow icon={Terminal} label="Ran npm test" defaultOpen>
        <TranscriptRowSection label="Output">
          <pre>ok</pre>
        </TranscriptRowSection>
      </TranscriptRow>
    );
    expect(g.well!.left).toBe(g.label.left);
  });

  it('draws a static line at the same geometry, with no chevron', async () => {
    if (!browser) return;
    const g = await layOut(<TranscriptRow icon={Terminal} label="Preparing Shell" running />);
    expect(g.head.height).toBe(28);
    expect(g.glyph.left).toBe(0);
    expect(g.label.left).toBe(24);
    expect(g.chevron).toBeNull();
  });

  it('truncates a long label inside the column and never cuts the meta', async () => {
    if (!browser) return;
    const g = await layOut(
      <TranscriptRow
        icon={Terminal}
        label={`Running ${'very-long-argument '.repeat(20)}`}
        meta="· not gated by Biorouter"
      >
        <pre>x</pre>
      </TranscriptRow>,
      320
    );
    expect(g.head.height).toBe(28);
    expect(g.head.right).toBeLessThanOrEqual(320 + 0.5);
    expect(g.label.clipped).toBe(true);
    expect(g.label.height).toBe(18);
    expect(g.meta!.clipped).toBe(false);
  });
});
