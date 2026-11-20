// @vitest-environment node
/**
 * The microphone under the whole stylesheet, in a real layout engine (DD-60, "Phases" in
 * `docs/dictation/surfaces.md`).
 *
 * jsdom lays nothing out and runs no Tailwind, so the component tests can say which phase the
 * microphone is in but not how big it is. Here the real `DictationButton` is bundled with
 * esbuild into the composer card's ONE row as `ChatInput` draws it (owner, 2026-11-13: `+`, the
 * text, the microphone and Send, each control in its 32px slot; the card's own class list, the
 * real `Button`), with the model chip on the footer line under the card, `styles/main.css` is
 * compiled by Tailwind with the classes the bundle uses, `pickers.css` and `composer.css` are
 * appended as written, and Chromium lays it out in panes of 760, 440 and 360px:
 *
 *  - at rest the microphone is the 28px ghost round button, immediately left of Send on the
 *    text's centre line, and Send still ends the row on the card's inner right edge;
 *  - recording, it is a 28px pill with five level bars and the `m:ss` timer, inside the card, on
 *    the same line, the text giving way to it, with the timer dropped in a pane narrower than
 *    440px;
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

/**
 * The dictionary's first-install choice reads the config and the dictionary settings from a
 * daemon this page does not have; here it is the state a fresh account shows, the box ticked.
 */
