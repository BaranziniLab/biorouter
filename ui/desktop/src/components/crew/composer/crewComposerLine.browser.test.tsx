// @vitest-environment node
/**
 * Crew's message box is one line, like the chat composer (owner, 2026-11-13; d099eb2cb), measured
 * in a real layout engine beside the chat composer's card.
 *
 * jsdom lays nothing out, runs no Tailwind and does not size a textarea, so only Chromium can say
 * that the two cards are the same box. The REAL Crew `Composer` is bundled with esbuild over the
 * test controller the composer's own tests use (`crewTestController`, made stateful here so typing
 * keeps the draft, as the real controller does); only its transfers module, which reaches the
 * daemon, is a stand-in. The chat card is `ChatInput`'s markup as `composerCard.browser.test.tsx`
 * draws it (the card's own class list, the real `+` and its popover, the real microphone and
 * `Button`), because `ChatInput` itself needs the app's contexts. `styles/main.css` is compiled
 * by Tailwind with the classes the bundle uses, the component stylesheets are appended as
 * written, and both are laid out side by side in panes of 760, 440 and 360px:
 *
 *  - Crew's row is ONE 32px line at rest: `+`, the text, the agent, the microphone and Send, on
 *    one centre line, Send last and on the card's inner right edge;
 *  - with three lines of text the controls keep to the last line;
 *  - Crew's card at rest is `--crew-compose-rest-height` tall;
 *  - the two cards have the same padding, row height, rest height, and `+` and Send places,
 *    each measured from its own card's edges (the cards differ in width: their bars' gutters
 *    are their own).
 *
 * Set `CREW_SHOTS_DIR` to also save light and dark screenshots of both cards, empty and with
 * three lines of text.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { CHAT_COLUMN_CHROME, CHAT_MIN_WIDTH } from '../../Layout/yieldLadder';
import { composerCopy } from './copy';

const here = dirname(fileURLToPath(import.meta.url));
const COMPONENTS = resolve(here, '../..');
const STYLES = resolve(COMPONENTS, '../styles');
const requireFromDesktop = createRequire(resolve(COMPONENTS, '../../package.json'));
const SHOTS = process.env.CREW_SHOTS_DIR;

/** The `+` popover's three lists, whose hooks read the daemon. */
const toolLists: Plugin = {
  name: 'composer-tool-lists',
  setup(builder) {
    const lists: Record<string, { hook: string; panel: string }> = {
      './BottomMenuExtensionSelection': { hook: 'useExtensionPicker', panel: 'ExtensionsPanel' },
      './BottomMenuSkillSelection': { hook: 'useSkillPicker', panel: 'SkillsPanel' },
      './BottomMenuKnowledgeSelection': { hook: 'useKnowledgePicker', panel: 'KnowledgePanel' },
    };
    builder.onResolve(
      { filter: /^\.\/BottomMenu(Extension|Skill|Knowledge)Selection$/ },
      (args) => ({ path: args.path, namespace: 'composer-tool-list' })
    );
    builder.onLoad({ filter: /.*/, namespace: 'composer-tool-list' }, ({ path }) => {
      const { hook, panel } = lists[path];
      return {
        contents: `
          export function ${hook}() {
            return { activeCount: 1, visibleCount: 1, answered: true, setQuery() {} };
          }
          export function ${panel}() { return null; }
        `,
        loader: 'tsx',
      };
    });
  },
};

