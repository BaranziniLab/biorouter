// @vitest-environment node
/**
 * A Waiting to join row, measured in a real layout engine (Q3-53).
 *
 * jsdom lays nothing out, so the component tests can only say which cell holds what. The defect
 * this file exists for was geometry: an expired row put "Invitation expired · Invite again…"
 * (about 204px) beside the username in a 208px row, and a `minmax(0, 1fr) auto` grid left
 * `@crew_gina` a column 0px wide, one letter per line. Every assertion in the suite passed.
 *
 * The real `AttentionSections` is rendered to markup (react-dom/server) and laid out in Chromium
 * with the real `crew-app.css` and `sidebar/crew-sidebar.css`, at the app's font stacks and text
 * sizes, inside the 240px Crew column. Tailwind does not run here, so the utilities the row uses
 * are written out below at `main.css`'s values — and a class the table does not know fails the
 * suite, so a new layout utility cannot slip in unmeasured.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { Snapshot } from '../crewApi';
import { CrewControllerProvider } from '../state/CrewControllerContext';
import { AttentionSections } from './AttentionSections';
import { makeController, makeSnapshot } from './sidebarTestUtils';

const crewDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const STYLESHEETS = ['crew-app.css', 'sidebar/crew-sidebar.css'];

const CREW_COLUMN = 240;
/** The column, less the list's 8px inset and the row's own `px-2` on each side. */
const ROW = CREW_COLUMN - 2 * 8 - 2 * 8;

/** `main.css`'s font stacks and the text roles the row uses. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, BlinkMacSystemFont,
    'Segoe UI', Roboto, sans-serif;
  --font-mono: ui-monospace, 'SF Mono', SFMono-Regular, 'Cascadia Mono', Menlo, Consolas,
    'Liberation Mono', monospace;
  --row-height: 32px;
  --radius-element: 8px;
  --sidebar-border: rgb(200, 200, 200);
  --dur-fast-min: 80ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px var(--font-body); }
* { box-sizing: border-box; }
button { margin: 0; border: 0; font: inherit; background: none; }`;

/**
 * The Tailwind utilities that move a box, at `main.css`'s values (spacing 4px; text-label 14/20
 * 500; text-secondary 13/18; text-supporting 12/16; text-caps 11/16 500 0.08em;
 * control-compact 24px).
 */
const UTILITIES: Record<string, string> = {
  flex: 'display: flex',
  'inline-flex': 'display: inline-flex',
  'flex-col': 'flex-direction: column',
  'items-center': 'align-items: center',
  'items-start': 'align-items: flex-start',
  'justify-center': 'justify-content: center',
  'gap-1': 'gap: 4px',
  'gap-1.5': 'gap: 6px',
  'px-2': 'padding-inline: 8px',
  'py-1.5': 'padding-block: 6px',
  'mt-0.5': 'margin-top: 2px',
  'size-3': 'width: 12px; height: 12px',
  'shrink-0': 'flex-shrink: 0',
  'whitespace-nowrap': 'white-space: nowrap',
  'h-control-compact': 'height: 24px',
  'font-mono': 'font-family: var(--font-mono)',
  'text-label': 'font-size: 14px; line-height: 20px; font-weight: 500',
  'text-secondary': 'font-size: 13px; line-height: 18px; font-weight: 400',
  'text-supporting': 'font-size: 12px; line-height: 16px; font-weight: 400',
  'text-caps':
    'font-size: 11px; line-height: 16px; font-weight: 500; letter-spacing: 0.08em; text-transform: uppercase',
  'sr-only':
    'position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0',
};