const activationChoice: Plugin = {
  name: 'activation-choice',
  setup(builder) {
    builder.onResolve({ filter: /\/activationChoice$/ }, () => ({
      path: 'activation-choice',
      namespace: 'activation-stub',
    }));
    builder.onLoad({ filter: /.*/, namespace: 'activation-stub' }, () => ({
      contents: `export function useDictionaryActivationChoice() {
        return {
          ready: true, checked: true, disabled: false, showChoice: true, needsSave: false,
          help: 'Terms from chats are suggested.', setChecked() {}, startInstall() {},
          activationFailed: false, savedChoiceDiffered: false, saving: false,
          retryActivation() {},
        };
      }`,
      loader: 'js',
    }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { composerCardClassName } from '../composer/composerCard';
import { Button } from '../ui/button';
import { ArrowUp, Plus } from '../icons/app-icons';
import { DictationButton } from './DictationButton';
import { setDictationLevelForTests, setDictationSnapshotForTests } from './dictationSession';
import { refreshDictationStatus, resetDictationStoreForTests } from './dictationStore';

// The status store reads this page's answer; set before the first render, because a reset
// drops the store's subscribers.
let currentStatus = null;
resetDictationStoreForTests({
  status: async () =>
    currentStatus
      ? { ok: true, status: 200, data: currentStatus }
      : { ok: false, status: 500, error: { code: 'http500', message: '' }, retryAfterS: null },
});

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
    <div className="relative flex w-full flex-col gap-1">
      <div id={id} className={composerCardClassName({ isDraggingOver: false })}>
        <div className="br-composer-row">
          <span className="br-composer-row__slot">
            <Button id={id + '-plus'} type="button" variant="ghost" size="default" shape="round" aria-label="Add files and tools" className="br-composer-plus">
              <Plus aria-hidden className="br-composer-plus__glyph" />
            </Button>
          </span>
          <form className="br-composer-row__field">
            <textarea id={id + '-text'} rows={1} aria-label="Message" className="br-composer-input block w-full resize-none border-none bg-transparent px-0 py-1.5 text-body" />
          </form>
          <span className="br-composer-row__slot">
            <DictationButton control={control(id)} setView={() => {}} />
          </span>
          <span className="br-composer-row__slot">
            <span className="flex-shrink-0">
              <Button id={id + '-send'} type="button" size="default" shape="round" variant="secondary" aria-label="Send" className="br-composer-send">
                <span className="br-composer-send__glyph"><ArrowUp className="size-4" /></span>
              </Button>
            </span>
          </span>
        </div>
      </div>
      <div className="br-footline br-composer-footer">
        <div className="br-footline__side" />
        <div className="br-footline__side" data-side="end">
          <div className="br-footline__model">
            <button type="button" className="br-picker-chip" data-kind="model">
              <span className="br-picker-chip__name">gpt-5.6-sol-with-a-long-model-name · Deep</span>
            </button>
          </div>
        </div>
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

window.dictation = {
  setSnapshot: setDictationSnapshotForTests,
  setLevel: setDictationLevelForTests,
  setStatus: (status) => {
    currentStatus = status;
    return refreshDictationStatus();
  },
};
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
      plugins: [workletUrl, activationChoice],
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
      // In the order `ComposerPlusMenu` imports them: the composer's rules re-cut the footer's.
      readFileSync(resolve(here, '../bottom_menu/pickers.css'), 'utf8') +
      readFileSync(resolve(here, '../composer/composer.css'), 'utf8');
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
        row: rect('#card .br-composer-row')!,
        plus: rect('#card-plus')!,
        text: rect('#card-text')!,
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
        const middle = (r: { top: number; height: number }) => r.top + r.height / 2;
        const rest = await geometry(tab);
        expect(rest.card.width).toBe(width - CHAT_COLUMN_CHROME);
        expect(rest.row.height).toBe(32);
        expect([rest.mic.width, rest.mic.height]).toEqual([28, 28]);
        expect(rest.mic.right).toBeLessThanOrEqual(rest.send.left);
        expect(rest.send.left - rest.mic.right).toBe(4);
        // On the text's line: one centre for the text, the microphone and Send.
        expect(middle(rest.mic)).toBe(middle(rest.text));
        expect(middle(rest.send)).toBe(middle(rest.text));
        // The text runs up to the microphone (the row's 4px gap), never under it.
        expect(rest.mic.left - rest.text.right).toBe(4);
        expect(Math.round(rest.innerRight - rest.send.right)).toBe(0);
        // The model is no longer in the row: it sits on the footer line under the card.
        expect(rest.model.top).toBeGreaterThanOrEqual(rest.card.bottom);
        expect(rest.name.width, 'the model name keeps room for its ellipsis').toBeGreaterThan(12);

        await record(tab);
        const pill = await geometry(tab);
        expect(pill.mic.height).toBe(28);
        expect(pill.bars).toBe(5);
        expect(pill.mic.left).toBeGreaterThanOrEqual(pill.card.left);
        expect(pill.send.right).toBeLessThanOrEqual(pill.innerRight + 0.5);
        expect(pill.mic.right).toBeLessThanOrEqual(pill.send.left);
        expect(pill.send.left - pill.mic.right).toBe(4);
        // The pill keeps the row's line and height, and the text gives way to it.
        expect(pill.row.height).toBe(32);
        expect(middle(pill.mic)).toBe(middle(pill.text));
        expect(pill.mic.left - pill.text.right).toBe(4);
        expect(pill.plus.right).toBeLessThanOrEqual(pill.text.left);
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

  /** A fresh account's status: no model yet, the recommended turbo offered, maybe installing. */
  const freshStatus = (job: { done: number } | null) => {
    const allowed = { allowed: true, reason: null };
    const bytes = 574_041_195;
    return {
      engine: {
        compiled: true,
        state: 'ready',
        backend: 'metal',
        device: 'Apple M4 Max',
        threads: 8,
        missing_cpu_features: [],
        rss_bytes: null,
      },
      models: [
        {
          id: 'whisper-large-v3-turbo-q5_0',
          display_name: 'Whisper large-v3-turbo (q5_0)',
          installed: false,
          bytes,
          location: null,
          superseded: false,
          suitability: 'recommended',
          recommended: true,
        },
      ],
      active_model: null,
      language: 'en',
      install: allowed,
      install_job: job
        ? {
            model: 'whisper-large-v3-turbo-q5_0',
            bytes_done: job.done,
            bytes_total: bytes,
            bytes_per_second: 400_000,
            phase: 'downloading',
            error: null,
          }
        : null,
      policy: {
        managed: false,
        enabled: allowed,
        capture: allowed,
        install: allowed,
        external_engine: allowed,
        dictionary: allowed,
        agent_writes: allowed,
        allowed_models: null,
      },
      limits: { max_seconds: 600, max_chunk_bytes: 524_288, max_streams: 4 },
      queue: { streams: 0, segments: 0 },
      account: 'wgu',
      host: 'this-mac',
      models_dir: '/Users/wgu/.local/share/biorouter/dictation/models',
    };
  };

  /**
   * As `open`, on a secure origin: `setContent` leaves the page on an opaque origin, which is not
   * a secure context (so `getUserMedia` is absent and every status reads MSG-INSECURE) and has no
   * `localStorage`. The page is served by a route, so nothing reaches the network.
   */
  async function openSecure(width: number): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 600 } });
    await tab.route('https://dictation.test/**', (route) =>
      route.fulfill({
        contentType: 'text/html',
        body:
          `<!doctype html><html><head><style>${css}</style></head>` +
          `<body style="margin:0;padding-top:320px"><div id="root"></div></body></html>`,
      })
    );
    await tab.goto('https://dictation.test/');
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('[data-testid="dictation-button"]');
    await tab.evaluate((w) => {
      document.getElementById('pane')!.style.width = `${w}px`;
    }, width);
    return tab;
  }

  const setStatus = (tab: Page, status: unknown, phase: string) =>
    tab
      .evaluate(
        (s) =>
          (
            window as unknown as { dictation: { setStatus(s: unknown): Promise<unknown> } }
          ).dictation.setStatus(s),
        status
      )
      .then(() =>
        tab.waitForSelector(`[data-testid="dictation-button"][data-dictation-phase="${phase}"]`)
      );

  it('draws the install ring round the microphone, both centred in the 28px button', async (ctx) => {
    // The owner's report (2026-11-19): while the speech model installed, the glyph sat squeezed
    // into the button's top left corner. A descendant `svg` rule for the ring also caught the
    // microphone, pinning it to the corner and turning it a quarter, and the button's own 16px
    // `svg` size shrank the ring into the same corner.
    if (!browser) return ctx.skip();
    const tab = await openSecure(760);
    try {
      await setStatus(tab, freshStatus({ done: 229_616_478 }), 'installing');
      const measured = await tab.evaluate(() => {
        const box = (el: Element | null) => {
          const r = el!.getBoundingClientRect();
          return { left: r.left, top: r.top, width: r.width, height: r.height };
        };
        const button = document.querySelector('[data-testid="dictation-button"]')!;
        // The ring is the first `svg` in its span, the microphone the second.
        const ring = button.querySelector('.br-dictation-ring > svg:first-child');
        const mic = button.querySelector('svg.lucide-mic');
        return {
          button: box(button),
          ring: box(ring),
          mic: box(mic),
          micTransform: getComputedStyle(mic!).transform,
          dash: button.querySelector('.br-dictation-ring__value')!.getAttribute('stroke-dasharray'),
        };
      });
      const centre = (r: { left: number; top: number; width: number; height: number }) => [
        r.left + r.width / 2,
        r.top + r.height / 2,
      ];
      expect([measured.button.width, measured.button.height]).toEqual([28, 28]);
      // The ring fills the button, edge to edge, rather than a 16px circle in its corner.
      expect(measured.ring).toEqual(measured.button);
      // The microphone keeps its 16px glyph, upright, on the button's centre.
      expect([measured.mic.width, measured.mic.height]).toEqual([16, 16]);
      expect(measured.micTransform).toBe('none');
      expect(centre(measured.mic)).toEqual(centre(measured.button));
      expect(measured.dash).toBe('40 100');
    } finally {
      await tab.close();
    }
  });

  it('opens the setup popover with its choice on one line, the box on the text column', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openSecure(760);
    try {
      await setStatus(tab, freshStatus(null), 'not-installed');
      await tab.click('[data-testid="dictation-button"]');
      await tab.waitForSelector('[data-testid="dictation-setup-suggest"]');
      // Measure the popover at rest, after its open motion (a scale from 0.97).
      await tab.evaluate(() => Promise.all(document.getAnimations().map((a) => a.finished)));
      const measured = await tab.evaluate(() => {
        const box = (selector: string) => {
          const r = document.querySelector(selector)!.getBoundingClientRect();
          return { left: r.left, right: r.right, top: r.top, height: r.height };
        };
        return {
          title: box('.br-dictation-setup__title'),
          square: box('[data-testid="dictation-setup-suggest"] .br-checkbox-box'),
          words: box('.br-dictation-setup__choice-label > span:not(.br-checkbox)'),
          tip: box('[data-testid="dictation-setup-suggest"] .br-info-tip svg'),
          wordsFont: getComputedStyle(document.querySelector('.br-dictation-setup__choice-label')!)
            .fontSize,
          textFont: getComputedStyle(document.querySelector('.br-dictation-setup__text')!).fontSize,
          focused: document.activeElement?.className ?? '',
        };
      });
      // One line, in the popover's own size.
      expect(measured.wordsFont).toBe(measured.textFont);
      expect(measured.words.height).toBeLessThanOrEqual(16);
      // The square on the text column, the words 8px after it, the InfoTip's glyph 4px after them.
      expect(measured.square.left).toBe(measured.title.left);
      expect(measured.words.left - measured.square.right).toBe(8);
      expect(Math.round(measured.tip.left - measured.words.right)).toBe(4);
      // A pointer open rests focus on the popover box, which draws no ring.
      expect(measured.focused).toContain('br-dictation-setup-popover');
      expect(await tab.evaluate(() => getComputedStyle(document.activeElement!).outlineStyle)).toBe(
        'none'
      );
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