/** Crew's transfers reach the main process and the daemon; nothing here transfers a file. */
const crewTransfers: Plugin = {
  name: 'crew-transfers',
  setup(builder) {
    builder.onResolve({ filter: /^\.\.\/crewTransfers$/ }, () => ({
      path: 'crewTransfers',
      namespace: 'crew-transfers-stub',
    }));
    builder.onLoad({ filter: /.*/, namespace: 'crew-transfers-stub' }, () => ({
      contents: `
        export async function chooseTransferFile() { return null; }
        export async function listTransfers() { return []; }
        export async function beginTransfer() { return null; }
        export async function resumeTransfer() { return null; }
        export async function pauseTransfer() { return {}; }
        export async function cancelUpload() { return 'cancelled'; }
        export async function forgetTransfer() { return {}; }
        export async function clearPublishedTransfers() { return {}; }
        export async function previewAttachment() { return null; }
      `,
      loader: 'js',
    }));
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
import { Composer } from './Composer';
import { CrewTestProvider, crewTestController } from '../files/crewTestController';
import { composerCardClassName } from '../../composer/composerCard';
import { ComposerPlusMenu } from '../../composer/ComposerPlusMenu';
import { DictationButton } from '../../dictation/DictationButton';
import { Button } from '../../ui/button';
import { ArrowUp } from '../../icons/app-icons';

/** The microphone's control at rest, as \`useDictationComposer\` hands it in a chat. */
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

/**
 * \`ChatInput\`'s card and its one row, as \`composerCard.browser.test.tsx\` draws them. Send arms
 * once there is text, as \`ChatInput\`'s does.
 */
function ChatCard() {
  const [text, setText] = React.useState('');
  const armed = text.trim().length > 0;
  return (
    <div className="relative flex w-full flex-col gap-1">
      <div id="chat-card" className={composerCardClassName({ isDraggingOver: false })}>
        <div id="chat-row" className="br-composer-row">
          <span className="br-composer-row__slot">
            <ComposerPlusMenu sessionId="chat-1" onAttachFiles={() => {}} onInsertTrigger={() => {}} />
          </span>
          <form className="br-composer-row__field">
            <textarea
              id="chat-text"
              rows={1}
              aria-label="Message"
              placeholder="Ask a follow-up"
              value={text}
              onChange={(event) => setText(event.target.value)}
              className="br-composer-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body text-text-default placeholder:text-text-muted"
            />
          </form>
          <span className="br-composer-row__slot">
            <DictationButton control={micControl('chat')} setView={() => {}} />
          </span>
          <span className="br-composer-row__slot">
            <span className="flex-shrink-0">
              <Button id="chat-send" type="button" size="default" shape="round" variant={armed ? 'default' : 'secondary'} data-armed={armed ? 'true' : undefined} aria-label="Send" className="br-composer-send">
                <span className="br-composer-send__glyph"><ArrowUp className="size-4" /></span>
              </Button>
            </span>
          </span>
        </div>
      </div>
    </div>
  );
}

/** Crew's real composer over a controller that keeps its draft, as the real controller does. */
function CrewComposer() {
  const [body, setBody] = React.useState('');
  const controller = React.useMemo(
    () => crewTestController({ draft: { body, attachments: [], references: [] }, setBody }),
    [body]
  );
  return (
    <CrewTestProvider controller={controller}>
      <Composer />
    </CrewTestProvider>
  );
}

function App() {
  return (
    <div id="panes" style={{ display: 'flex', alignItems: 'flex-end', gap: 40, padding: 24 }}>
      <div id="chat-pane" className="bg-background-default" style={{ width: 760, paddingInline: ${CHAT_COLUMN_CHROME / 2}, paddingBlock: 24, flex: 'none' }}>
        <ChatCard />
      </div>
      <div id="crew-pane" style={{ width: 760, flex: 'none' }}>
        <CrewComposer />
      </div>
    </div>
  );
}

createRoot(document.getElementById('root')).render(<App />);
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

type Rect = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
};

/** One card measured from its own edges, the way the two are compared. */
interface CardGeometry {
  card: Rect;
  padding: [number, number, number, number];
  border: number;
  row: Rect;
  rowChildren: string[];
  plus: Rect;
  plusGlyph: Rect;
  text: Rect;
  agent: Rect | null;
  mic: Rect;
  send: Rect;
  sendIsLast: boolean;
}

const TEXT = 'one\ntwo\nthree';
const TOLERANCE = 0.5;
const middle = (rect: { top: number; height: number }) => rect.top + rect.height / 2;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    TOLERANCE
  );