/** Classes that only paint, or only act on something this row does not contain. */
const PAINT_ONLY = [
  /^crew-sidebar/,
  /^(bg|text-text|rounded)-/,
  /^(transition|cursor|tint|biorouter-focus|no-drag|lucide)/,
  /^(active|disabled|hover|focus-visible):/,
  /^\[&/,
];

const escapeClass = (name: string) => name.replace(/[.:[\]&]/g, (c) => `\\${c}`);
const UTILITY_CSS = Object.entries(UTILITIES)
  .map(([name, rule]) => `.${escapeClass(name)} { ${rule}; }`)
  .join('\n');

type Join = NonNullable<Snapshot['pending_joins']>[number];

function markup(joins: Join[]): string {
  const controller = makeController({ snapshot: makeSnapshot({ pending_joins: joins }) });
  return renderToStaticMarkup(
    <CrewControllerProvider controller={controller}>
      <AttentionSections />
    </CrewControllerProvider>
  );
}

/** Every class the markup carries that neither the table nor the paint-only list accounts for. */
function unmeasuredClasses(html: string): string[] {
  const names = new Set<string>();
  for (const match of html.matchAll(/class="([^"]*)"/g)) {
    for (const name of match[1]
      .replace(/&amp;/g, '&')
      .replace(/&#x27;/g, "'")
      .split(/\s+/)) {
      if (name) names.add(name);
    }
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

interface Box {
  top: number;
  bottom: number;
  left: number;
  right: number;
  width: number;
  height: number;
}

interface RowGeometry {
  head: Box;
  handle: Box;
  /** How many lines the username's text takes: one rect per line of an inline box. */
  handleLines: number;
  name: Box;
  state: string | null;
  action: Box | null;
  /** The button's text is wider than its box: it would be cut. */
  actionCut: boolean;
}

const EPSILON = 0.5;

describe('a Waiting to join row in a real layout engine (Q3-53)', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome) — skipping the Waiting to join row ' +
          'geometry tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = STYLESHEETS.map((path) => readFileSync(resolve(crewDir, path), 'utf8')).join('\n');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function layOut(joins: Join[]): Promise<Record<string, RowGeometry>> {
    const html = markup(joins);
    expect(unmeasuredClasses(html), 'a class with no measured rule').toEqual([]);
    const tab: Page = await browser!.newPage({ viewport: { width: 800, height: 900 } });
    try {
      await tab.setContent(
        '<!doctype html><html><head><style>' +
          TOKENS +
          UTILITY_CSS +
          css +
          `</style></head><body><div class="crew-sidebar" style="width: ${CREW_COLUMN}px">` +
          html +
          '</div></body></html>'
      );
      return await tab.evaluate(() => {
        const box = (el: Element): Box => {
          const r = el.getBoundingClientRect();
          return {
            top: r.top,
            bottom: r.bottom,
            left: r.left,
            right: r.right,
            width: r.width,
            height: r.height,
          };
        };
        const out: Record<string, RowGeometry> = {};
        for (const row of Array.from(document.querySelectorAll('[data-crew-waiting]'))) {
          const handle = row.querySelector('[data-crew-waiting-handle]')!;
          const username = handle.querySelector('[data-person-part="username"]')!;
          const action = row.querySelector('.crew-sidebar-waiting-action');
          const button = action?.querySelector('button') ?? null;
          out[(row as HTMLElement).dataset.crewWaiting!] = {
            head: box(row.querySelector('.crew-sidebar-waiting-head')!),
            handle: box(handle),
            handleLines: username.getClientRects().length,
            name: box(row.querySelector('[data-crew-waiting-name]')!),
            state:
              row.querySelector<HTMLElement>('[data-crew-waiting-state]')?.dataset
                .crewWaitingState ?? null,
            action: action ? box(action) : null,
            actionCut: button ? button.scrollWidth > button.clientWidth : false,
          };
        }
        return out;
      });
    } finally {
      await tab.close();
    }
  }

  /** The username is whole on one line, and the button — if any — sits clear of it, uncut. */
  function expectUnsqueezed(row: RowGeometry, what: string) {
    expect(row.head.width, `${what}: row width`).toBeCloseTo(ROW, 0);
    expect(row.handleLines, `${what}: lines the username takes`).toBe(1);
    // The name and state have the whole row to themselves, under the username.
    expect(row.name.width, `${what}: name line width`).toBeCloseTo(ROW, 0);
    expect(row.name.top, `${what}: name line under the username`).toBeGreaterThanOrEqual(
      row.handle.bottom - EPSILON
    );
    if (!row.action) return;
    expect(row.actionCut, `${what}: button text cut`).toBe(false);
    expect(row.action.right, `${what}: button inside the row`).toBeLessThanOrEqual(
      row.head.right + EPSILON
    );
    const beside = row.action.left >= row.handle.right - EPSILON;
    const below = row.action.top >= row.handle.bottom - EPSILON;
    expect(beside || below, `${what}: button overlaps the username`).toBe(true);
  }

  it('keeps @crew_gina whole in every state, with only a button beside it', async (ctx) => {
    if (!browser) return ctx.skip();
    const rows = await layOut([
      { username: 'crew_gina', full_name: 'Gina Rossi', expired: true },
      {
        username: 'crew_gina2',
        full_name: 'Gina Rossi',
        approved: true,
        mismatched_attempts: 1,
      },
      { username: 'crew_gina3', full_name: 'Gina Rossi', approved: true },
      { username: 'crew_gina4', full_name: 'Gina Rossi' },
    ]);
    expect(rows.crew_gina.state).toBe('expired');
    expect(rows.crew_gina2.state).toBe('code-entered');
    expect(rows.crew_gina3.state).toBe('code-entered');
    expect(rows.crew_gina4.state).toBe('invited');
    for (const [username, row] of Object.entries(rows)) expectUnsqueezed(row, username);
    // A short username keeps its button beside it, on the first line.
    for (const username of ['crew_gina', 'crew_gina2', 'crew_gina4']) {
      const row = rows[username];
      expect(row.action, `${username}: has a button`).not.toBeNull();
      expect(row.action!.top, `${username}: button on the first line`).toBeLessThan(
        row.handle.bottom
      );
    }
    // A code entered with nothing to fix has no button at all.
    expect(rows.crew_gina3.action).toBeNull();
  });

  it('drops the button below a username too wide to share the line, never squeezing it', async (ctx) => {
    if (!browser) return ctx.skip();
    const rows = await layOut([
      // Fits the row alone, but not beside Invite again… or Let in….
      { username: 'crew_gina_rossi_lab', full_name: 'Gina Rossi', expired: true },
      { username: 'crew_gina_rossi_la2', full_name: 'Gina Rossi' },
    ]);
    for (const [username, row] of Object.entries(rows)) {
      expectUnsqueezed(row, username);
      expect(row.action!.top, `${username}: button below`).toBeGreaterThanOrEqual(
        row.handle.bottom - EPSILON
      );
    }
  });

  it('gives a username wider than the row the whole row, and wraps only then', async (ctx) => {
    if (!browser) return ctx.skip();
    const username = 'crew_gina_rossi_structural_biology_core';
    const rows = await layOut([{ username, full_name: 'Gina Rossi', expired: true }]);
    const row = rows[username];
    expect(row.handle.width).toBeCloseTo(ROW, 0);
    expect(row.handleLines).toBe(2);
    expect(row.actionCut).toBe(false);
    expect(row.action!.top).toBeGreaterThanOrEqual(row.handle.bottom - EPSILON);
  });
});
