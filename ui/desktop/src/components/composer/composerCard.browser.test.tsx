// @vitest-environment node
/**
 * The chat composer card under the whole stylesheet, in a real layout engine (owner message 17,
 * R-32, which takes the card out of R-27's fill rule; spec 3.7).
 *
 * The card is the page ground with its resting hairline (`border-border-subtle/60`, the value it
 * had before R-27 and the pair Crew's card writes), the coral focus edge and the working sweep
 * on that same 1px border, and NOT a filled box: what sits in it rests as it does on any page
 * ground. The claims are about the cascade, which jsdom does not run: the hairline utility
 * against the unlayered focus edge in `main.css`, the control ground a filled box would hand its
 * children through `.br-fill-surface` (and this card must not), a thumbnail's tile, and forced
 * colours drawing the border. So the card's own class list (`composerCardClassName`, the one
 * `ChatInput` renders for a chat and for Home) and the real composer pieces (the `+` and its
 * popover, the queue, the chips row, the microphone, `Button`) are bundled with esbuild in
 * `ChatInput`'s markup (the one-line card and the footer line under it, owner 2026-11-13),
 * `styles/main.css` is compiled by Tailwind with the classes the bundle uses, the composer's
 * component CSS is appended as written, and Chromium paints them. Colours are compared with
 * probes that paint the token itself, so a token value can change without this file changing.
 *
 * Set `COMPOSER_SHOTS_DIR` to also save light and dark screenshots of the cards there.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { CHAT_COLUMN_CHROME, CHAT_MIN_WIDTH } from '../Layout/yieldLadder';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));
const SHOTS = process.env.COMPOSER_SHOTS_DIR;

/** Keep the real `+` and its tool lists (`toolsLists.tsx`); only the daemon-backed hooks are replaced. */
const toolLists: Plugin = {
  name: 'composer-tool-lists',
  setup(builder) {
    const lists: Record<string, { hook: string; panel: string; count: number }> = {
      './BottomMenuExtensionSelection': {
        hook: 'useExtensionPicker',
        panel: 'ExtensionsPanel',
        count: 12,
      },
      './BottomMenuSkillSelection': { hook: 'useSkillPicker', panel: 'SkillsPanel', count: 123 },
      './BottomMenuKnowledgeSelection': {
        hook: 'useKnowledgePicker',
        panel: 'KnowledgePanel',
        count: 45,
      },
    };
    builder.onResolve(
      { filter: /^\.\/BottomMenu(Extension|Skill|Knowledge)Selection$/ },
      (args) => ({ path: args.path, namespace: 'composer-tool-list' })
    );
    builder.onLoad({ filter: /.*/, namespace: 'composer-tool-list' }, ({ path }) => {
      const { hook, panel, count } = lists[path];
      return {
        contents: `
          export function ${hook}() {
            return { activeCount: ${count}, visibleCount: ${count}, answered: true, setQuery() {} };
          }
          export function ${panel}() { return null; }
        `,
        loader: 'tsx',
      };
    });
  },
};

