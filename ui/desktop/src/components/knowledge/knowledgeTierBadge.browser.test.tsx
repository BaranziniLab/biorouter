// @vitest-environment node
/**
 * The Knowledge band's tier badge is a filled chip on the band's ground (V3-R4-2, owner message 13,
 * R-27; design.md 9.11 rule 2), measured in a real layout engine.
 *
 * The badge is the one band chip that is also a button (`KbTierControl`, Crew's
 * `ClassificationBadge`). The chip inside is `PrivacyBadge`, which rests on `bg-background-muted`
 * (the rest fill's own value) so it reads the same everywhere else it is drawn. The band is the
 * sidebar's paint, a recessed ground where the rest fill is 1.03:1 off, so the badge read as plain
 * text and its neighbour, the neutral format chip, did not: the band's rule is that a filled thing
 * on a recessed ground rests on the strong fill. This lays the real band, `KbTierControl` and a
 * neutral `Badge` out in Chromium with the band, recessed-ground and badge rules cut from
 * `main.css` at the token values the stylesheet declares (read through
 * `scripts/lib/theme-tokens.mjs`, the way `check-contrast.mjs` does), in light and dark, with the
 * utilities in the layer the app puts them in. It pins:
 *  - the badge rests on `--background-fill-strong`, a step off the band's ground (not the 1.03:1
 *    it read before), in the same ground as the format chip beside it;
 *  - the ink on it still clears 4.5:1, for the Private word and the Public word;
 *  - hover and an open menu take `--background-strong`, one step deeper than the rest, and move
 *    nothing;
 *  - the instrument: with the badge's rule cut out, the same page reads the old 1.03:1 in light.
 * Set `KNOWLEDGE_TIER_SHOTS` to a directory to also save the screenshots the review reads.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import * as themeTokens from '../../../scripts/lib/theme-tokens.mjs';
import { PageHeader } from '../Layout/PageHeader';
import { Badge } from '../ui/badge';
import { KbTierControl } from './KbTierControl';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const SHOTS = process.env.KNOWLEDGE_TIER_SHOTS;

type Scope = { decls: Record<string, string>; theme: Record<string, string> };
const tokens = themeTokens as unknown as {
  buildScopes(css: string): Record<string, Scope>;
  resolveHex(name: string, scope: Scope): string | null;
  contrast(a: string, b: string): number;
};
const scopes = tokens.buildScopes(MAIN);
const MODES = { light: 'parchment:light', dark: 'parchment:dark' } as const;
type Mode = keyof typeof MODES;

/** Every token the band and the badge read, as the stylesheet declares it in that mode. */
function tokenCss(mode: Mode): string {
  const scope = scopes[MODES[mode]];
  const names = [
    '--sidebar',
    '--background-default',
    '--background-muted',
    '--background-fill',
    '--background-fill-strong',
    '--background-strong',
    '--text-default',
    '--text-muted',
    '--border-subtle',
    '--border-focus',
  ];
  const decls = names
    .map((name) => {
      const value = tokens.resolveHex(name, scope);
      if (!value) throw new Error(`${name} does not resolve to a hex in ${MODES[mode]}`);
      return `${name}: ${value};`;
    })
    .join('\n  ');
  return `:root {
  ${decls}
  --font-body: Arial, Helvetica, sans-serif;
  --chrome-height: 44px;
  --radius-element: 8px; --radius-inner: 4px;
  --dur-fast-min: 0s; --ease-out: ease;
}`;
}

/** The block that opens at `begin` (a selector or an at-rule), through its matching brace. */
function block(begin: string): string {
  const from = MAIN.indexOf(begin);
  if (from < 0) throw new Error(`block not found in main.css: ${begin}`);
  let depth = 0;
  for (let at = MAIN.indexOf('{', from); at < MAIN.length; at++) {
    if (MAIN[at] === '{') depth++;
    if (MAIN[at] === '}' && --depth === 0) return MAIN.slice(from, at + 1);
  }
  throw new Error(`unbalanced block in main.css: ${begin}`);
}

/** A rule block of `main.css`, cut between two markers (inclusive of everything between). */
function cut(begin: string, end: string): string {
  const from = MAIN.indexOf(begin);
  const to = MAIN.indexOf(end, from);
  if (from < 0 || to < from) throw new Error(`markers not found in main.css: ${begin}`);
  return MAIN.slice(from, to + end.length);
}

const TIER_RULE_START = ".br-knowledge-tier [data-testid='privacy-badge'] {";

