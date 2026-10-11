// @vitest-environment node
/**
 * A Radix tab trigger's keyboard ring keeps one geometry in every colour mode (vision V4-16).
 *
 * A trigger (`ui/tabs.tsx`) is a 36px box with no side padding round a 20px label, and the
 * settings band centres it in 44px, 4px below the window's top edge. Its focus mark is a 2px ring
 * on `::before`, 28px tall and 6px past each side, so it stands 4px clear of the glyphs and inside
 * the trigger's height. Under `prefers-contrast: more` and forced colours the OS ring block used
 * to switch that ring off and let the 2px outline at a 2px offset draw instead: round the BOX, so
 * it hugged the label ("Chat" touched it) and its top edge left the window. Now the ring stays and
 * takes the OS colour (`--ring`, or the user's `Highlight` in forced colours) and the outline
 * stands down.
 *
 * That is geometry and paint, so it is measured in Chromium: every top level unlayered rule and
 * media block in main.css that names `[role='tab']`, over markup shaped like the settings band,
 * focused with a real Tab, in normal colours, more contrast and forced colours.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, 'main.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');

/** Top level rules and at-rules that name a tab, leaving cascade layers out (none is needed). */
function tabRules(): string {
  const chunks: string[] = [];
  let depth = 0;
  let from = 0;
  for (let i = 0; i < MAIN_CSS.length; i += 1) {
    const ch = MAIN_CSS[i];
    if (ch === '{') depth += 1;
    else if (ch === '}') {
      depth -= 1;
      if (depth === 0) {
        chunks.push(MAIN_CSS.slice(from, i + 1).trim());
        from = i + 1;
      }
    } else if (ch === ';' && depth === 0) {
      from = i + 1;
    }
  }
  return chunks
    .filter((chunk) => chunk.includes("[role='tab']") && !chunk.startsWith('@layer'))
    .join('\n');
}

/** The first value main.css gives `--name` (the base `:root` / `@theme` one, light mode). */
function firstToken(name: string): string {
  const match = MAIN_CSS.match(new RegExp(`\\s${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`main.css does not declare ${name}`);
  return match[1].trim();
}

const TOKENS = [
  '--border-focus',
  '--ring',
  '--color-neutral-600',
  '--radius-element',
  '--text-default',
  '--accent-bar',
]
  .map((name) => `${name}: ${firstToken(name)};`)
  .join(' ');

/** The settings band: a 44px strip at the window's top edge, 36px triggers centred, gap-5. */
const html = `<!doctype html><html><head><style>
  :root { ${TOKENS} }
  body { margin: 0; background: #fff; color: #222; font: 14px/20px Arial, sans-serif; }
  .band { height: 44px; display: flex; align-items: center; padding-inline: 24px; }
  .list { display: inline-flex; gap: 20px; }
  .list > button {
    position: relative; display: inline-flex; align-items: center; height: 36px;
    padding: 0; border: 0; background: none; font: inherit; color: inherit;
  }
${tabRules()}
</style></head><body><div class="band"><div class="list" role="tablist">
  <button role="tab" data-orientation="horizontal" data-state="active">Settings</button>
  <button role="tab" data-orientation="horizontal" data-state="inactive">Models</button>
  <button role="tab" data-orientation="horizontal" data-state="inactive" id="chat">Chat</button>
  <button role="tab" data-orientation="horizontal" data-state="inactive">App</button>
</div></div></body></html>`;

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

type Measure = {
  focused: boolean;
  outline: string;
  ringContent: string;
  ringWidth: string;
  ringColour: string;
  highlight: string;
  ring: { left: number; right: number; top: number; bottom: number };
  label: { left: number; right: number };
  band: { top: number };
};

const measure = (page: Page): Promise<Measure> =>
  page.evaluate(() => {
    const chat = document.getElementById('chat')!;
    const cs = getComputedStyle(chat);
    const before = getComputedStyle(chat, '::before');
    const box = chat.getBoundingClientRect();
    const range = document.createRange();
    range.selectNodeContents(chat);
    const label = range.getBoundingClientRect();
    const probe = document.createElement('div');
    probe.style.cssText = 'color: Highlight; forced-color-adjust: none';
    document.body.append(probe);
    const highlight = getComputedStyle(probe).color;
    probe.remove();
    // An absolutely positioned box resolves its insets to pixels against the trigger.
    const px = (v: string) => Number.parseFloat(v);
    return {
      focused: chat.matches(':focus-visible'),
      outline: cs.outlineStyle,
      ringContent: before.content,
      ringWidth: before.borderTopWidth,
      ringColour: before.borderTopColor,
      highlight,
      ring: {
        left: box.left + px(before.left),
        right: box.right - px(before.right),
        top: box.top + px(before.top),
        bottom: box.bottom - px(before.bottom),
      },
      label: { left: label.left, right: label.right },
      band: { top: document.querySelector('.band')!.getBoundingClientRect().top },
    };
  });

/** The token's colour as Chromium computes it (`--ring` is a var chain in main.css). */
const computedRing = (page: Page) =>
  page.evaluate(() => {
    const probe = document.createElement('div');
    probe.style.color = 'var(--ring)';
    document.body.append(probe);
    const colour = getComputedStyle(probe).color;
    probe.remove();
    return colour;
  });

const MODES = ['normal colours', 'more contrast', 'forced colours'] as const;

describe('a Radix tab trigger keeps its label-sized ring in every colour mode (V4-16)', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found, so the tab focus ring tests are skipped. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
  });

  afterAll(async () => {
    await browser?.close();
  });

  it.for(MODES)('%s', async (mode, ctx) => {
    if (!browser) return ctx.skip();
    const page = await browser.newPage({ viewport: { width: 520, height: 120 } });
    try {
      if (mode === 'more contrast') await page.emulateMedia({ contrast: 'more' });
      if (mode === 'forced colours') await page.emulateMedia({ forcedColors: 'active' });
      await page.setContent(html);
      for (let i = 0; i < 3; i += 1) await page.keyboard.press('Tab');
      const m = await measure(page);
      expect(m.focused, 'Chat has keyboard focus').toBe(true);

      // One ring, and it is the pseudo ring: no outline round the box.
      expect(m.outline, `${mode}: no outline round the trigger`).toBe('none');
      expect(m.ringContent, `${mode}: the ::before ring is drawn`).not.toBe('none');
      expect(m.ringWidth).toBe('2px');

      // Its geometry: 28px tall inside the trigger, 6px past each side, inside the window.
      expect(m.ring.bottom - m.ring.top, `${mode}: the ring is 28px tall`).toBeCloseTo(28, 0);
      expect(
        m.ring.top - m.band.top,
        `${mode}: the ring's top is inside the window`
      ).toBeGreaterThanOrEqual(4);
      const clearLeft = m.label.left - (m.ring.left + 2);
      const clearRight = m.ring.right - 2 - m.label.right;
      expect(clearLeft, `${mode}: inner edge clear of the label's start`).toBeGreaterThanOrEqual(4);
      expect(clearRight, `${mode}: inner edge clear of the label's end`).toBeGreaterThanOrEqual(4);

      // Its colour: the neutral focus edge, the OS ring, or the user's own Highlight.
      if (mode === 'more contrast') expect(m.ringColour).toBe(await computedRing(page));
      if (mode === 'forced colours') expect(m.ringColour).toBe(m.highlight);
    } finally {
      await page.close();
    }
  });
});
