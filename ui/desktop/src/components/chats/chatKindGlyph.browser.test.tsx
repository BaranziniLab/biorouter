// @vitest-environment node
/**
 * The chat glyphs, painted in a real engine with the real `main.css`.
 *
 * jsdom computes no colour and no box, so `ChatKindIcon.test.tsx` can only
 * assert that the rules exist. This file checks what they do: the lock badge
 * of a private glyph paints in the family accent while the body keeps the
 * row's muted ink (spec 3.3: "body muted, badge --text-accent, on every row
 * including the active one"), and the glyph takes the 16px row slot even when
 * a caller still passes a 14px size class.
 *
 * Tailwind does not run here, so the three utilities the glyph carries are
 * written out below inside `@layer utilities`, where Tailwind puts them. That
 * matters: the authored rules are unlayered and must beat a layered utility,
 * which is the claim being measured.
 *
 * Set `WS_ICONS_SHOT_DIR` to also write a contact sheet of every kind, public
 * and private, in each family and mode.
 */
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

vi.mock('../ConfigContext', () => ({ usePrivacyTiersEnabled: () => true }));

import { ChatKindIcon } from './ChatKindIcon';
import type { ChatKindSource } from './chatKind';

const here = dirname(fileURLToPath(import.meta.url));
// A browser ignores Tailwind's `@theme` at-rule, and the palette scale
// (`--color-coral-600`, which Parchment's `--text-accent` points at) lives in
// one, so it is replayed as plain `:root` declarations the way Tailwind emits it.
const MAIN_CSS = readFileSync(join(here, '../../styles/main.css'), 'utf8').replace(
  /@theme(?:\s+inline)?\s*\{/g,
  ':root {'
);
const UTILITIES = `@layer utilities {
  .text-text-muted { color: var(--text-muted); }
  .text-text-default { color: var(--text-default); }
  .h-3\\.5 { height: 0.875rem; }
  .w-3\\.5 { width: 0.875rem; }
}
body { margin: 0; padding: 16px; background: var(--background-default); color: var(--text-default);
  font: 13px/18px Arial, sans-serif; }
.row { display: flex; align-items: center; gap: 8px; height: 28px; padding: 0 8px; }
.row[data-active] { background: var(--sidebar-active); }
.probe-accent { color: var(--text-accent); }
.probe-muted { color: var(--text-muted); }
.probe-default { color: var(--text-default); }
.sheet { display: grid; grid-template-columns: repeat(2, 220px); gap: 0 16px; }`;

const KINDS: Array<[string, ChatKindSource]> = [
  ['Chat', { name: 'Cohort query' }],
  ['Crew task', { name: 'Crew · #methods · Plot', working_dir: '/x/crew/tasks' }],
  ['Sub-agent', { name: 'Worker', session_type: 'sub_agent' }],
  ['Branch', { name: 'Fork', diverged_from: 's0' }],
  ['App', { name: 'app:spec-002' }],
  ['Scheduled run', { name: 'Nightly', session_type: 'scheduled' }],
  ['Terminal', { name: 'zsh', session_type: 'terminal' }],
];

function sheet(): string {
  const rows = KINDS.flatMap(([label, session], i) =>
    (['public', 'private'] as const).map(
      (tier) =>
        `<div class="row"${i === 0 ? ' data-active' : ''}>${renderToStaticMarkup(
          <ChatKindIcon
            session={session}
            tier={tier}
            testId={`${label}-${tier}`}
            isActive={i === 0}
            // A caller that still passes the old 14px box: the slot must win.
            className="h-3.5 w-3.5"
          />
        )}<span>${label}${tier === 'private' ? ', private' : ''}</span></div>`
    )
  );
  return `<div class="sheet">${rows.join('')}</div>
    <span class="probe-accent">a</span><span class="probe-muted">m</span><span class="probe-default">d</span>`;
}

const html = (family: string, dark: boolean) =>
  `<!doctype html><html data-theme="${family}"${dark ? ' class="dark"' : ''}><head><meta charset="utf-8">` +
  `<style>${MAIN_CSS}</style><style>${UTILITIES}</style></head><body>${sheet()}</body></html>`;

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

interface Paint {
  accent: string;
  muted: string;
  ink: string;
  glyphs: Array<{
    id: string;
    width: number;
    height: number;
    body: string;
    badge: string | null;
  }>;
}

const paint = (page: Page): Promise<Paint> =>
  page.evaluate(() => {
    const color = (sel: string) => getComputedStyle(document.querySelector(sel)!).color;
    return {
      accent: color('.probe-accent'),
      muted: color('.probe-muted'),
      ink: color('.probe-default'),
      glyphs: [...document.querySelectorAll<SVGSVGElement>('svg.br-chat-kind-icon')].map((svg) => {
        const rect = svg.getBoundingClientRect();
        const badge = svg.querySelector('.br-icon-lock-badge rect');
        // The body's first drawn element: the nested base glyph when badged.
        const body = svg.querySelector(
          'g[mask] svg > :not(defs), :scope > path, :scope > rect, :scope > circle, :scope > line, :scope > polyline'
        );
        return {
          id: svg.getAttribute('data-testid') ?? '',
          width: rect.width,
          height: rect.height,
          body: body ? getComputedStyle(body).stroke : '',
          badge: badge ? getComputedStyle(badge).stroke : null,
        };
      }),
    };
  });

describe('chat glyphs in a real engine', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found: skipping the chat glyph paint tests. ' +
          'Run `npx playwright install chromium` to enable them.'
      );
    }
  }, 60_000);

  afterAll(async () => {
    await browser?.close();
  });

  for (const family of ['parchment', 'alma-mater', 'roche-limit']) {
    for (const dark of [false, true]) {
      it(`${family} ${dark ? 'dark' : 'light'}: badge in the accent, body in the row ink, 16px`, async () => {
        if (!browser) return;
        const dir = process.env.WS_ICONS_SHOT_DIR;
        const page = await browser.newPage({
          viewport: { width: 520, height: 280 },
          deviceScaleFactor: dir ? 2 : 1,
        });
        try {
          await page.setContent(html(family, dark), { waitUntil: 'load' });
          const result = await paint(page);
          expect(result.glyphs).toHaveLength(KINDS.length * 2);
          // The accent must differ from the muted ink, or the check proves nothing.
          expect(result.accent).not.toBe(result.muted);
          for (const glyph of result.glyphs) {
            expect([glyph.width, glyph.height], glyph.id).toEqual([16, 16]);
            const active = glyph.id.startsWith('Chat-');
            expect(glyph.body, glyph.id).toBe(active ? result.ink : result.muted);
            if (glyph.id.endsWith('-private')) {
              expect(glyph.badge, glyph.id).toBe(result.accent);
            } else {
              expect(glyph.badge, glyph.id).toBeNull();
            }
          }
          if (dir) {
            await page.screenshot({
              path: join(dir, `chat-kinds-${family}-${dark ? 'dark' : 'light'}.png`),
            });
          }
        } finally {
          await page.close();
        }
      }, 30_000);
    }
  }
});
