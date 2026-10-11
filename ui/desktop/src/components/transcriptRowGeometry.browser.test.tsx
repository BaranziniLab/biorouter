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
import { summarizeToolCall } from './ToolCallWithResponse';
import { toolRowLabel } from './toolCallCopy';
import { ListTodo, Terminal } from './icons/app-icons';

const here = dirname(fileURLToPath(import.meta.url));
const COLUMN = 760;

/** `main.css`'s values for the tokens the row reads. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --text-secondary: 13px;
  --text-secondary--line-height: 18px;
  --text-supporting: 12px;
  --text-supporting--line-height: 16px;
  --text-prose: 14px;
  --text-prose--line-height: 21px;
  --text-code: 13px;
  --text-code--line-height: 20px;
  --font-mono: ui-monospace, Menlo, monospace;
  --radius-element: 8px;
  --text-muted: #635c54;
  --text-default: #1c1b19;
  --text-danger: #a51a14;
  --overlay-hover: rgba(0, 0, 0, 0.05);
  --background-focus: #e0e0dc;
  --border-focus: #6b6963;
  --background-fill: #f4f4f2;
  --background-default: #ffffff;
  --background-muted: #f4f4f2;
  --dur-fast-min: 95ms;
  --dur-fast: 125ms;
  --dur-fast-max: 175ms;
  --dur-slow: 525ms;
  --ease-out: cubic-bezier(0.24, 1, 0.4, 1);
}
body { margin: 0; font: 14px/20px var(--font-body); }
/* main.css: the markdown prose line, which a well must be able to re-voice. */
.biorouter-markdown.prose { font-size: var(--text-prose); line-height: var(--text-prose--line-height); }
* { box-sizing: border-box; }
/* The Progress track's ground, a Tailwind utility (layered, as in the app). */
@layer utilities { .bg-background-muted { background-color: var(--background-muted); } }
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
  meta: { left: number; right: number; clipped: boolean } | null;
  well: { left: number; top: number } | null;
  /** The second row's glyph and head, when a row nests inside another's body. */
  nested: { glyphLeft: number; headHeight: number } | null;
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
        const nestedGlyph = document.querySelectorAll('.br-transcript-row-glyph')[1];
        const nestedHead = document.querySelectorAll('.br-transcript-row-head')[1];
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
                left: metaNode.getBoundingClientRect().left - origin,
                right: metaNode.getBoundingClientRect().right - origin,
                clipped: metaNode.scrollWidth > metaNode.clientWidth,
              }
            : null,
          well: well ? { left: well.left - origin, top: well.top } : null,
          nested:
            nestedGlyph && nestedHead
              ? {
                  glyphLeft: nestedGlyph.getBoundingClientRect().left - origin,
                  headHeight: nestedHead.getBoundingClientRect().height,
                }
              : null,
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

  // V2-R3-2: a to-do item was cut in its middle at 96 characters in a row with
  // room for all of it. The row's CSS ellipsis is now the only cut.
  const todoLabel = (status: string, text: string) =>
    toolRowLabel(
      summarizeToolCall(
        { name: 'todo__todo_update', arguments: { id: '#12', status } },
        {
          status: 'success',
          value: {
            content: [{ type: 'text', text: JSON.stringify({ task: { id: '12', text } }) }],
          },
        }
      ),
      'done'
    );

  it('shows a to-do item that fits the column whole, with no cut at all', async () => {
    if (!browser) return;
    // 98 characters: past the old 96-character cut, well inside the column.
    const item =
      'BLOCKED (2nd time): design intent still not received, the follow-up had only the header. ' +
      'Paste it.';
    const label = todoLabel('in_progress', item);
    expect(label).toBe(`Started “${item}”`);
    const g = await layOut(
      <TranscriptRow icon={ListTodo} label={label}>
        <pre>x</pre>
      </TranscriptRow>
    );
    expect(g.label.clipped).toBe(false);
    expect(g.label.right).toBeLessThan(COLUMN);
  });

  it('lets the column cut a long to-do item once, at its end, after the outcome', async () => {
    if (!browser) return;
    const label = todoLabel(
      'completed',
      'Audit git state under ~/Desktop/kdps-build, confirmed: kdps-build is not a repo; only ' +
        'kdps holds history (shallow clone, branch main, 98ccee1, clean tree)'
    );
    expect(label.startsWith('Completed “Audit git state')).toBe(true);
    expect(label).not.toContain('…');
    const g = await layOut(
      <TranscriptRow icon={ListTodo} label={label}>
        <pre>x</pre>
      </TranscriptRow>
    );
    expect(g.label.clipped).toBe(true);
    expect(g.head.right).toBeLessThanOrEqual(COLUMN + 0.5);
  });

  it('sets the meta one word space after the label, the chevron 8px after the meta', async () => {
    if (!browser) return;
    const g = await layOut(
      <TranscriptRow icon={Terminal} label="Explored" meta="· 2 files">
        <p>x</p>
      </TranscriptRow>
    );
    // "Explored · 2 files": 4px before the dot, about the space after it.
    expect(g.meta!.left - g.label.right).toBe(4);
    expect(g.chevron!.left).toBe(g.meta!.right + 8);
  });

  it('collapses an empty meta, so the chevron keeps its 8px from the label', async () => {
    if (!browser) return;
    const g = await layOut(
      <TranscriptRow icon={Terminal} label="Thinking" meta="">
        <p>x</p>
      </TranscriptRow>
    );
    expect(g.chevron!.left).toBe(g.label.right + 8);
  });

  /** The computed paint of every row's chevron, label and head, outermost first. */
  async function paint(element: React.ReactElement) {
    const html = renderToStaticMarkup(element);
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 600 } });
    try {
      await tab.setContent(
        `<!doctype html><html><head><style>${TOKENS}${css}</style></head><body>` +
          `<div id="column" style="width:${COLUMN}px;margin-left:100px">${html}</div></body></html>`
      );
      return await tab.evaluate(() => ({
        chevrons: [...document.querySelectorAll('.br-transcript-row-chevron')].map((node) => {
          const style = getComputedStyle(node);
          return { transform: style.transform, opacity: style.opacity };
        }),
        labels: [...document.querySelectorAll('.br-transcript-row-label')].map(
          (node) => getComputedStyle(node).animationName
        ),
        heads: [...document.querySelectorAll('.br-transcript-row-head')].map(
          (node) => getComputedStyle(node).color
        ),
      }));
    } finally {
      await tab.close();
    }
  }

  it("keeps a closed nested row's chevron at rest inside an open row", async () => {
    if (!browser) return;
    // Explored opens onto the calls themselves: the open row's rotated, full
    // strength chevron must not reach down into a row nested in its body.
    const p = await paint(
      <TranscriptRow icon={Terminal} label="Explored" body="plain" defaultOpen>
        <TranscriptRow icon={Terminal} label="Read a.ts">
          <pre>x</pre>
        </TranscriptRow>
      </TranscriptRow>
    );
    expect(p.chevrons).toHaveLength(2);
    expect(p.chevrons[0]).toEqual({ transform: 'matrix(0, 1, -1, 0, 0, 0)', opacity: '1' });
    expect(p.chevrons[1]).toEqual({ transform: 'none', opacity: '0.6' });
  });

  it("does not pass a running or failed row's state down to the rows in its body", async () => {
    if (!browser) return;
    const running = await paint(
      <TranscriptRow icon={Terminal} label="Exploring" body="plain" defaultOpen running>
        <TranscriptRow icon={Terminal} label="Read a.ts">
          <pre>x</pre>
        </TranscriptRow>
      </TranscriptRow>
    );
    expect(running.labels[0]).toBe('br-tool-text-pulse');
    expect(running.labels[1]).toBe('none');

    const danger = await paint(
      <TranscriptRow icon={Terminal} label="Failed" tone="danger" body="plain" defaultOpen>
        <TranscriptRow icon={Terminal} label="Read a.ts">
          <pre>x</pre>
        </TranscriptRow>
      </TranscriptRow>
    );
    expect(danger.heads[0]).toBe('rgb(165, 26, 20)');
    expect(danger.heads[1]).toBe('rgb(99, 92, 84)');
  });

  it("sets a tool's markdown output in the code role inside the well, and prose elsewhere", async () => {
    if (!browser) return;
    const html = renderToStaticMarkup(
      <>
        <TranscriptRow icon={Terminal} label="Ran ls" defaultOpen>
          <div className="biorouter-markdown prose" id="in-well">
            ok
          </div>
        </TranscriptRow>
        <TranscriptRow icon={Terminal} label="Thought" body="plain" defaultOpen>
          <div className="biorouter-markdown prose" id="in-plain">
            thinking
          </div>
        </TranscriptRow>
      </>
    );
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 600 } });
    try {
      await tab.setContent(
        `<!doctype html><html><head><style>${TOKENS}${css}</style></head><body>` +
          `<div id="column" style="width:${COLUMN}px;margin-left:100px">${html}</div></body></html>`
      );
      const voice = (id: string) =>
        tab.evaluate((target) => {
          const style = getComputedStyle(document.getElementById(target)!);
          return {
            size: style.fontSize,
            line: style.lineHeight,
            mono: /monospace/.test(style.fontFamily),
          };
        }, id);
      expect(await voice('in-well')).toEqual({ size: '13px', line: '20px', mono: true });
      expect(await voice('in-plain')).toEqual({ size: '14px', line: '21px', mono: false });
    } finally {
      await tab.close();
    }
  });

  /** Lay out `element` and read `read` off the page. */
  async function measure<T>(element: React.ReactElement, read: () => T): Promise<T> {
    const html = renderToStaticMarkup(element);
    const tab = await browser!.newPage({ viewport: { width: 1000, height: 600 } });
    try {
      await tab.setContent(
        `<!doctype html><html><head><style>${TOKENS}${css}</style></head><body>` +
          `<div id="column" style="width:${COLUMN}px;margin-left:100px">${html}</div></body></html>`
      );
      return (await tab.evaluate(read)) as T;
    } finally {
      await tab.close();
    }
  }

  it('rests the well on the subtle fill with no visible outline, its content still 12px in (R-27)', async () => {
    if (!browser) return;
    const well = await measure(
      <TranscriptRow icon={Terminal} label="Ran ls" defaultOpen>
        <pre id="content" style={{ margin: 0 }}>
          ok
        </pre>
      </TranscriptRow>,
      () => {
        const node = document.querySelector('.br-transcript-row-well')!;
        const style = getComputedStyle(node);
        const box = node.getBoundingClientRect();
        const content = document.getElementById('content')!.getBoundingClientRect();
        return {
          ground: style.backgroundColor,
          // The 1px border stays, transparent: nothing shows, and forced
          // colours draw it in the system ink, so the well keeps its box there.
          edge: `${style.borderTopWidth} ${style.borderTopStyle} ${style.borderTopColor}`,
          radius: style.borderTopLeftRadius,
          inset: [content.left - box.left, content.top - box.top],
        };
      }
    );
    expect(well).toEqual({
      ground: 'rgb(244, 244, 242)',
      edge: '1px solid rgba(0, 0, 0, 0)',
      radius: '8px',
      inset: [12, 12],
    });
  });

  it("steps a tool's progress track back to the page ground, so the whole track shows on the well", async () => {
    if (!browser) return;
    // The shared Progress paints its track in `--background-muted`, which is
    // the fill in both modes: on the well only the coral part showed.
    const grounds = await measure(
      <TranscriptRow icon={Terminal} label="Running sleep 30" defaultOpen>
        <div className="br-progress bg-background-muted" id="track" />
      </TranscriptRow>,
      () => {
        const well = getComputedStyle(document.querySelector('.br-transcript-row-well')!);
        return [
          getComputedStyle(document.getElementById('track')!).backgroundColor,
          well.backgroundColor,
        ];
      }
    );
    expect(grounds).toEqual(['rgb(255, 255, 255)', 'rgb(244, 244, 242)']);
  });

  it("breaks each line of an output paragraph once and keeps the line's spaces", async () => {
    if (!browser) return;
    // Chat markdown renders "a.txt\nb.txt" as `a.txt<br>` plus the newline
    // itself, and pre-wrap honoured both: every break drew a blank line.
    const p = await measure(
      <TranscriptRow icon={Terminal} label="Ran ls" defaultOpen>
        <div className="biorouter-markdown prose">
          <p id="out">
            {'a.txt'}
            <br />
            {'\nb.txt   c.txt'}
          </p>
        </div>
      </TranscriptRow>,
      () => {
        const node = document.getElementById('out')!;
        return {
          height: node.getBoundingClientRect().height,
          space: getComputedStyle(node).whiteSpace,
        };
      }
    );
    expect(p).toEqual({ height: 40, space: 'pre-wrap' });
  });

  it('steps a code block and a table in the output back to the page ground', async () => {
    if (!browser) return;
    const grounds = await measure(
      <TranscriptRow icon={Terminal} label="Queried" defaultOpen>
        <div className="biorouter-markdown prose">
          <div className="br-md-code" id="code">
            <pre>select 1;</pre>
          </div>
          <table className="br-md-table" id="table">
            <tbody>
              <tr>
                <td>1</td>
              </tr>
            </tbody>
          </table>
        </div>
      </TranscriptRow>,
      () =>
        ['code', 'table'].map(
          (id) => getComputedStyle(document.getElementById(id)!).backgroundColor
        )
    );
    expect(grounds).toEqual(['rgb(255, 255, 255)', 'rgb(255, 255, 255)']);
  });

  it("nests rows on a plain body with their glyphs on the outer label's edge", async () => {
    if (!browser) return;
    // The Explored row (ToolCallRows): the calls of a run sit on a plain body.
    const g = await layOut(
      <TranscriptRow icon={Terminal} label="Explored" meta="· 2 files" body="plain" defaultOpen>
        <TranscriptRow icon={Terminal} label="Read a.ts">
          <pre>x</pre>
        </TranscriptRow>
      </TranscriptRow>
    );
    expect(g.nested).toEqual({ glyphLeft: g.label.left, headHeight: 28 });
  });
});
