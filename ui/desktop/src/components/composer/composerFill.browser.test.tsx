// @vitest-environment node
/**
 * The chat composer card under the whole stylesheet, in a real layout engine (owner message 13,
 * R-27 "fills before lines", spec 3.7 and section 9).
 *
 * The claims are about the cascade, which jsdom does not run: the card's `border-transparent`
 * utility against the unlayered focus edge in `main.css`, the private control ground the card
 * hands its children through `.br-fill-surface`, a neutral chip's strong step off the card, and
 * forced colours drawing the transparent border. So the card's own class list
 * (`composerCardClassName`, the one `ChatInput` renders) and the real composer pieces (the `+`
 * menu, the queue, the chips row, `Button`) are bundled with esbuild, `styles/main.css` is
 * compiled by Tailwind with the classes the bundle uses, the composer's component CSS is appended
 * as written, and Chromium paints them. Colours are compared with probes that paint the token
 * itself, so a token value can change without this file changing.
 *
 * Set `COMPOSER_SHOTS_DIR` to also save light and dark screenshots of the cards there.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.COMPOSER_SHOTS_DIR;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { composerCardClassName } from './composerCard';
import { ComposerChips } from './ComposerChips';
import { ComposerPlusMenu } from './ComposerPlusMenu';
import { MessageQueue } from '../MessageQueue';
import { Button } from '../ui/button';
import { ArrowUp, ChevronDown, Folder } from '../icons/app-icons';

const IMAGE =
  'data:image/svg+xml;utf8,' +
  encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96"><rect width="96" height="96" fill="#7aa6c2"/><circle cx="64" cy="34" r="14" fill="#f2d16b"/><path d="M0 96 L36 50 L60 80 L74 64 L96 96Z" fill="#3f6f52"/></svg>'
  );

function Chip({ children }: { children: React.ReactNode }) {
  return (
    <button type="button" className="br-picker-chip">
      {children}
      <ChevronDown className="br-picker-chip__chevron" aria-hidden />
    </button>
  );
}

function Card({
  id,
  text = '',
  ready = false,
  dragging = false,
  working = false,
  children,
}: {
  id: string;
  text?: string;
  ready?: boolean;
  dragging?: boolean;
  working?: boolean;
  children?: React.ReactNode;
}) {
  return (
    <div className="relative flex w-full flex-col gap-1">
      <div
        id={id}
        className={composerCardClassName({ isDraggingOver: dragging })}
        data-working={working ? 'true' : undefined}
      >
        {children}
        <textarea
          id={id + '-text'}
          rows={1}
          aria-label="Message"
          defaultValue={text}
          placeholder="Ask a follow-up"
          className="br-composer-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body text-text-default placeholder:text-text-muted"
        />
        <div className="br-composer-controls">
          <ComposerPlusMenu onAttachFiles={() => {}} onInsertTrigger={() => {}} />
          <Chip>
            <span>Tools</span>
            <span className="br-picker-chip__count">15</span>
          </Chip>
          <span className="br-composer-controls__spacer" />
          <div className="br-composer-controls__model">
            <Chip>
              <span className="br-picker-chip__name">gpt-5.6-sol</span>
            </Chip>
          </div>
          <span className="flex-shrink-0">
            <Button
              id={id + '-send'}
              type="button"
              size="default"
              shape="round"
              variant={ready ? 'default' : 'secondary'}
              aria-label="Send"
            >
              <ArrowUp className="size-4" />
            </Button>
          </span>
        </div>
      </div>
      <div className="br-footline br-composer-footer">
        <div className="br-footline__side">
          <span className="br-footline__item">
            <Folder className="size-3.5" aria-hidden />
            <span className="br-footline__name">Desktop</span>
          </span>
        </div>
        <div className="br-footline__side" data-side="end">
          <span className="br-footline__item">38% left</span>
          <span className="br-footline__item">$0.42</span>
        </div>
      </div>
    </div>
  );
}

const QUEUE = [
  { id: 'q1', content: 'and rerun the fit with the outliers removed', timestamp: 1 },
  { id: 'q2', content: 'then plot the residuals', timestamp: 2 },
];

function App() {
  return (
    <div style={{ display: 'grid', gap: 0 }}>
      <div id="chat-ground" className="bg-background-default" style={{ padding: 24, width: 760 }}>
        <div style={{ display: 'grid', gap: 24 }}>
          <Card id="rest" />
          <Card id="full" text="Compare the two fits and say which one holds." ready>
            <MessageQueue
              queuedMessages={QUEUE}
              onRemoveMessage={() => {}}
              onClearQueue={() => {}}
              onStopAndSend={() => {}}
              onReorderMessages={() => {}}
            />
            <ComposerChips
              refs={[]}
              images={[{ id: 'i1', dataUrl: IMAGE, isLoading: false }]}
              files={[
                { id: 'f1', path: '/tmp/fit.csv', name: 'fit.csv', type: 'text/csv', isImage: false },
                {
                  id: 'f2',
                  path: '/tmp/scan.tiff',
                  name: 'scan.tiff',
                  type: 'image/tiff',
                  isImage: false,
                  error: 'This file could not be read.',
                },
              ]}
              onRemoveRef={() => {}}
              onRemoveImage={() => {}}
              onRetryImage={() => {}}
              onRemoveFile={() => {}}
            />
          </Card>
          <Card id="refs" text="Plot these">
            <div className="br-composer-notice">
              <span className="text-supporting">No model selected</span>
              <Button type="button" variant="link" size="xs" className="h-auto px-0 text-supporting">
                Choose a provider
              </Button>
            </div>
            <ComposerChips
              refs={[
                { kind: 'skill', value: 'ggplot', start: 0, end: 0, raw: '' },
                { kind: 'knowledge_base', value: 'kb1', label: 'Soul', start: 0, end: 0, raw: '' },
                {
                  kind: 'quote',
                  value: 'q',
                  label: 'Response',
                  start: 0,
                  end: 0,
                },
              ]}
              images={[{ id: 'i2', dataUrl: IMAGE, isLoading: true }]}
              files={[]}
              onRemoveRef={() => {}}
              onRemoveImage={() => {}}
              onRetryImage={() => {}}
              onRemoveFile={() => {}}
            />
          </Card>
          <Card id="drag" dragging />
          <Card id="working" text="Summarise the run" working />
        </div>
      </div>
      <div id="home-ground" className="bg-background-canvas" style={{ padding: 24, width: 760 }}>
        <Card id="home" />
      </div>
    </div>
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

describe('the chat composer card under the whole stylesheet (R-27)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the composer fill tests.');
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
      // The component stylesheets the bundle imports (esbuild drops them), as written.
      readFileSync(resolve(here, 'composer.css'), 'utf8') +
      readFileSync(resolve(here, '../bottom_menu/pickers.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(mode: 'light' | 'dark', forcedColors = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 808, height: 1100 } });
    if (forcedColors) await tab.emulateMedia({ forcedColors: 'active' });
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#home-send');
    // Let every colour transition settle before reading a colour.
    await tab.waitForTimeout(300);
    return tab;
  }

  /** The computed colour a probe inside `within` paints for `value` (a token, in that place). */
  const paint = (tab: Page, value: string, within = 'body') =>
    tab.evaluate(
      ([v, w]) => {
        const probe = document.createElement('div');
        probe.style.backgroundColor = v;
        document.querySelector(w)!.append(probe);
        const out = getComputedStyle(probe).backgroundColor;
        probe.remove();
        return out;
      },
      [value, within] as const
    );
  const edgeOf = (tab: Page, value: string) =>
    tab.evaluate((v) => {
      const probe = document.createElement('div');
      probe.style.borderTop = `1px solid ${v}`;
      document.body.append(probe);
      const out = getComputedStyle(probe).borderTopColor;
      probe.remove();
      return out;
    }, value);
  const style = (tab: Page, selector: string) =>
    tab.evaluate((s) => {
      const el = document.querySelector(s)!;
      const cs = getComputedStyle(el);
      const box = el.getBoundingClientRect();
      return {
        ground: cs.backgroundColor,
        edge: cs.borderTopColor,
        edgeWidth: cs.borderTopWidth,
        size: [box.width, box.height],
      };
    }, selector);
  const away = (tab: Page) => tab.mouse.move(800, 1090);

  it('rests the card on the subtle fill with no line around it, on a chat and on Home', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const fill = await paint(tab, 'var(--background-fill)');
        const clear = await paint(tab, 'transparent');
        for (const id of ['#rest', '#home']) {
          const card = await style(tab, id);
          expect(card.ground, `${mode} ${id} is the fill`).toBe(fill);
          expect(card.edge, `${mode} ${id} draws no resting line`).toBe(clear);
          expect(card.edgeWidth, `${mode} ${id} keeps its 1px room`).toBe('1px');
        }
        expect(fill, `${mode} the fill is a step off the chat's ground`).not.toBe(
          await paint(tab, 'var(--background-default)', '#chat-ground')
        );
        expect(fill, `${mode} the fill is a step off Home's canvas`).not.toBe(
          await paint(tab, 'var(--background-canvas)', '#home-ground')
        );
      } finally {
        await tab.close();
      }
    }
  });

  it('turns the edge the accent while typing, without moving a pixel', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const before = await style(tab, '#rest');
        await tab.focus('#rest-text');
        await tab.waitForTimeout(300);
        const focused = await style(tab, '#rest');
        expect(focused.edge, `${mode} focus edge`).toBe(await edgeOf(tab, 'var(--border-accent)'));
        expect(focused.ground, `${mode} the card keeps its fill while focused`).toBe(
          await paint(tab, 'var(--background-fill)')
        );
        expect(focused.size).toEqual(before.size);
        // Focusing Send is not "typing here".
        await tab.focus('#rest-send');
        await tab.waitForTimeout(300);
        expect((await style(tab, '#rest')).edge).toBe(await paint(tab, 'transparent'));
      } finally {
        await tab.close();
      }
    }
  });

  it('steps what sits in the card back to the page ground, and a chip a step past the card', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const page = await paint(tab, 'var(--background-default)');
        const fill = await paint(tab, 'var(--background-fill)');
        const strong = await paint(tab, 'var(--background-fill-strong)');
        // The quiet Send: the filled secondary button, on the page ground inside the card.
        expect((await style(tab, '#rest-send')).ground, `${mode} quiet Send`).toBe(page);
        await tab.hover('#rest-send');
        await tab.waitForTimeout(250);
        expect((await style(tab, '#rest-send')).ground, `${mode} quiet Send hovered`).toBe(strong);
        await away(tab);
        // The ready Send keeps the accent.
        expect((await style(tab, '#full-send')).ground).toBe(
          await paint(tab, 'var(--background-accent)')
        );
        // A file chip (the neutral badge) rests on the strong step, never on the card's own fill.
        const chip = await style(tab, '#full [data-testid="composer-chips"] li:nth-child(2) > *');
        expect(chip.ground, `${mode} file chip`).toBe(strong);
        expect(chip.ground).not.toBe(fill);
        // The thumbnail draws no outline; its tile is the page ground.
        const thumb = await style(tab, '#full [data-testid="composer-image-thumb"]');
        expect(thumb.edge, `${mode} thumbnail edge`).toBe(await paint(tab, 'transparent'));
        expect(thumb.ground).toBe(page);
      } finally {
        await tab.close();
      }
    }
  });

  it('shows a drag over the card as the strong fill and the inset accent edge', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const drag = await tab.evaluate(() => {
          const cs = getComputedStyle(document.getElementById('drag')!);
          return { ground: cs.backgroundColor, shadow: cs.boxShadow };
        });
        expect(drag.ground, `${mode} drag-over ground`).toBe(
          await paint(tab, 'var(--background-fill-strong)')
        );
        expect(drag.shadow, `${mode} drag-over edge`).toContain('inset');
        expect((await style(tab, '#drag')).size).toEqual((await style(tab, '#rest')).size);
      } finally {
        await tab.close();
      }
    }
  });

  it('draws the card’s edge in forced colours, at the same size', async (ctx) => {
    if (!browser) return ctx.skip();
    const rest = await open('light');
    const forced = await open('light', true);
    try {
      const canvas = await paint(forced, 'Canvas');
      for (const id of ['#rest', '#home']) {
        const before = await style(rest, id);
        const after = await style(forced, id);
        expect(after.edgeWidth, `${id} keeps a 1px edge`).toBe('1px');
        expect(after.edge, `${id}'s edge is drawn`).not.toBe(canvas);
        expect(after.edge).not.toMatch(/rgba\(.*,\s*0\)$/);
        expect(after.size, `${id} costs no geometry`).toEqual(before.size);
      }
    } finally {
      await rest.close();
      await forced.close();
    }
  });

  it('saves screenshots when asked', async (ctx) => {
    if (!browser || !SHOTS) return ctx.skip();
    mkdirSync(SHOTS, { recursive: true });
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        await tab.screenshot({ path: resolve(SHOTS, `composer-fill-${mode}.png`), fullPage: true });
        await tab.focus('#rest-text');
        await tab.waitForTimeout(300);
        await tab
          .locator('#chat-ground')
          .screenshot({ path: resolve(SHOTS, `composer-fill-${mode}-focused.png`) });
        // The queue unfolded, with a row hovered.
        await tab.click('#full [data-testid="message-queue"] .br-queue-line');
        await tab.waitForTimeout(200);
        await tab.hover('#full .br-queue-row');
        await tab.waitForTimeout(250);
        await tab
          .locator('#full')
          .screenshot({ path: resolve(SHOTS, `composer-fill-${mode}-queue.png`) });
      } finally {
        await tab.close();
      }
    }
  });
});
