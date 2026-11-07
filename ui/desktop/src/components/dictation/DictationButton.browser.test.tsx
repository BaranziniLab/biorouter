// @vitest-environment node
/**
 * The microphone under the whole stylesheet, in a real layout engine (DD-60, "Phases" in
 * `docs/dictation/surfaces.md`).
 *
 * jsdom lays nothing out and runs no Tailwind, so the component tests can say which phase the
 * microphone is in but not how big it is. Here the real `DictationButton` is bundled with
 * esbuild into the composer card's controls row (the card's own class list, the real `Button`),
 * `styles/main.css` is compiled by Tailwind with the classes the bundle uses, `composer.css` is
 * appended as written, and Chromium lays it out in panes of 760, 440 and 360px:
 *
 *  - at rest the microphone is the 28px ghost round button, immediately left of Send, and Send
 *    still ends the row on the card's inner right edge;
 *  - recording, it is a 28px pill with five level bars and the `m:ss` timer, inside the card,
 *    with the timer dropped in a pane narrower than 440px;
 *  - the bars follow the level through one custom property, and stand still under
 *    `prefers-reduced-motion`.
 */
import { readFileSync } from 'node:fs';
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

/** Vite's `?url&no-inline` import has no meaning to esbuild; the worklet is not loaded here. */
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
import { composerCardClassName } from '../composer/composerCard';
import { Button } from '../ui/button';
import { ArrowUp } from '../icons/app-icons';
import { DictationButton } from './DictationButton';
import { setDictationLevelForTests, setDictationSnapshotForTests } from './dictationSession';

