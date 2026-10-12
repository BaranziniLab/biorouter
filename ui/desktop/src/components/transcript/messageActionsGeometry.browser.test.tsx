// @vitest-environment node
/**
 * A message's floating actions, measured in a real layout engine (spec 3.6,
 * 5.7; acceptance "Hovering a message shows the floating cluster with no layout
 * shift").
 *
 * jsdom lays nothing out and evaluates no `:hover`, so a component test cannot
 * tell a cluster that floats from one that reserves a row. The real
 * `UserMessage` is rendered to markup and laid out in Chromium with the
 * authored stylesheet (`message-meta.css`) inside the 760px chat column. It pins
 * what the redesign is for: no row is reserved for the actions (the old meta row
 * added 28px under every message), the cluster rises 26px above the message's
 * top right edge and ends 2px into it, so it covers the words of neither the
 * message it acts on nor the block a turn's gap above (vision V2-12, twice), it
 * is invisible and click-through at rest, and a hover reveals it without moving
 * anything by a pixel. The transcript cases lay out the list's own rows (their
 * `data-gap` and margins, `transcriptRhythm.ts`) and measure the cluster
 * against the glyph boxes of the text around it, which is what a reader sees.
 *
 * Tailwind does not run here, so the 24px buttons are stood in for by one rule
 * (`TAILWIND_STANDINS`); everything the test measures about the cluster is
 * authored CSS.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import UserMessage from '../UserMessage';
import { MessageActions } from '../MessageMeta';
import { TranscriptRow } from '../TranscriptRow';
import type { Message } from '../../api';

const here = dirname(fileURLToPath(import.meta.url));
const COLUMN = 760;

/** `main.css`'s values for the tokens the cluster reads. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --text-secondary: 13px;
  --text-secondary--line-height: 18px;
  --radius-element: 8px;
  --background-default: #ffffff;
  --shadow-popover: 0 4px 16px rgb(0 0 0 / 0.12);
  --inset-hairline: inset 0 0 0 1px rgb(0 0 0 / 0.08);
  --dur-fast-min: 95ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px var(--font-body); }
* { box-sizing: border-box; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
  overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }`;

/** The few Tailwind utilities the measured boxes depend on. */
const TAILWIND_STANDINS = `
.flex { display: flex; } .flex-col { flex-direction: column; } .justify-end { justify-content: flex-end; }
.w-full { width: 100%; } .w-fit { width: fit-content; } .max-w-\\[85\\%\\] { max-width: 85%; }
.min-w-0 { min-width: 0; } .self-end { align-self: flex-end; }
[data-testid='user-message-bubble'] { padding: 10px 14px; }
.br-message-actions button { width: 24px; height: 24px; padding: 0; border: 0; background: none; }
.br-message-actions svg { width: 14px; height: 14px; }
.relative { position: relative; } .items-center { align-items: center; } .gap-2 { gap: 8px; }
.mb-1 { margin-bottom: 4px; } .max-w-full { max-width: 100%; }
.mt-6 { margin-top: 24px; } .mt-2 { margin-top: 8px; } .mt-0 { margin-top: 0; } .pt-7 { padding-top: 28px; }
.gap-1 { gap: 4px; }
.text-supporting { font-size: 12px; line-height: 16px; } .tabular-nums { font-variant-numeric: tabular-nums; }
[data-provenance-kind] { display: inline-flex; align-items: center; height: 20px; padding: 0 6px; }
p { margin: 0; }`;

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

const message: Message = {
  id: 'm1',
  role: 'user',
  created: 1_700_000_000,
  content: [{ type: 'text', text: 'Which genes moved?' }],
  metadata: { userVisible: true, agentVisible: true },
};

interface Box {
  top: number;
  left: number;
  right: number;
  bottom: number;
  height: number;
}

interface Measure {
  row: Box;
  anchor: Box;
  bubble: Box;
  cluster: Box;
  text: Box;
  opacity: number;
  pointerEvents: string;
}

