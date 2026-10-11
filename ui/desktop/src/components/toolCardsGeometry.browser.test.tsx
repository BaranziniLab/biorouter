// @vitest-environment node
/**
 * The cards of the transcript, measured in a real layout engine (spec 3.6, 5.7).
 *
 * jsdom lays nothing out and never evaluates `:hover`, `:focus-visible` or
 * `::before`, so a component test that reads a class passes whether the rule
 * behind it exists or not. The action card (approval, elicitation, secret
 * request) and the artifact card are rendered to markup and laid out in
 * Chromium with their own authored stylesheet (`tool-call.css`) at
 * `main.css`'s token values, inside the 760px chat column.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import MCPUIResourceRenderer from './MCPUIResourceRenderer';
import { ToolCallPreview } from './ToolCallPreview';
import type { EmbeddedResource, ToolPreview } from '../api';

const here = dirname(fileURLToPath(import.meta.url));
const COLUMN = 760;

/** `main.css`'s values for the tokens the cards read. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --text-supporting: 12px;
  --text-supporting--line-height: 16px;
  --radius-inner: 4px;
  --radius-element: 8px;
  --radius-container: 12px;
  --text-muted: #635c54;
  --text-default: #1c1b19;
  --overlay-hover: rgba(0, 0, 0, 0.05);
  --background-default: #ffffff;
  --background-fill: #f4f4f2;
  --background-fill-strong: #eaeae6;
  --background-focus: #e0e0dc;
  --border-subtle: #e4e3df;
  --border-focus: #6b6963;
  --dur-fast-min: 95ms;
  --dur-fast: 125ms;
  --dur-fast-max: 175ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px var(--font-body); }
* { box-sizing: border-box; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-supporting { font-size: 12px; line-height: 16px; }
.block { display: block; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.min-w-0 { min-width: 0; }
.flex-1 { flex: 1 1 0%; }
/* The border utilities a hairline would be written with, layered as Tailwind's
   are, so a rule drawn by a utility on the preview shows up here too. */