const control = (key) => ({
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

function Card({ id }) {
  return (
    <div id={id} className={composerCardClassName({ isDraggingOver: false })}>
      <textarea rows={1} aria-label="Message" className="br-composer-input block w-full resize-none border-none bg-transparent px-0 py-1.5" />
      <div className="br-composer-controls">
        <span className="br-composer-controls__spacer" />
        <div className="br-composer-controls__model">
          <button type="button" className="br-picker-chip" data-kind="model">
            <span className="br-picker-chip__name">gpt-5.6-sol-with-a-long-model-name · Deep</span>
          </button>
        </div>
        <DictationButton control={control(id)} setView={() => {}} />
        <span className="flex-shrink-0">
          <Button id={id + '-send'} type="button" size="default" shape="round" variant="secondary" aria-label="Send">
            <ArrowUp className="size-4" />
          </Button>
        </span>
      </div>
    </div>
  );
}

function App() {
  return (
    <div id="pane" className="bg-background-default" style={{ width: 760, paddingInline: ${CHAT_COLUMN_CHROME / 2} }}>
      <Card id="card" />
    </div>
  );
}

window.dictation = { setSnapshot: setDictationSnapshotForTests, setLevel: setDictationLevelForTests };
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

describe('the microphone in a real layout engine', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the microphone geometry tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [workletUrl],
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
      readFileSync(resolve(here, '../composer/composer.css'), 'utf8') +
      readFileSync(resolve(here, '../bottom_menu/pickers.css'), 'utf8');
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(width: number, reducedMotion = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 400 } });
    if (reducedMotion) await tab.emulateMedia({ reducedMotion: 'reduce' });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head>` +
        `<body style="margin:0"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="dictation-button"]');
    await tab.evaluate((w) => {
      document.getElementById('pane')!.style.width = `${w}px`;
    }, width);
    return tab;
  }

  const geometry = (tab: Page) =>
    tab.evaluate(() => {
      const rect = (selector: string) => {
        const el = document.querySelector(selector);
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
      const card = document.getElementById('card')!;
      const cs = getComputedStyle(card);
      return {
        card: rect('#card')!,
        innerRight:
          card.getBoundingClientRect().right -
          parseFloat(cs.borderRightWidth) -
          parseFloat(cs.paddingRight),
        mic: rect('[data-testid="dictation-button"]')!,
        send: rect('#card-send')!,
        model: rect('[data-kind="model"]')!,
        name: rect('[data-kind="model"] .br-picker-chip__name')!,
        timer: rect('[data-testid="dictation-timer"]'),
        bars: [...document.querySelectorAll('.br-dictation-bar')].length,
      };
    });

  const record = (tab: Page) =>
    tab.evaluate(async () => {
      (window as unknown as { dictation: { setSnapshot(s: unknown): void } }).dictation.setSnapshot(
        {
          key: 'card',
          activity: 'recording',
          elapsedS: 7,
        }
      );
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
    });

  it.for([760, 440, CHAT_MIN_WIDTH])(
    'rests as the 28px round button left of Send, and records as a 28px pill in a %ipx pane',
    async (width, ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(width);
      try {
        const rest = await geometry(tab);
        expect(rest.card.width).toBe(width - CHAT_COLUMN_CHROME);
        expect([rest.mic.width, rest.mic.height]).toEqual([28, 28]);
        expect(rest.mic.right).toBeLessThanOrEqual(rest.send.left);
        expect(rest.send.left - rest.mic.right).toBe(4);
        expect(rest.model.right).toBeLessThanOrEqual(rest.mic.left + 0.5);
        expect(Math.round(rest.innerRight - rest.send.right)).toBe(0);
        expect(rest.name.width, 'the model name keeps room for its ellipsis').toBeGreaterThan(12);

        await record(tab);
        const pill = await geometry(tab);
        expect(pill.mic.height).toBe(28);
        expect(pill.bars).toBe(5);
        expect(pill.mic.left).toBeGreaterThanOrEqual(pill.card.left);
        expect(pill.send.right).toBeLessThanOrEqual(pill.innerRight + 0.5);
        expect(pill.mic.right).toBeLessThanOrEqual(pill.send.left);
        if (width < 440) {
          expect(pill.timer, 'the timer is dropped under a 440px pane').toBeNull();
        } else {
          expect(pill.timer).not.toBeNull();
          expect(await tab.textContent('[data-testid="dictation-timer"]')).toBe('0:07');
          expect(pill.timer!.right).toBeLessThanOrEqual(pill.mic.right);
        }
        expect(await tab.getAttribute('[data-testid="dictation-button"]', 'aria-label')).toBe(
          'Stop dictation, 0:07'
        );
      } finally {
        await tab.close();
      }
    }
  );

  const barScales = (tab: Page) =>
    tab.evaluate(() =>
      [...document.querySelectorAll('.br-dictation-bar')].map(
        (bar) => new DOMMatrix(getComputedStyle(bar).transform).d
      )
    );

  it('moves the bars with the level, without a render', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(760);
    try {
      await record(tab);
      const quiet = await barScales(tab);
      await tab.evaluate(async () => {
        (window as unknown as { dictation: { setLevel(l: number): void } }).dictation.setLevel(0.3);
        await new Promise((r) => setTimeout(r, 400));
      });
      const loud = await barScales(tab);
      expect(loud[2]).toBeGreaterThan(quiet[2] + 0.2);
    } finally {
      await tab.close();
    }
  });

  it('holds the bars still under reduced motion', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(760, true);
    try {
      await record(tab);
      const before = await barScales(tab);
      await tab.evaluate(async () => {
        (window as unknown as { dictation: { setLevel(l: number): void } }).dictation.setLevel(0.3);
        await new Promise((r) => setTimeout(r, 400));
      });
      expect(await barScales(tab)).toEqual(before);
      // No transition left to see (main.css's own reduced motion rule makes it 0.01 ms).
      const duration = await tab.evaluate(() =>
        parseFloat(
          getComputedStyle(document.querySelector('.br-dictation-bar')!).transitionDuration
        )
      );
      expect(duration).toBeLessThanOrEqual(0.00001);
    } finally {
      await tab.close();
    }
  });
});