/** Vite's `?url&no-inline` worklet import means nothing to esbuild; the worklet is not loaded here. */
const workletUrl: Plugin = {
  name: 'worklet-url',
  setup(builder) {
    builder.onResolve({ filter: /\.worklet\.js\?/ }, () => ({
      path: 'worklet',
      namespace: 'stub',
    }));
    builder.onLoad({ filter: /.*/, namespace: 'stub' }, () => ({
      contents: 'export default "";',
      loader: 'js',
    }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { composerCardClassName } from './composerCard';
import { ComposerChips } from './ComposerChips';
import { ComposerPlusMenu } from './ComposerPlusMenu';
import { MessageQueue } from '../MessageQueue';
import {
  NO_MODEL_CHIP_LABEL,
  NO_MODEL_COMPOSER_ACTION,
  NO_MODEL_COMPOSER_HINT,
} from '../composerNoProvider';
import { Button } from '../ui/button';
import { PrivacyBadge } from '../ui/PrivacyBadge';
import { ArrowUp, ChevronDown, Folder } from '../icons/app-icons';
import { DictationButton } from '../dictation/DictationButton';

/** The microphone's control at rest: what \`useDictationComposer\` hands it in a chat (DD-60). */
const micControl = (key) => ({
  key,
  available: true,
  activity: 'idle',
  holdsComposer: false,
  press() {},
  stop() {},
  cancel() {},
  handleKeyDown: () => false,
  noteInterest() {},
});

const IMAGE =
  'data:image/svg+xml;utf8,' +
  encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96"><rect width="96" height="96" fill="#7aa6c2"/><circle cx="64" cy="34" r="14" fill="#f2d16b"/><path d="M0 96 L36 50 L60 80 L74 64 L96 96Z" fill="#3f6f52"/></svg>'
  );

/** \`ModelEffortChip\`'s trigger as \`ModelsBottomBar\` draws it, with a long name and the padlock. */
function ModelChip() {
  return (
    <div data-testid="composer-model-chip" className="flex min-w-0 items-center">
      <div className="relative flex min-w-0 items-center">
        <button type="button" className="br-picker-chip" data-kind="model">
          <span className="br-picker-chip__name">gpt-5.6-sol-with-a-long-model-name · Deep</span>
          <span className="br-picker-chip__badge">
            <PrivacyBadge tier="private" dense />
          </span>
          <ChevronDown className="br-picker-chip__chevron" aria-hidden />
        </button>
      </div>
    </div>
  );
}

/**
 * \`ChatInput\`'s markup: the card (what goes with the message, then ONE row of \`+\`, the text,
 * the microphone and Send), and the footer line under it (the folder; the model, the ring and the
 * cost). \`decision\` draws a decision chat's row, which has no \`+\`.
 */
function Card({
  id,
  text = '',
  ready = false,
  dragging = false,
  working = false,
  decision = false,
  modelChip,
  children,
}: {
  id: string;
  text?: string;
  ready?: boolean;
  dragging?: boolean;
  working?: boolean;
  decision?: boolean;
  /** Replaces the model chip, as the no-model state does (no chevron: it opens the catalog). */
  modelChip?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div id={id + '-composer'} className="relative flex w-full flex-col gap-1">
      <div
        id={id}
        className={composerCardClassName({ isDraggingOver: dragging })}
        data-working={working ? 'true' : undefined}
        data-composer-empty={text ? undefined : 'true'}
      >
        {children}
        <div
          id={id + '-row'}
          data-testid="chat-input-toolbar"
          className="br-composer-row"
          data-plus={decision ? 'none' : undefined}
        >
          {!decision && (
            <span className="br-composer-row__slot">
              <ComposerPlusMenu sessionId={id} onAttachFiles={() => {}} onInsertTrigger={() => {}} />
            </span>
          )}
          <form className="br-composer-row__field">
            <textarea
              id={id + '-text'}
              rows={1}
              aria-label="Message"
              defaultValue={text}
              placeholder="Ask a follow-up"
              className="br-composer-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body text-text-default placeholder:text-text-muted"
            />
          </form>
          <span className="br-composer-row__slot">
            <DictationButton control={micControl(id)} setView={() => {}} />
          </span>
          <span className="br-composer-row__slot">
            <span className="flex-shrink-0">
              <Button
                id={id + '-send'}
                type="button"
                size="default"
                shape="round"
                variant={ready ? 'default' : 'secondary'}
                aria-label="Send"
                data-armed={ready ? 'true' : undefined}
                className="br-composer-send"
              >
                <span className="br-composer-send__glyph">
                  <ArrowUp className="size-4" />
                </span>
              </Button>
            </span>
          </span>
        </div>
      </div>
      <div id={id + '-footer'} className="br-footline br-composer-footer">
        <div className="br-footline__side">
          <span className="br-footline__item" data-testid="footer-folder">
            <Folder className="size-3.5" aria-hidden />
            <span className="br-footline__name">Desktop</span>
          </span>
        </div>
        <div className="br-footline__side" data-side="end">
          <div className="br-footline__model">{modelChip ?? <ModelChip />}</div>
          <span className="br-footline__item" data-testid="footer-ring">
            38% left
          </span>
          <span className="br-footline__item" data-testid="footer-cost">
            $0.42
          </span>
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
        <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr)', gap: 24 }}>
          <span>
            <Button
              id="bare-send"
              type="button"
              size="default"
              shape="round"
              variant="secondary"
              aria-label="Send"
            >
              <ArrowUp className="size-4" />
            </Button>
          </span>
          <Card id="rest" />
          <Card id="full" text="Compare the two fits and say which one holds." ready>
            <MessageQueue
              queuedMessages={QUEUE}
              onRemoveMessage={() => {}}
              onClearQueue={() => {}}
              onStopAndSend={() => {}}
              onReorderMessages={() => {}}
              onEditMessage={() => {}}
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
              <span className="text-supporting">{NO_MODEL_COMPOSER_HINT}</span>
              <Button type="button" variant="link" size="xs" className="h-auto px-0 text-supporting">
                {NO_MODEL_COMPOSER_ACTION}
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
          <Card
            id="nomodel"
            text="hello"
            modelChip={
              <button type="button" className="br-picker-chip" data-kind="model">
                <span className="br-picker-chip__name">{NO_MODEL_CHIP_LABEL}</span>
              </button>
            }
          >
            <div className="br-composer-notice">
              <span className="text-supporting">{NO_MODEL_COMPOSER_HINT}</span>
              <Button
                id="nomodel-action"
                type="button"
                variant="link"
                size="xs"
                className="h-auto px-0 text-supporting"
              >
                {NO_MODEL_COMPOSER_ACTION}
              </Button>
            </div>
          </Card>
          <Card id="typing" text="Compare the two fits" />
          <Card id="decision" text="Is the run done?" decision />
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

describe('the chat composer card under the whole stylesheet (R-32)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the composer card tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [toolLists, workletUrl],
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
      // The component stylesheets the bundle imports (esbuild drops them), as written, in the
      // order `ComposerPlusMenu` imports them: the composer's footer rules re-cut the footer's.
      readFileSync(resolve(here, '../bottom_menu/pickers.css'), 'utf8') +
      readFileSync(resolve(here, 'composer.css'), 'utf8');
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

  /** The resting hairline: `border-border-subtle/60`, as Tailwind compiles it. */
  const HAIRLINE = 'color-mix(in oklab, var(--border-subtle) 60%, transparent)';

  it('rests the card on the page ground with its hairline, on a chat and on Home', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const page = await paint(tab, 'var(--background-default)');
        const fill = await paint(tab, 'var(--background-fill)');
        const hairline = await edgeOf(tab, HAIRLINE);
        expect(hairline, `${mode} the hairline is a line`).not.toBe(
          await paint(tab, 'transparent')
        );
        for (const id of ['#rest', '#home']) {
          const card = await style(tab, id);
          expect(card.ground, `${mode} ${id} is the page ground`).toBe(page);
          expect(card.ground, `${mode} ${id} has no grey fill`).not.toBe(fill);
          expect(card.edge, `${mode} ${id} rests on its hairline`).toBe(hairline);
          expect(card.edgeWidth, `${mode} ${id} draws 1px`).toBe('1px');
        }
      } finally {
        await tab.close();
      }
    }
  });

  it('turns the hairline the accent while typing, without moving a pixel', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const hairline = await edgeOf(tab, HAIRLINE);
        const before = await style(tab, '#typing');
        await tab.focus('#typing-text');
        await tab.waitForTimeout(300);
        const focused = await style(tab, '#typing');
        expect(focused.edge, `${mode} focus edge`).toBe(await edgeOf(tab, 'var(--border-accent)'));
        expect(focused.ground, `${mode} the card keeps the page ground while focused`).toBe(
          await paint(tab, 'var(--background-default)')
        );
        expect(focused.size).toEqual(before.size);
        // Focusing Send is not "typing here": the hairline comes back.
        await tab.focus('#typing-send');
        await tab.waitForTimeout(300);
        expect((await style(tab, '#typing')).edge, `${mode} Send focused`).toBe(hairline);
      } finally {
        await tab.close();
      }
    }
  });

  it('runs the working sweep on the hairline, and dims a focused edge under it', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const sweep = await tab.evaluate(() => {
          const cs = getComputedStyle(document.getElementById('working')!, '::after');
          return { name: cs.animationName, image: cs.backgroundImage };
        });
        expect(sweep.name, `${mode} the sweep runs`).toBe('br-composer-working');
        expect(sweep.image).toContain('conic-gradient');
        expect((await style(tab, '#working')).edge, `${mode} working, not focused`).toBe(
          await edgeOf(tab, HAIRLINE)
        );
        await tab.focus('#working-text');
        await tab.waitForTimeout(300);
        expect((await style(tab, '#working')).edge, `${mode} working and focused`).toBe(
          await edgeOf(tab, 'color-mix(in oklab, var(--border-accent) 30%, transparent)')
        );
      } finally {
        await tab.close();
      }
    }
  });

  it('rests what sits in the card as it does on any page ground', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const page = await paint(tab, 'var(--background-default)');
        const fill = await paint(tab, 'var(--background-fill)');
        const strong = await paint(tab, 'var(--background-fill-strong)');
        // The quiet Send: the filled secondary button rests on the fill, exactly as it does
        // straight on the page. A filled box would have stepped it back to the page ground.
        const bare = (await style(tab, '#bare-send')).ground;
        expect(bare, `${mode} a quiet Send on the page`).toBe(fill);
        expect((await style(tab, '#rest-send')).ground, `${mode} quiet Send in the card`).toBe(
          bare
        );
        expect((await style(tab, '#home-send')).ground, `${mode} quiet Send on Home`).toBe(bare);
        expect(bare, `${mode} the quiet Send reads off the card`).not.toBe(page);
        await tab.hover('#rest-send');
        await tab.waitForTimeout(250);
        expect((await style(tab, '#rest-send')).ground, `${mode} quiet Send hovered`).toBe(strong);
        await away(tab);
        // The ready Send keeps the accent.
        expect((await style(tab, '#full-send')).ground).toBe(
          await paint(tab, 'var(--background-accent)')
        );
        // A file chip (the neutral badge) rests on the strong step, as anywhere.
        const chip = await style(tab, '#full [data-testid="composer-chips"] li:nth-child(2) > *');
        expect(chip.ground, `${mode} file chip`).toBe(strong);
        // The thumbnail draws no outline (R-27 still holds for it); its tile is the subtle fill,
        // so a loading or transparent image still shows a tile on the white card.
        const thumb = await style(tab, '#full [data-testid="composer-image-thumb"]');
        expect(thumb.edge, `${mode} thumbnail edge`).toBe(await paint(tab, 'transparent'));
        expect(thumb.ground, `${mode} thumbnail tile`).toBe(fill);
        // The queue's edit field: the shared field's rest on the fill, not the page ground.
        await tab.click('#full [data-testid="message-queue"] .br-queue-line');
        await tab.waitForTimeout(200);
        await tab.click('#full .br-queue-row .br-queue-edit');
        await tab.waitForSelector('#full .br-queue-row textarea');
        await tab.focus('#rest-text');
        await away(tab);
        await tab.waitForTimeout(250);
        const field = await style(tab, '#full .br-queue-row textarea');
        expect(field.ground, `${mode} queue edit field`).toBe(fill);
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
        expect(drag.ground, `${mode} drag-over is not the resting card`).not.toBe(
          await paint(tab, 'var(--background-default)')
        );
        expect(drag.shadow, `${mode} drag-over edge`).toContain('inset');
        // The quiet Send still reads on the strong fill.
        expect((await style(tab, '#drag-send')).ground, `${mode} quiet Send`).not.toBe(drag.ground);
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

  /**
   * FR-R2-9: the first screen after leaving first run showed one missing step as two nouns and
   * two controls, "Choose a provider" in the card's notice and "Choose a model" on the chip. The
   * notice's link and the chip must read the same, in a real layout, and the composer must stay
   * one card and its footer line (the notice sits above the text in the card, the chip on the
   * footer line under it).
   */
  it('names the one missing step the same way on the notice link and on the model chip', async (ctx) => {
    if (!browser) return ctx.skip();
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const words = await tab.evaluate(() => {
          const text = (el: Element | null) => (el?.textContent ?? '').trim();
          return {
            link: text(document.getElementById('nomodel-action')),
            chip: text(
              document.querySelector('#nomodel-footer .br-footline__model .br-picker-chip')
            ),
            chipInCard: document.querySelector('#nomodel .br-picker-chip') !== null,
            notice: text(document.querySelector('#nomodel .br-composer-notice > span')),
            // Every visible word the composer (card and footer line) carries about the
            // missing step, once each.
            composer: text(document.getElementById('nomodel-composer')),
          };
        });
        expect(words.link, `${mode} link`).toBe('Choose a model');
        expect(words.chip, `${mode} chip`).toBe(words.link);
        expect(words.chipInCard, `${mode} the chip is on the footer line`).toBe(false);
        expect(words.notice, `${mode} notice`).toBe('No model selected');
        expect(words.composer, `${mode} no second noun`).not.toMatch(/provider/i);
      } finally {
        await tab.close();
      }
    }
  });

  it.for([760, 440, CHAT_MIN_WIDTH])(
    'keeps +, the text, the microphone and Send on one row, and the model on the footer line, in a %ipx pane',
    async (width, ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open('light');
      try {
        await tab.evaluate(
          ({ width, chrome }) => {
            const pane = document.getElementById('chat-ground')!;
            pane.style.width = `${width}px`;
            pane.style.paddingInline = `${chrome / 2}px`;
          },
          { width, chrome: CHAT_COLUMN_CHROME }
        );
        // The counts the Tools chip carried are the `+`'s name now; it shows no text of its own.
        const plus = tab.locator('#rest [data-testid="composer-plus"]');
        expect(await plus.getAttribute('aria-label')).toBe(
          'Add files and tools. Tools: 12 extensions, 123 skills, 45 knowledge bases'
        );
        expect(await plus.textContent()).toBe('');
        const geometry = await tab.evaluate(() => {
          const card = document.getElementById('rest')!;
          const footer = document.getElementById('rest-footer')!;
          const box = (root: Element, selector: string) => {
            const el = root.querySelector<HTMLElement | SVGElement>(selector)!;
            const rect = el.getBoundingClientRect();
            return {
              left: rect.left,
              right: rect.right,
              top: rect.top,
              bottom: rect.bottom,
              width: rect.width,
              height: rect.height,
              scrollWidth: el.scrollWidth,
              clientWidth: el.clientWidth,
              display: getComputedStyle(el).display,
            };
          };
          // A readout's words: their own box, not the item's padded one.
          const words = (selector: string) => {
            const item = footer.querySelector<HTMLElement>(selector)!;
            const range = document.createRange();
            range.selectNodeContents(item);
            const text = range.getBoundingClientRect();
            const style = getComputedStyle(item);
            const inner = item.getBoundingClientRect();
            return {
              width: text.width,
              right: text.right,
              // The item's content box, which the words must fit in.
              room: inner.width - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight),
            };
          };
          return {
            card: card.getBoundingClientRect().toJSON(),
            row: box(card, '.br-composer-row'),
            rowChildren: card.querySelector('.br-composer-row')!.children.length,
            plus: box(card, '[data-testid="composer-plus"]'),
            plusGlyph: box(card, '[data-testid="composer-plus"] svg'),
            text: box(card, '#rest-text'),
            mic: box(card, '[data-testid="dictation-button"]'),
            send: box(card, '#rest-send'),
            modelInCard: card.querySelector('[data-kind="model"]') !== null,
            footer: { ...footer.getBoundingClientRect().toJSON(), scrollWidth: footer.scrollWidth },
            folderGlyph: box(footer, '[data-testid="footer-folder"] svg'),
            model: box(footer, '[data-kind="model"]'),
            name: box(footer, '[data-kind="model"] .br-picker-chip__name'),
            lock: box(footer, '[data-kind="model"] [data-testid="privacy-badge"]'),
            chevron: box(footer, '[data-kind="model"] .br-picker-chip__chevron'),
            ring: box(footer, '[data-testid="footer-ring"]'),
            cost: box(footer, '[data-testid="footer-cost"]'),
            ringWords: words('[data-testid="footer-ring"]'),
            costWords: words('[data-testid="footer-cost"]'),
          };
        });
        const tolerance = 0.5;
        const middle = (rect: { top: number; height: number }) => rect.top + rect.height / 2;
        expect(geometry.card.width).toBe(width - CHAT_COLUMN_CHROME);

        // THE CARD: one row, `+`, the text, the microphone and Send, left to right.
        expect(geometry.row.display).toBe('flex');
        expect(geometry.rowChildren).toBe(4);
        expect(geometry.row.height).toBe(32);
        expect(geometry.modelInCard).toBe(false);
        const ordered = [geometry.plus, geometry.text, geometry.mic, geometry.send];
        for (const control of ordered) {
          expect(control.left).toBeGreaterThanOrEqual(geometry.card.left);
          expect(control.right).toBeLessThanOrEqual(geometry.card.right);
          // One centre line, the 28px microphone included.
          expect(Math.abs(middle(control) - middle(geometry.text))).toBeLessThanOrEqual(tolerance);
        }
        for (let index = 1; index < ordered.length; index++) {
          expect(ordered[index - 1].right).toBeLessThanOrEqual(ordered[index].left + tolerance);
        }
        // The text takes what the controls leave, at every width.
        expect(geometry.text.width).toBeGreaterThan(100);
        expect([geometry.plus.width, geometry.plus.height]).toEqual([32, 32]);
        expect([geometry.send.width, geometry.send.height]).toEqual([32, 32]);
        // The microphone: the 28px ghost round button, immediately left of Send (DD-60).
        expect([geometry.mic.width, geometry.mic.height]).toEqual([28, 28]);
        expect(geometry.send.left - geometry.mic.right).toBe(4);
        // Send on the card's inner right edge (1px border, 8px padding), the `+` glyph on the
        // stack's 16px edge (and the 32px button's 8px inset).
        expect(geometry.card.right - geometry.send.right).toBe(1 + 8);
        expect(geometry.plusGlyph.left - geometry.card.left).toBe(1 + 8 + 8);

        // THE FOOTER LINE: as wide as the card, the folder glyph under the `+` glyph.
        expect(geometry.footer.left).toBe(geometry.card.left);
        expect(geometry.footer.right).toBe(geometry.card.right);
        expect(Math.abs(geometry.folderGlyph.left - geometry.plusGlyph.left)).toBeLessThanOrEqual(
          tolerance
        );
        // The ring and the cost never give way (composer.css): only the folder's and the
        // model's names ellipsize, so each readout's words fit inside its own padding.
        for (const [name, readout] of [
          ['ring', geometry.ringWords],
          ['cost', geometry.costWords],
        ] as const) {
          expect(
            readout.width,
            `the ${name}'s words fit its box at ${width}px: ${JSON.stringify(readout)}`
          ).toBeLessThanOrEqual(readout.room + tolerance);
        }
        // So the last readout's words end on Send's outer edge.
        expect(Math.abs(geometry.costWords.right - geometry.send.right)).toBeLessThanOrEqual(
          tolerance
        );
        // The model first on the right, then the ring and the cost, on one line, all inside it.
        const readouts = [geometry.model, geometry.ring, geometry.cost];
        for (const item of readouts) {
          expect(item.left).toBeGreaterThanOrEqual(geometry.footer.left);
          expect(item.right).toBeLessThanOrEqual(geometry.footer.right + tolerance);
          expect(Math.abs(middle(item) - middle(geometry.ring))).toBeLessThanOrEqual(tolerance);
        }
        for (let index = 1; index < readouts.length; index++) {
          expect(readouts[index - 1].right).toBeLessThanOrEqual(readouts[index].left + tolerance);
        }
        expect(geometry.folderGlyph.right).toBeLessThanOrEqual(geometry.model.left);
        expect(geometry.footer.scrollWidth).toBeLessThanOrEqual(geometry.footer.width);
        // In the footer the chip is a readout: 24px, no chevron, its name giving way first.
        expect(geometry.model.height).toBe(24);
        expect(geometry.chevron.display).toBe('none');
        expect(geometry.name.right).toBeLessThanOrEqual(geometry.lock.left + tolerance);
        expect(geometry.lock.width).toBe(12);
        expect(geometry.lock.height).toBe(12);
        expect(
          geometry.lock.right,
          `the model contents stay inside their trigger: ${JSON.stringify({
            model: geometry.model,
            name: geometry.name,
            lock: geometry.lock,
          })}`
        ).toBeLessThanOrEqual(geometry.model.right + tolerance);
        expect(geometry.name.width, 'the model name keeps room for its ellipsis').toBeGreaterThan(
          12
        );
        expect(geometry.name.scrollWidth).toBeGreaterThan(geometry.name.clientWidth);
        if (SHOTS) {
          mkdirSync(SHOTS, { recursive: true });
          await tab.locator('#rest-composer').screenshot({
            path: resolve(SHOTS, `composer-card-light-pane-${width}.png`),
          });
        }
      } finally {
        await tab.close();
      }
    }
  );

  it('starts a decision chat’s text on the stack edge, with no +, and keeps the microphone and Send', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('light');
    try {
      const geometry = await tab.evaluate(() => {
        const rect = (selector: string) =>
          document.querySelector(selector)!.getBoundingClientRect();
        return {
          plus: document.querySelector('#decision [data-testid="composer-plus"]') !== null,
          plusAttr: document.getElementById('decision-row')!.getAttribute('data-plus'),
          card: rect('#decision').toJSON(),
          text: rect('#decision-text').toJSON(),
          mic: rect('#decision [data-testid="dictation-button"]').toJSON(),
          send: rect('#decision-send').toJSON(),
          restPlusGlyph: rect('#rest [data-testid="composer-plus"] svg').toJSON(),
          rest: rect('#rest').toJSON(),
        };
      });
      expect(geometry.plus).toBe(false);
      expect(geometry.plusAttr).toBe('none');
      // The text starts where an ordinary chat's `+` glyph does: the stack's 16px edge.
      expect(geometry.text.left - geometry.card.left).toBe(1 + 8 + 8);
      expect(geometry.text.left - geometry.card.left).toBe(
        geometry.restPlusGlyph.left - geometry.rest.left
      );
      expect(geometry.card.right - geometry.send.right).toBe(1 + 8);
      expect(geometry.send.left - geometry.mic.right).toBe(4);
    } finally {
      await tab.close();
    }
  });

  it('saves screenshots when asked', async (ctx) => {
    if (!browser || !SHOTS) return ctx.skip();
    mkdirSync(SHOTS, { recursive: true });
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(mode);
      try {
        const shot = (selector: string, name: string) =>
          tab
            .locator(selector)
            .screenshot({ path: resolve(SHOTS, `composer-card-${mode}-${name}.png`) });
        await tab.screenshot({ path: resolve(SHOTS, `composer-card-${mode}.png`), fullPage: true });
        // At rest on a chat and on Home, and the drag-over card.
        await shot('#rest-composer', 'rest');
        await shot('#decision-composer', 'decision');
        await shot('#home-ground', 'home');
        await shot('#drag', 'drag-over');
        // Focused while empty, then typing (focused with text).
        await tab.focus('#rest-text');
        await tab.waitForTimeout(300);
        await shot('#rest', 'focused');
        await tab.focus('#typing-text');
        await tab.waitForTimeout(300);
        await shot('#typing', 'typing');
        await tab.focus('#home-text');
        await tab.waitForTimeout(300);
        await shot('#home-ground', 'home-focused');
        // The quiet Send hovered.
        await tab.focus('#rest-send');
        await tab.hover('#rest-send');
        await tab.waitForTimeout(250);
        await shot('#rest', 'send-hover');
        await away(tab);
        // The queue unfolded, with a row hovered; the chips and thumbnails under it.
        await tab.click('#full [data-testid="message-queue"] .br-queue-line');
        await tab.waitForTimeout(200);
        await tab.hover('#full .br-queue-row');
        await tab.waitForTimeout(250);
        await shot('#full', 'queue');
        await shot('#refs', 'refs');
        await shot('#nomodel', 'no-model');
        // The queue's edit field open.
        await tab.click('#full .br-queue-row .br-queue-edit');
        await tab.waitForSelector('#full .br-queue-row textarea');
        await tab.waitForTimeout(250);
        await shot('#full', 'queue-edit');
      } finally {
        await tab.close();
      }
    }
  });
});