describe('message actions in a real layout engine', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the message actions ' +
          'geometry tests. Run `npx playwright install chromium` to enable them.'
      );
    }
    css = readFileSync(resolve(here, '../message-meta.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const html = renderToStaticMarkup(<UserMessage message={message} />);
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 400 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${TOKENS}${TAILWIND_STANDINS}${css}</style></head>` +
        `<body><div id="column" style="width:${COLUMN}px;margin:80px 0 0 100px">` +
        `<div id="row">${html}</div></div></body></html>`
    );
    return tab;
  }

  const measure = (tab: Page): Promise<Measure> =>
    tab.evaluate(() => {
      const box = (selector: string) => {
        const r = document.querySelector(selector)!.getBoundingClientRect();
        return { top: r.top, left: r.left, right: r.right, bottom: r.bottom, height: r.height };
      };
      const cluster = document.querySelector('.br-message-actions')!;
      const style = getComputedStyle(cluster);
      return {
        row: box('#row'),
        anchor: box('.br-message-actions-anchor'),
        bubble: box("[data-testid='user-message-bubble']"),
        cluster: box('.br-message-actions'),
        text: box("[data-testid='user-message-bubble'] > *"),
        opacity: Number(style.opacity),
        pointerEvents: style.pointerEvents,
      };
    });

  it('reserves no row: the message is exactly as tall as its bubble', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.row.height).toBe(m.bubble.height);
      expect(m.anchor.height).toBe(m.bubble.height);
    } finally {
      await tab.close();
    }
  });

  it("floats 26px above the bubble, on the bubble's right edge, hidden and click-through", async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const m = await measure(tab);
      expect(m.cluster.top).toBe(m.bubble.top - 26);
      // It ends 2px into the bubble, inside its 10px top padding: never on the
      // first line (vision V2-12).
      expect(m.cluster.bottom).toBe(m.bubble.top + 2);
      expect(m.text.top - m.cluster.bottom).toBeGreaterThanOrEqual(2);
      expect(Math.abs(m.cluster.right - m.bubble.right)).toBeLessThanOrEqual(0.5);
      // Two 24px buttons and the time, with 2px of padding: one compact strip.
      expect(m.cluster.height).toBe(28);
      expect(m.opacity).toBe(0);
      expect(m.pointerEvents).toBe('none');
    } finally {
      await tab.close();
    }
  });

  it('shows on hover without moving the message by a pixel', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      const before = await measure(tab);
      await tab.hover("[data-testid='user-message-bubble']");
      await tab.waitForTimeout(250);
      const after = await measure(tab);
      expect(after.opacity).toBe(1);
      expect(after.pointerEvents).toBe('auto');
      expect(after.bubble).toEqual(before.bubble);
      expect(after.row).toEqual(before.row);
      expect(after.cluster).toEqual(before.cluster);
    } finally {
      await tab.close();
    }
  });

  it('shows on keyboard focus inside the message, so Tab reaches every action', async () => {
    if (!browser) return;
    const tab = await open();
    try {
      await tab.focus('.br-message-actions button');
      await tab.waitForTimeout(250);
      const m = await measure(tab);
      expect(m.opacity).toBe(1);
    } finally {
      await tab.close();
    }
  });
});

/**
 * The cluster among its neighbours: the rows exactly as `ProgressiveMessageList`
 * draws them (a wrapper per message carrying `data-role`, `data-gap` and the
 * gap's margin), measured against the GLYPH boxes of the text around it. A
 * Range's client rects are a line's inline content area, the ascent to descent
 * box the glyphs paint in, without the half-leading, so "no overlap" here means
 * no ink under the cluster. Round 2 of the vision loop measured the old 8px rise
 * covering the right end of an answer's first line (cluster 724..900 x
 * 215..243 over a line at 223..244).
 */
describe('message actions between the rows of a transcript', () => {
  let browser: Browser | null = null;
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    // WS-TOOLS' row stylesheet, read as the app reads it: the tool rows above
    // an answer are measured as they really lay out.
    css =
      readFileSync(resolve(here, '../transcript-row.css'), 'utf8') +
      readFileSync(resolve(here, '../message-meta.css'), 'utf8');
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  const FULL_LINE =
    'PCR, invented by Kary Mullis and enabled in practice by thermostable polymerases ' +
    'like Taq, is a cyclic amplification method whose major variants transformed ' +
    'molecular biology and medicine by making rapid, sensitive DNA detection routine.';

  const user = (id: string, text: string, metadata: Partial<Message['metadata']> = {}) =>
    renderToStaticMarkup(
      <UserMessage
        message={{
          ...message,
          id,
          content: [{ type: 'text', text }],
          metadata: { userVisible: true, agentVisible: true, ...metadata },
        }}
      />
    );

  /** BioRouterMessage's answer: the scope, the cluster first, then the words. */
  const answer = (text: string, timestamp = '06/10/2026 1:28 PM') =>
    `<div class="br-message-actions-scope flex flex-col" data-answer>` +
    renderToStaticMarkup(
      <MessageActions timestamp={timestamp}>
        <button type="button" aria-label="Copy" />
        <button type="button" aria-label="More" />
      </MessageActions>
    ) +
    `<p data-answer-text>${text}</p></div>`;

  const row = (role: 'user' | 'assistant', gap: string, cls: string, html: string, attrs = '') =>
    `<div class="relative ${cls}" data-role="${role}" data-gap="${gap}"${attrs}>${html}</div>`;

  async function lay(rows: string): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 600 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${TOKENS}${TAILWIND_STANDINS}${css}</style></head>` +
        `<body><div style="width:${COLUMN}px;margin:40px 0 0 100px">${rows}</div></body></html>`
    );
    return tab;
  }

  type Rect = { top: number; bottom: number; left: number; right: number };

  /** The cluster in the n-th row (its box), and every glyph line box in the page. */
  const geometry = (tab: Page, rowIndex: number) =>
    tab.evaluate((index) => {
      const rect = (r: DOMRect) => ({ top: r.top, bottom: r.bottom, left: r.left, right: r.right });
      const rows = Array.from(document.querySelectorAll<HTMLElement>('[data-gap]'));
      const cluster = rows[index].querySelector('.br-message-actions')!;
      const ink: Array<{ row: number; box: ReturnType<typeof rect> }> = [];
      rows.forEach((r, i) => {
        r.querySelectorAll(
          "[data-answer-text], [data-testid='user-message-bubble'] > *, [data-provenance-kind]"
        ).forEach((el) => {
          const range = document.createRange();
          range.selectNodeContents(el);
          for (const box of Array.from(range.getClientRects())) {
            if (box.width > 0) ink.push({ row: i, box: rect(box) });
          }
        });
      });
      const chip = rows[index].querySelector('[data-provenance-kind]');
      const bubble = rows[index].querySelector("[data-testid='user-message-bubble']");
      return {
        cluster: rect(cluster.getBoundingClientRect()),
        chip: chip ? rect(chip.getBoundingClientRect()) : null,
        bubble: bubble ? rect(bubble.getBoundingClientRect()) : null,
        ink,
      };
    }, rowIndex);

  const overlaps = (a: Rect, b: Rect) =>
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

  it("over an answer under a question, it covers no ink of either (V2-12's case)", async () => {
    if (!browser) return;
    const tab = await lay(
      row('user', 'first', 'mt-0 pt-7', user('u1', 'Then summarize it in one sentence.')) +
        row('assistant', 'turn', 'mt-6', answer(FULL_LINE))
    );
    try {
      const g = await geometry(tab, 1);
      // The answer's first line runs to the column's right edge, under where
      // the cluster floats.
      const firstLine = g.ink.find((i) => i.row === 1)!.box;
      expect(firstLine.right).toBeGreaterThan(g.cluster.left);
      for (const { box } of g.ink) expect(overlaps(g.cluster, box)).toBe(false);
      expect(g.cluster.bottom).toBeLessThanOrEqual(firstLine.top);
    } finally {
      await tab.close();
    }
  });

  it('over a question under an answer, it covers no ink of either', async () => {
    if (!browser) return;
    const tab = await lay(
      row('assistant', 'first', 'mt-0 pt-7', answer(FULL_LINE)) +
        row('user', 'turn', 'mt-6', user('u1', 'Shorter, please.'))
    );
    try {
      const g = await geometry(tab, 1);
      for (const { box } of g.ink) expect(overlaps(g.cluster, box)).toBe(false);
      expect(g.cluster.bottom).toBe(g.bubble!.top + 2);
    } finally {
      await tab.close();
    }
  });

  it('over a question 8px under another, it rises 18px and touches only paddings', async () => {
    if (!browser) return;
    const tab = await lay(
      row('user', 'first', 'mt-0 pt-7', user('u1', 'please write me a poem about tides')) +
        row('user', 'block', 'mt-2', user('u2', 'Run a shell command that prints a marker'))
    );
    try {
      const g = await geometry(tab, 1);
      expect(g.cluster.top).toBe(g.bubble!.top - 18);
      for (const { box } of g.ink) expect(overlaps(g.cluster, box)).toBe(false);
    } finally {
      await tab.close();
    }
  });

  it("beside a provenance chip, it sits on the chip's line to its left, over nothing", async () => {
    if (!browser) return;
    const tab = await lay(
      row(
        'user',
        'first',
        'mt-0 pt-7',
        user('u1', 'Count the files in the folder', {
          provenance: { kind: 'spawn_context', fromSessionId: 's-parent' },
        })
      )
    );
    try {
      const g = await geometry(tab, 0);
      expect(g.chip).not.toBeNull();
      expect(Math.abs(g.chip!.left - 8 - g.cluster.right)).toBeLessThanOrEqual(0.5);
      const chipMiddle = (g.chip!.top + g.chip!.bottom) / 2;
      const clusterMiddle = (g.cluster.top + g.cluster.bottom) / 2;
      expect(Math.abs(chipMiddle - clusterMiddle)).toBeLessThanOrEqual(0.5);
      expect(overlaps(g.cluster, g.chip!)).toBe(false);
      for (const { box } of g.ink) expect(overlaps(g.cluster, box)).toBe(false);
      // Nothing above the first row's 28px is cut off by the list's top edge.
      expect(g.cluster.top).toBeGreaterThanOrEqual(40);
    } finally {
      await tab.close();
    }
  });

  it('the first row leaves room for the rise: nothing pokes above the list', async () => {
    if (!browser) return;
    const tab = await lay(row('user', 'first', 'mt-0 pt-7', user('u1', 'Which genes moved?')));
    try {
      const g = await geometry(tab, 0);
      // The list starts at y=40 in this page.
      expect(g.cluster.top).toBeGreaterThanOrEqual(40);
    } finally {
      await tab.close();
    }
  });

  /*
   * Vision V2-12, round 3: inside a turn an answer sits 8px under a tool row,
   * and the cluster's 26px rise reached 18px into it. A shell command's label
   * runs the full column, so the cluster hid its end and its chevron (measured
   * on chat 20260904_1: row 336..1104 x 375..403, cluster 928..1104 x 385..413).
   * The list marks the row above such an answer, and its last tool row ends the
   * cluster's width short.
   */
  const LONG_COMMAND =
    'Ran cd /Users/wgu/Desktop/kdps-build && R CMD INSTALL kdps_1.0.0.tar.gz 2>&1 | tail -8 ' +
    '&& Rscript -e \'library(kdps); packageVersion("kdps")\' && echo installed';

  const Glyph = () => <svg viewBox="0 0 16 16" />;

  /** BioRouterMessage's tool block: rows 4px apart, each a closed TranscriptRow. */
  const tools = (...labels: string[]) =>
    `<div class="flex w-full flex-col gap-1">` +
    labels
      .map(
        (label) =>
          `<div class="biorouter-message-tool">` +
          renderToStaticMarkup(
            <TranscriptRow icon={Glyph} glyph="shell" label={label}>
              <p>output</p>
            </TranscriptRow>
          ) +
          `</div>`
      )
      .join('') +
    `</div>`;

  /** The answer's cluster, and every tool row's label glyphs, head and chevron. */
  const toolGeometry = (tab: Page) =>
    tab.evaluate(() => {
      const rect = (r: DOMRect) => ({ top: r.top, bottom: r.bottom, left: r.left, right: r.right });
      const lines = Array.from(document.querySelectorAll<HTMLElement>('.br-transcript-row-line'));
      return {
        cluster: rect(
          document.querySelector('[data-answer] .br-message-actions')!.getBoundingClientRect()
        ),
        rows: lines.map((line) => {
          const label = line.querySelector<HTMLElement>('.br-transcript-row-label')!;
          const range = document.createRange();
          range.selectNodeContents(label);
          return {
            head: rect(line.querySelector('.br-transcript-row-head')!.getBoundingClientRect()),
            chevron: rect(
              line.querySelector('.br-transcript-row-chevron')!.getBoundingClientRect()
            ),
            // The label's glyphs as painted: the visible part of its text box.
            ink: rect(range.getBoundingClientRect()),
            labelBox: rect(label.getBoundingClientRect()),
            truncated: label.scrollWidth > label.clientWidth,
          };
        }),
      };
    });

  const underToolRows = (labels: string[], marked: boolean) =>
    lay(
      row('user', 'first', 'mt-0 pt-7', user('u1', 'Install the package')) +
        row('assistant', 'turn', 'mt-6', tools(...labels), marked ? ' data-actions-below' : '') +
        // The widest time `formatMessageTimestamp` writes: the widest cluster.
        row('assistant', 'block', 'mt-2', answer(FULL_LINE, '12/31/2026 12:59 PM'))
    );

  it("under a full-width tool row, it covers neither the label's end nor the chevron", async () => {
    if (!browser) return;
    const tab = await underToolRows([LONG_COMMAND, LONG_COMMAND], true);
    try {
      const g = await toolGeometry(tab);
      const [upper, last] = g.rows;
      // The cluster still reaches into the last row's band (it rises 26px over
      // an 8px gap): what keeps it off the row is the row ending short.
      expect(g.cluster.top).toBeLessThan(last.head.bottom);
      expect(last.truncated).toBe(true);
      const visibleLabel = {
        ...last.labelBox,
        top: last.ink.top,
        bottom: last.ink.bottom,
      };
      expect(overlaps(g.cluster, visibleLabel)).toBe(false);
      expect(overlaps(g.cluster, last.chevron)).toBe(false);
      expect(overlaps(g.cluster, last.head)).toBe(false);
      // With the widest time the cluster is 183px; 8px of air stays between
      // it and the row's head (its hover wash), 16px and more to the chevron.
      expect(g.cluster.right - g.cluster.left).toBeGreaterThan(180);
      expect(g.cluster.left - last.head.right).toBeGreaterThanOrEqual(8);
      expect(g.cluster.left - last.chevron.right).toBeGreaterThanOrEqual(16);
      // Only the row the cluster reaches ends short; the row above keeps the
      // full column, out to the cluster's edge and past it.
      expect(upper.truncated).toBe(true);
      expect(upper.chevron.right).toBeGreaterThan(g.cluster.left);
      expect(last.head.left).toBe(upper.head.left);
    } finally {
      await tab.close();
    }
  });

  it('without the mark the cluster lands on the label and the chevron (the case it fixes)', async () => {
    if (!browser) return;
    const tab = await underToolRows([LONG_COMMAND], false);
    try {
      const g = await toolGeometry(tab);
      const [last] = g.rows;
      expect(overlaps(g.cluster, last.chevron)).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('a short label is not moved by the mark', async () => {
    if (!browser) return;
    const short = 'Ran shell status';
    const marked = await underToolRows([short], true);
    const plain = await underToolRows([short], false);
    try {
      const a = await toolGeometry(marked);
      const b = await toolGeometry(plain);
      expect(a.rows[0].head).toEqual(b.rows[0].head);
      expect(a.rows[0].truncated).toBe(false);
      expect(overlaps(a.cluster, a.rows[0].head)).toBe(false);
    } finally {
      await marked.close();
      await plain.close();
    }
  });

  /*
   * WS-TOOLS-3: the last entry is an OPEN Explored group, `ExploredRow`'s DOM
   * (`ToolCallRows.tsx`): a row whose plain body holds one closed row per call.
   * The rule's selector reaches every line in the entry, so the nested rows
   * all end 192px short, not only the bottom one. Kept on purpose: the rows of
   * one list then share one right edge, whereas freeing the upper ones would
   * leave the row at the bottom clipped 192px earlier than its siblings at
   * rest, which reads as a glitch (a child chain cannot pick out the bottom
   * line: a call's row sits under varying wrappers, and an open well below it
   * would defeat a `:last-child` chain). Pinned here so a change to the
   * selector is a choice, not an accident.
   */
  const READ_PATH = (name: string) =>
    `Read /Users/wgu/Desktop/BioRouter/.claude/worktrees/crew-landing-redesign-91b46e/ui/desktop/src/components/${name}`;

  const exploredGroup = (labels: string[]) =>
    `<div class="flex w-full flex-col gap-1"><div class="biorouter-message-tool">` +
    renderToStaticMarkup(
      <TranscriptRow
        icon={Glyph}
        glyph="explored"
        label="Explored"
        meta="· 3 files"
        body="plain"
        defaultOpen
      >
        <div className="flex min-w-0 flex-col gap-1">
          {labels.map((label) => (
            <div key={label} className="min-w-0">
              <TranscriptRow icon={Glyph} glyph="read" label={label}>
                <p>output</p>
              </TranscriptRow>
            </div>
          ))}
        </div>
      </TranscriptRow>
    ) +
    `</div></div>`;

  const underExploredGroup = (labels: string[], marked: boolean) =>
    lay(
      row('user', 'first', 'mt-0 pt-7', user('u1', 'Look around')) +
        row(
          'assistant',
          'turn',
          'mt-6',
          exploredGroup(labels),
          marked ? ' data-actions-below' : ''
        ) +
        row('assistant', 'block', 'mt-2', answer(FULL_LINE, '12/31/2026 12:59 PM'))
    );

  it('an open Explored group: the nested rows end short together and the last clears the cluster', async () => {
    if (!browser) return;
    const labels = ['A.tsx', 'B.tsx', 'C.tsx'].map(READ_PATH);
    const marked = await underExploredGroup(labels, true);
    const plain = await underExploredGroup(labels, false);
    try {
      const g = await toolGeometry(marked);
      const free = await toolGeometry(plain);
      // The group's own line, then the three calls under it.
      expect(g.rows).toHaveLength(4);
      const nested = g.rows.slice(1);
      const last = nested[nested.length - 1];
      // The instrument: the cluster really reaches the last nested row's band,
      // and without the mark it lands on that row's chevron.
      expect(g.cluster.top).toBeLessThan(last.head.bottom);
      expect(overlaps(free.cluster, free.rows[3].chevron)).toBe(true);
      // With it, the last call's label end, chevron and head are clear.
      expect(last.truncated).toBe(true);
      expect(
        overlaps(g.cluster, { ...last.labelBox, top: last.ink.top, bottom: last.ink.bottom })
      ).toBe(false);
      expect(overlaps(g.cluster, last.chevron)).toBe(false);
      expect(overlaps(g.cluster, last.head)).toBe(false);
      expect(g.cluster.left - last.head.right).toBeGreaterThanOrEqual(8);
      // The decision: every nested row shares that right edge.
      for (const call of nested) {
        expect(call.truncated).toBe(true);
        expect(call.head.right).toBeCloseTo(last.head.right, 1);
      }
      // And the unmarked group runs its calls the whole column.
      for (const call of free.rows.slice(1)) {
        expect(call.head.right).toBeGreaterThan(last.head.right + 150);
      }
    } finally {
      await marked.close();
      await plain.close();
    }
  });
});