@layer utilities {
  .border-b { border-bottom-style: solid; border-bottom-width: 1px; }
  .border-t { border-top-style: solid; border-top-width: 1px; }
  .border-border-subtle { border-color: var(--border-subtle); }
}
/* main.css's entrance: the card must be able to opt out of it. */
.br-enter { animation: br-enter 175ms cubic-bezier(0.24, 1, 0.4, 1) both; }
@keyframes br-enter { from { opacity: 0; } to { opacity: 1; } }`;

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

const artifact = {
  type: 'resource',
  resource: {
    uri: 'ui://autovisualiser/chart',
    mimeType: 'text/html',
    text: '<!doctype html><html><body><h1>Chart</h1></body></html>',
  },
} as unknown as EmbeddedResource & { type: 'resource' };

describe('the transcript cards in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the transcript card ' +
          'geometry tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(here, 'tool-call.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(html: string) {
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 600 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${TOKENS}${css}</style></head><body>` +
        `<div id="column" style="width:${COLUMN}px;margin-left:100px">${html}</div></body></html>`
    );
    return tab;
  }

  it('draws an action card at radius 12 on the subtle fill with no outline, 16px in, actions at the bottom right', async () => {
    if (!browser) return;
    const tab = await open(
      `<div class="biorouter-message-content br-action-card br-enter" id="card">` +
        `<div class="text-label">Run Shell?</div>` +
        `<div id="actions" style="display:flex;justify-content:flex-end"><button id="go">Allow once</button></div>` +
        `</div>`
    );
    try {
      const m = await tab.evaluate(() => {
        const card = document.getElementById('card')!;
        const style = getComputedStyle(card);
        const box = card.getBoundingClientRect();
        const go = document.getElementById('go')!.getBoundingClientRect();
        return {
          width: box.width,
          radius: style.borderTopLeftRadius,
          border: `${style.borderTopWidth} ${style.borderTopStyle}`,
          borderColor: style.borderTopColor,
          padding: style.padding,
          gap: style.rowGap,
          shadow: style.boxShadow,
          ground: style.backgroundColor,
          // The button's right edge sits on the card's content edge: 16px in.
          buttonInset: box.right - go.right,
        };
      });
      expect(m.width).toBe(COLUMN);
      expect(m.radius).toBe('12px');
      // Fills before lines (R-27): the 1px border stays, transparent, so no
      // geometry moves and forced colours still draw it.
      expect(m.border).toBe('1px solid');
      expect(m.borderColor).toBe('rgba(0, 0, 0, 0)');
      expect(m.padding).toBe('16px');
      expect(m.gap).toBe('12px');
      expect(m.shadow).toBe('none');
      expect(m.ground).toBe('rgb(244, 244, 242)');
      expect(m.buttonInset).toBe(17);
    } finally {
      await tab.close();
    }
  });

  it("steps the call's preview back to the page ground inside the filled card", async () => {
    if (!browser) return;
    const tab = await open(
      `<div class="br-action-card"><div class="br-tool-preview" id="inside">x</div></div>` +
        `<div class="br-tool-preview" id="alone">y</div>`
    );
    try {
      const ground = (id: string) =>
        tab.evaluate((target) => {
          const style = getComputedStyle(document.getElementById(target)!);
          return [style.backgroundColor, style.borderTopLeftRadius];
        }, id);
      expect(await ground('inside')).toEqual(['rgb(255, 255, 255)', '8px']);
      expect(await ground('alone')).toEqual(['rgb(244, 244, 242)', '8px']);
    } finally {
      await tab.close();
    }
  });

  it('draws the preview as one box with no visible line: no rule under its head or over the Truncated note (R-27)', async () => {
    if (!browser) return;
    const preview = {
      kind: 'fileEdit',
      path: '/work/design.R',
      added: 1,
      removed: 1,
      lines: [
        { kind: 'removed', text: 'fit <- DESeq(dds)' },
        { kind: 'added', text: 'fit <- DESeq(dds, test = "LRT")' },
      ],
      truncated: true,
    } as unknown as ToolPreview;
    // Tailwind's preflight zeroes every border (the InfoTip is a <button>, which
    // the browser would otherwise frame).
    const tab = await open(
      `<style>@layer base { *, ::before, ::after { border: 0 solid; } }</style>` +
        `<div class="br-action-card">${renderToStaticMarkup(<ToolCallPreview preview={preview} />)}</div>`
    );
    try {
      const m = await tab.evaluate(() => {
        const box = document.querySelector('.br-tool-preview') as HTMLElement;
        const style = getComputedStyle(box);
        // Every edge drawn inside the preview, the box included: width, style
        // and a colour that is not transparent.
        const visible = [box, ...box.querySelectorAll('*')].flatMap((node) => {
          const s = getComputedStyle(node);
          return (['Top', 'Right', 'Bottom', 'Left'] as const)
            .filter(
              (side) =>
                parseFloat(s.getPropertyValue(`border-${side.toLowerCase()}-width`)) > 0 &&
                s.getPropertyValue(`border-${side.toLowerCase()}-style`) !== 'none' &&
                s.getPropertyValue(`border-${side.toLowerCase()}-color`) !== 'rgba(0, 0, 0, 0)'
            )
            .map((side) => `${(node as HTMLElement).className || node.tagName} ${side}`);
        });
        const edge = (selector: string, side: 'top' | 'bottom') => {
          const s = getComputedStyle(box.querySelector(selector)!);
          return `${s.getPropertyValue(`border-${side}-width`)} ${s.getPropertyValue(`border-${side}-style`)}`;
        };
        return {
          box: `${style.borderTopWidth} ${style.borderTopStyle} ${style.borderTopColor}`,
          ground: style.backgroundColor,
          head: edge('.br-tool-preview-head', 'bottom'),
          note: edge('.br-tool-preview-note', 'top'),
          visible,
        };
      });
      // The edges stay, transparent, so forced colours still draw the box and
      // the head's division; nothing shows at rest.
      expect(m.box).toBe('1px solid rgba(0, 0, 0, 0)');
      expect(m.ground).toBe('rgb(255, 255, 255)');
      expect(m.head).toBe('1px solid');
      expect(m.note).toBe('1px solid');
      expect(m.visible).toEqual([]);
    } finally {
      await tab.close();
    }
  });

  it('plays the entrance for a card that arrives live and not for one that mounts at rest', async () => {
    if (!browser) return;
    const card = `<div class="br-action-card br-enter">Run Shell?</div>`;
    const live = await open(card);
    const still = await open(`<div data-motion-still="">${card}</div>`);
    try {
      const nameOf = (page: typeof live) =>
        page.evaluate(
          () => getComputedStyle(document.querySelector('.br-action-card')!).animationName
        );
      expect(await nameOf(live)).toBe('br-enter');
      expect(await nameOf(still)).toBe('none');
    } finally {
      await live.close();
      await still.close();
    }
  });

  it('draws the artifact card across the 760 column on the fill: a 32px tile, the title over its kind, no shadow', async () => {
    if (!browser) return;
    const tab = await open(
      renderToStaticMarkup(<MCPUIResourceRenderer content={artifact} onOpenArtifact={() => {}} />)
    );
    try {
      const m = await tab.evaluate(() => {
        const card = document.querySelector('.br-artifact-card') as HTMLElement;
        const style = getComputedStyle(card);
        const box = card.getBoundingClientRect();
        const tileNode = card.querySelector('.br-artifact-card-tile')!;
        const tile = tileNode.getBoundingClientRect();
        const lines = [...card.querySelectorAll('.block')].map((node) =>
          node.getBoundingClientRect()
        );
        return {
          width: box.width,
          radius: style.borderTopLeftRadius,
          border: `${style.borderTopWidth} ${style.borderTopStyle}`,
          borderColor: style.borderTopColor,
          ground: style.backgroundColor,
          shadow: style.boxShadow,
          tile: { width: tile.width, height: tile.height },
          tileGround: getComputedStyle(tileNode).backgroundColor,
          titleAboveKind: lines.length === 2 && lines[0].bottom <= lines[1].top,
          kind: card.getAttribute('data-artifact-kind'),
          kindText: card.querySelectorAll('.block')[1]?.textContent,
        };
      });
      expect(m.width).toBe(COLUMN);
      expect(m.radius).toBe('12px');
      expect(m.border).toBe('1px solid');
      expect(m.borderColor).toBe('rgba(0, 0, 0, 0)');
      expect(m.ground).toBe('rgb(244, 244, 242)');
      expect(m.shadow).toBe('none');
      expect(m.tile).toEqual({ width: 32, height: 32 });
      // A fill never sits on a fill: the tile steps back to the page ground.
      expect(m.tileGround).toBe('rgb(255, 255, 255)');
      expect(m.titleAboveKind).toBe(true);
      expect(m.kind).toBe('figure');
      expect(m.kindText).toBe('Figure');
    } finally {
      await tab.close();
    }
  });

  it('steps the artifact card to the strong fill on hover and draws the row focus on a Tab', async () => {
    if (!browser) return;
    const tab = await open(
      renderToStaticMarkup(<MCPUIResourceRenderer content={artifact} onOpenArtifact={() => {}} />)
    );
    try {
      const ground = () =>
        tab.evaluate(
          () => getComputedStyle(document.querySelector('.br-artifact-card')!).backgroundColor
        );
      expect(await ground()).toBe('rgb(244, 244, 242)');
      await tab.hover('.br-artifact-card');
      await tab.waitForTimeout(200);
      expect(await ground()).toBe('rgb(234, 234, 230)');

      await tab.mouse.move(900, 500);
      await tab.keyboard.press('Tab');
      // The ground transitions (95ms), so read it once it has settled.
      await tab.waitForTimeout(200);
      const focus = await tab.evaluate(() => {
        const style = getComputedStyle(document.querySelector('.br-artifact-card')!);
        return { ground: style.backgroundColor, shadow: style.boxShadow };
      });
      expect(focus.ground).toBe('rgb(224, 224, 220)');
      expect(focus.shadow).toContain('inset');
      expect(focus.shadow).toContain('2px');
    } finally {
      await tab.close();
    }
  });
});