/** The band, the recessed ground, the neutral badge's layered rule, and the Knowledge band block. */
function ruleCss(options: { withoutBadgeRest?: boolean } = {}): string {
  let knowledge = cut('.br-knowledge-pane {', '/* @ws WS-KNOWLEDGE end: band */');
  if (options.withoutBadgeRest) {
    // The instrument: the badge's resting rule cut out (the hover and open rule after it is a
    // separate selector and stays).
    const rest = block(TIER_RULE_START);
    knowledge = knowledge.replace(rest, '');
    expect(knowledge, 'the rest rule was cut').not.toContain(TIER_RULE_START);
  }
  return [
    cut('.biorouter-page-header {', '/* @ws WS-VIEWS-A end: page band */'),
    block('.br-ground-recessed {'),
    // The neutral badge's layered ground (`.br-badge-neutral`), in the layer the app has it in.
    block('@layer components {\n  .br-field,\n  .br-fill-rest {'),
    knowledge,
  ].join('\n');
}

/** The few utilities the markup needs, in the utilities layer as in the app (they beat components). */
const UTILITIES = `@layer base, components, utilities;
@layer base {
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px var(--font-body); color: var(--text-default); }
h1 { margin: 0; font: inherit; }
button { font: inherit; padding: 0; border: 0 solid; background: none; color: inherit; }
svg { width: 16px; height: 16px; flex-shrink: 0; }
}
@layer utilities {
.flex { display: flex; } .inline-flex { display: inline-flex; }
.items-center { align-items: center; } .justify-center { justify-content: center; }
.min-w-0 { min-width: 0; } .shrink-0 { flex-shrink: 0; } .gap-1 { gap: 4px; } .gap-2 { gap: 8px; }
.px-1\\.5 { padding-inline: 6px; } .h-5 { height: 20px; } .h-3 { height: 12px; } .w-3 { width: 12px; }
.rounded-inner { border-radius: var(--radius-inner); }
.text-chip { font-size: 12px; line-height: 16px; font-weight: 500; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-supporting { font-size: 12px; line-height: 16px; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.text-text-default { color: var(--text-default); } .text-text-muted { color: var(--text-muted); }
.bg-background-muted { background-color: var(--background-muted); }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
}`;