describe('Crew’s message box is the chat composer’s one-line card', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Crew composer line tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [toolLists, crewTransfers, workletUrl],
      // Vite's own build-time values, read by modules Crew's composer imports.
      define: {
        'process.env.NODE_ENV': '"production"',
        'import.meta.env': '{"MODE":"production","DEV":false,"PROD":true}',
      },
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
      // The component stylesheets the bundle imports (esbuild drops them), as written: the chat
      // composer's in the order `ComposerPlusMenu` imports them, then Crew's composer.
      readFileSync(resolve(COMPONENTS, 'bottom_menu/pickers.css'), 'utf8') +
      readFileSync(resolve(COMPONENTS, 'composer/composer.css'), 'utf8') +
      readFileSync(resolve(here, 'composer.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(width: number, mode: 'light' | 'dark' = 'light'): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 1640, height: 600 } });
    const errors: string[] = [];
    tab.on('pageerror', (error) => errors.push(error.message));
    await tab.setContent(
      `<!doctype html><html class="${mode === 'dark' ? 'dark' : ''}"><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-canvas)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    try {
      await tab.waitForSelector('.crew-compose-card textarea', { timeout: 10_000 });
      await tab.waitForSelector('#chat-text');
    } catch (error) {
      throw new Error(`The composers did not render: ${errors.join(' | ') || String(error)}`);
    }
    await tab.evaluate((w) => {
      for (const id of ['chat-pane', 'crew-pane'])
        document.getElementById(id)!.style.width = `${w}px`;
    }, width);
    // Let every colour transition and the auto-grow settle before reading anything.
    await settle(tab);
    return tab;
  }

  const settle = async (tab: Page) => {
    await tab.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done)))
    );
    await tab.waitForTimeout(300);
  };

  /** Types into both fields the way a person does, so Crew's draft and auto-grow follow. */
  const type = async (tab: Page, text: string) => {
    await tab.fill('#chat-text', text);
    await tab.fill('.crew-compose-card textarea', text);
    await tab.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    await settle(tab);
  };

  const measure = (tab: Page) =>
    tab.evaluate(
      ({ attach, agent, send }) => {
        const rect = (el: Element | null) => {
          if (!el) return null;
          const r = el.getBoundingClientRect();
          return {
            left: r.left,
            right: r.right,
            top: r.top,
            bottom: r.bottom,
            width: r.width,
            height: r.height,
          };
        };
        const geometry = (
          card: HTMLElement,
          row: HTMLElement,
          buttons: {
            plus: HTMLElement;
            agent: HTMLElement | null;
            send: HTMLElement;
          }
        ) => {
          const cs = getComputedStyle(card);
          const sendSlot = buttons.send.closest('.crew-compose-slot, .br-composer-row__slot');
          return {
            card: rect(card)!,
            padding: [cs.paddingTop, cs.paddingRight, cs.paddingBottom, cs.paddingLeft].map(
              parseFloat
            ) as [number, number, number, number],
            border: parseFloat(cs.borderTopWidth),
            row: rect(row)!,
            rowChildren: [...row.children].map((child) => child.className),
            plus: rect(buttons.plus)!,
            plusGlyph: rect(buttons.plus.querySelector('svg'))!,
            text: rect(row.querySelector('textarea'))!,
            agent: rect(buttons.agent),
            mic: rect(row.querySelector('[data-testid="dictation-button"]'))!,
            send: rect(buttons.send)!,
            sendIsLast: row.lastElementChild === sendSlot,
          };
        };
        const crewCard = document.querySelector<HTMLElement>('.crew-compose-card')!;
        const crewRow = crewCard.querySelector<HTMLElement>('.crew-compose-row')!;
        const chatCard = document.getElementById('chat-card')!;
        // What the stylesheet says the card's rest height is, resolved in the bar that declares it.
        const probe = document.createElement('div');
        probe.style.height = 'var(--crew-compose-rest-height)';
        document.querySelector('.crew-compose')!.append(probe);
        const restHeight = probe.getBoundingClientRect().height;
        probe.remove();
        return {
          restHeight,
          crewRowChildren: crewRow.children.length,
          stackAboveCrewRow: [...crewCard.children]
            .filter((child) => child !== crewRow)
            .map((child) => [child.className, getComputedStyle(child).position]),
          chat: geometry(chatCard, document.getElementById('chat-row')!, {
            plus: chatCard.querySelector('[data-testid="composer-plus"]')!,
            agent: null,
            send: document.getElementById('chat-send')!,
          }),
          crew: geometry(crewCard, crewRow, {
            plus: crewRow.querySelector(`button[aria-label="${attach}"]`)!,
            agent: crewRow.querySelector(`button[aria-label="${agent}"]`),
            send: crewRow.querySelector(`.crew-compose-send button[aria-label="${send}"]`)!,
          }),
        };
      },
      { attach: composerCopy.attach, agent: composerCopy.askAgent, send: composerCopy.send }
    );

  /** The row's controls on one line with the text's last line, left to right, Send last. */
  function expectOneLine(name: string, g: CardGeometry, lines: number) {
    const where = `${name}, ${lines} line${lines === 1 ? '' : 's'}`;
    const textHeight = lines * 20 + 12;
    expect(g.text.height, `${where}: the text`).toBe(textHeight);
    expect(g.row.height, `${where}: the row is the text's height`).toBe(textHeight);
    near(g.text.top, g.row.top, `${where}: the text starts the row`);
    const controls = [g.plus, ...(g.agent ? [g.agent] : []), g.mic, g.send];
    const lastLine = g.text.bottom - 16;
    for (const control of controls) {
      near(middle(control), lastLine, `${where}: a control on the last line`);
      expect(control.bottom, `${where}: inside the row`).toBeLessThanOrEqual(
        g.row.bottom + TOLERANCE
      );
    }
    const ordered = [g.plus, g.text, ...(g.agent ? [g.agent] : []), g.mic, g.send];
    for (let i = 1; i < ordered.length; i++) {
      expect(ordered[i - 1].right, `${where}: order at ${i}`).toBeLessThanOrEqual(
        ordered[i].left + TOLERANCE
      );
    }
    expect(g.sendIsLast, `${where}: Send is the row's last child`).toBe(true);
    // Send on the card's inner right edge: its border and its padding.
    near(g.card.right - g.send.right, g.border + g.padding[1], `${where}: Send to the inner edge`);
    expect([g.send.width, g.send.height], `${where}: Send`).toEqual([32, 32]);
    expect([g.plus.width, g.plus.height], `${where}: +`).toEqual([32, 32]);
    expect([g.mic.width, g.mic.height], `${where}: microphone`).toEqual([28, 28]);
    expect(g.send.left - g.mic.right, `${where}: microphone beside Send`).toBe(4);
  }

  /** The two cards' boxes, each from its own edges, are the same. */
  function expectSameBox(chat: CardGeometry, crew: CardGeometry, where: string) {
    expect(crew.padding, `${where}: padding`).toEqual(chat.padding);
    expect(crew.border, `${where}: border`).toBe(chat.border);
    expect(crew.row.height, `${where}: row height`).toBe(chat.row.height);
    expect(crew.card.height, `${where}: card height`).toBe(chat.card.height);
    const from = (g: CardGeometry) => ({
      plusLeft: g.plus.left - g.card.left,
      plusTop: g.plus.top - g.card.top,
      plusGlyphLeft: g.plusGlyph.left - g.card.left,
      textLeft: g.text.left - g.card.left,
      textTop: g.text.top - g.card.top,
      sendRight: g.card.right - g.send.right,
      sendBottom: g.card.bottom - g.send.bottom,
      micToSend: g.send.left - g.mic.right,
      rowBottom: g.card.bottom - g.row.bottom,
    });
    const a = from(chat);
    const b = from(crew);
    for (const key of Object.keys(a) as Array<keyof typeof a>) {
      near(b[key], a[key], `${where}: ${key} (chat ${a[key]}, Crew ${b[key]})`);
    }
  }

  it.for([760, 440, CHAT_MIN_WIDTH])(
    'keeps Crew’s row one 32px line, and the same box as the chat composer, in a %ipx pane',
    async (width, ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(width);
      try {
        const rest = await measure(tab);
        // The cards as the panes draw them: the chat's 56px of column chrome, Crew's bar gutters.
        expect(rest.chat.card.width).toBe(width - CHAT_COLUMN_CHROME);
        expect(rest.crew.card.width).toBeLessThanOrEqual(width);
        // Nothing is stacked above Crew's row at rest that takes room (a live region is absolute).
        for (const [name, position] of rest.stackAboveCrewRow) {
          expect(position, `${name} above the row takes no room`).toBe('absolute');
        }
        // Five controls in Crew's row: `+`, the text, the agent, the microphone and Send.
        expect(rest.crewRowChildren).toBe(5);
        expect(rest.crew.rowChildren).toEqual([
          'crew-compose-slot',
          'crew-compose-field',
          'crew-compose-slot',
          'crew-compose-slot',
          'crew-compose-slot crew-compose-send',
        ]);
        expect(rest.crew.agent, 'the agent toggle').not.toBeNull();

        expectOneLine('Crew', rest.crew, 1);
        expectOneLine('chat', rest.chat, 1);
        expect(rest.crew.row.height).toBe(32);
        // The rest height the stylesheet declares is the card's, and the chat card's too.
        expect(rest.restHeight).toBe(16 + 32 + 2);
        expect(rest.crew.card.height).toBe(rest.restHeight);
        expect(rest.chat.card.height).toBe(rest.restHeight);
        expectSameBox(rest.chat, rest.crew, `at rest in a ${width}px pane`);

        await type(tab, TEXT);
        const three = await measure(tab);
        expectOneLine('Crew', three.crew, 3);
        expectOneLine('chat', three.chat, 3);
        expect(three.crew.card.height).toBe(rest.restHeight + 2 * 20);
        expectSameBox(three.chat, three.crew, `three lines in a ${width}px pane`);
      } finally {
        await tab.close();
      }
    }
  );

  it('marks the agent toggle as a toggle: named, and saying whether its pane is open', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(760);
    try {
      const toggle = tab.locator(`.crew-compose-row button[aria-label="${composerCopy.askAgent}"]`);
      expect(await toggle.getAttribute('aria-expanded')).toBe('false');
      expect(await toggle.getAttribute('class')).toContain('crew-compose-agent-toggle');
      const plus = tab.locator(`.crew-compose-row button[aria-label="${composerCopy.attach}"]`);
      expect(await plus.getAttribute('aria-haspopup')).toBe('menu');
    } finally {
      await tab.close();
    }
  });

  it('saves screenshots of both cards when asked', async (ctx) => {
    if (!browser || !SHOTS) return ctx.skip();
    mkdirSync(SHOTS, { recursive: true });
    for (const mode of ['light', 'dark'] as const) {
      const tab = await open(760, mode);
      try {
        for (const [state, text] of [
          ['empty', ''],
          ['three-lines', TEXT],
        ] as const) {
          await type(tab, text);
          await tab
            .locator('#chat-card')
            .screenshot({ path: resolve(SHOTS, `chat-card-${mode}-${state}.png`) });
          await tab
            .locator('.crew-compose-card')
            .screenshot({ path: resolve(SHOTS, `crew-card-${mode}-${state}.png`) });
          await tab
            .locator('#panes')
            .screenshot({ path: resolve(SHOTS, `side-by-side-${mode}-${state}.png`) });
        }
      } finally {
        await tab.close();
      }
    }
  });
});