/** The band as `KnowledgeView` draws it, once for a Private base and once for a Public one. */
function documentFor(mode: Mode, css: string): string {
  const band = (id: string, tier: 'private' | 'public') =>
    renderToStaticMarkup(
      <div id={id} style={{ width: 900, marginBottom: 12 }}>
        <PageHeader
          title="Knowledge"
          adornment={
            <>
              <Badge id={`${id}-format`}>OKF</Badge>
              <KbTierControl kb={{ id: 'omop', name: 'OMOP', tier }} onSetTier={() => undefined} />
              <span className="shrink-0">43 pages</span>
            </>
          }
        />
      </div>
    );
  return `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${tokenCss(mode)}\n${UTILITIES}\n${css}</style></head><body>
    ${band('priv', 'private')}
    ${band('pub', 'public')}
  </body></html>`;
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

/** `rgb(r, g, b)` as Chromium reports it, to the `#rrggbb` the token maths takes. */
const hexOf = (rgb: string) => {
  const m = rgb.match(/\d+/g);
  if (!m || m.length < 3) throw new Error(`not an opaque colour: ${rgb}`);
  return `#${m
    .slice(0, 3)
    .map((n) => Number(n).toString(16).padStart(2, '0'))
    .join('')}`;
};

type Look = { bg: string; ink: string; w: number; h: number };
const look = (page: Page, selector: string): Promise<Look> =>
  page.evaluate((sel) => {
    const el = document.querySelector(sel)!;
    const cs = getComputedStyle(el);
    const box = el.getBoundingClientRect();
    return {
      bg: cs.backgroundColor,
      ink: cs.color,
      w: Math.round(box.width * 10) / 10,
      h: Math.round(box.height * 10) / 10,
    };
  }, selector);

describe.each(Object.keys(MODES) as Mode[])('the Knowledge tier badge (%s)', (mode) => {
  let browser: Browser | null = null;
  let page: Page | null = null;
  const scope = scopes[MODES[mode]];
  const hex = (name: string) => tokens.resolveHex(name, scope)!;
  const BADGE = {
    priv: '#priv [data-testid="kb-tier-chip"] [data-testid="privacy-badge"]',
    pub: '#pub [data-testid="kb-tier-chip"] [data-testid="privacy-badge"]',
  };
  const BUTTON = {
    priv: '#priv [data-testid="kb-tier-chip"]',
    pub: '#pub [data-testid="kb-tier-chip"]',
  };

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Knowledge tier badge tests.');
      return;
    }
    page = await browser.newPage({ viewport: { width: 940, height: 200 } });
    await page.setContent(documentFor(mode, ruleCss()));
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  for (const key of ['priv', 'pub'] as const) {
    it(`rests the ${key} badge on the strong fill, a step off the band, beside a chip on the same ground`, async (ctx) => {
      if (!page) return ctx.skip();
      await page.mouse.move(0, 0);
      const band = await look(page, `#${key} [data-testid="page-header"]`);
      const badge = await look(page, BADGE[key]);
      const format = await look(page, `#${key}-format`);

      expect(hexOf(band.bg)).toBe(hex('--sidebar'));
      expect(hexOf(badge.bg), 'rests on the strong fill').toBe(hex('--background-fill-strong'));
      // The R-27 floor for a filled thing on its ground (check-contrast.mjs holds 1.08).
      expect(tokens.contrast(hexOf(badge.bg), hexOf(band.bg))).toBeGreaterThanOrEqual(1.08);
      // One ground in the band: the tier badge and the format chip read as siblings.
      expect(badge.bg).toBe(format.bg);
    });
  }

  for (const [key, ink] of [
    ['priv', '--text-default'],
    ['pub', '--text-muted'],
  ] as const) {
    it(`keeps the ${key} badge’s ink at 4.5:1 or better on that fill`, async (ctx) => {
      if (!page) return ctx.skip();
      await page.mouse.move(0, 0);
      const badge = await look(page, BADGE[key]);
      expect(hexOf(badge.ink)).toBe(hex(ink));
      expect(tokens.contrast(hexOf(badge.ink), hexOf(badge.bg))).toBeGreaterThanOrEqual(4.5);
      // And under the pointer, on the deeper step.
      await page.hover(BUTTON[key]);
      const hover = await look(page, BADGE[key]);
      expect(tokens.contrast(hexOf(hover.ink), hexOf(hover.bg))).toBeGreaterThanOrEqual(4.5);
      await page.mouse.move(0, 0);
    });
  }

  for (const key of ['priv', 'pub'] as const) {
    it(`steps the ${key} badge one fill deeper under the pointer and while its menu is open, moving nothing`, async (ctx) => {
      if (!page) return ctx.skip();
      await page.mouse.move(0, 0);
      const rest = await look(page, BADGE[key]);

      await page.hover(BUTTON[key]);
      const hover = await look(page, BADGE[key]);
      expect(hexOf(hover.bg)).toBe(hex('--background-strong'));
      expect(tokens.contrast(hexOf(hover.bg), hexOf(rest.bg))).toBeGreaterThanOrEqual(1.08);
      expect([hover.w, hover.h]).toEqual([rest.w, rest.h]);
      await page.mouse.move(0, 0);

      // An open menu holds the same step with the pointer away (the trigger carries data-state).
      await page.evaluate(
        (sel) => document.querySelector(sel)!.setAttribute('data-state', 'open'),
        BUTTON[key]
      );
      const open = await look(page, BADGE[key]);
      expect(hexOf(open.bg)).toBe(hex('--background-strong'));
      expect([open.w, open.h]).toEqual([rest.w, rest.h]);
      await page.evaluate(
        (sel) => document.querySelector(sel)!.removeAttribute('data-state'),
        BUTTON[key]
      );
      expect(hexOf((await look(page, BADGE[key])).bg)).toBe(hex('--background-fill-strong'));
    });
  }

  it('instrument: with the badge’s rule cut out, the band reads the old muted fill', async (ctx) => {
    if (!browser) return ctx.skip();
    const old = await browser.newPage({ viewport: { width: 940, height: 200 } });
    try {
      await old.setContent(documentFor(mode, ruleCss({ withoutBadgeRest: true })));
      const badge = await look(old, BADGE.priv);
      const band = await look(old, '#priv [data-testid="page-header"]');
      expect(hexOf(badge.bg)).toBe(hex('--background-muted'));
      expect(hexOf(badge.bg)).not.toBe(hex('--background-fill-strong'));
      if (mode === 'light') {
        // The report's reading: rgb(244,244,242) on rgb(247,247,245), 1.03:1.
        expect(tokens.contrast(hexOf(badge.bg), hexOf(band.bg))).toBeLessThan(1.05);
      }
    } finally {
      await old.close();
    }
  });

  it('saves the screenshots the review reads', async (ctx) => {
    if (!page || !SHOTS) return ctx.skip();
    mkdirSync(SHOTS, { recursive: true });
    await page.mouse.move(0, 0);
    await page.screenshot({ path: join(SHOTS, `tier-badge-${mode}.png`), fullPage: true });
    await page.hover(BUTTON.priv);
    await page.screenshot({ path: join(SHOTS, `tier-badge-${mode}-hover.png`), fullPage: true });
    await page.mouse.move(0, 0);
  });
});
