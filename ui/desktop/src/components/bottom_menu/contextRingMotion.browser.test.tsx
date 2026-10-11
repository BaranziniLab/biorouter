// @vitest-environment node
/**
 * The context ring's arc in a real style engine (V2-R3-5; spec 2.6, "a value that
 * loads asynchronously does not animate"; vision checklist 12, nothing replays on
 * a tab switch).
 *
 * jsdom runs no transitions, so `ContextWindowIndicator.test.tsx` can only pin the
 * `data-motion-still` flag. What the finding measured is a running 250ms
 * `stroke-dashoffset` transition about a second after a tab click, when the
 * chat's count and window arrived. So the real `ContextWindowIndicator` is
 * bundled with esbuild, rendered by React in Chromium, styled by `pickers.css`
 * with the motion tokens read from `main.css`, and each change is committed
 * synchronously and asked for its running animations.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const PICKERS_CSS = readFileSync(resolve(here, 'pickers.css'), 'utf8');

const TOKEN_NAMES = ['--dur-med-min', '--dur-fast-min', '--ease-out', '--icon-chip'];

/** The first declaration of each token in main.css, so a token change reaches this test. */
function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
svg { display: block; }
`;

/** The gauge inside the popover reads the threshold through ConfigContext; it never opens here. */
const stubConfig: Plugin = {
  name: 'stub-config-context',
  setup(builder) {
    builder.onResolve({ filter: /^\.\/ConfigContext$/ }, (args) => ({
      path: args.path,
      namespace: 'config-stub',
    }));
    builder.onLoad({ filter: /.*/, namespace: 'config-stub' }, () => ({
      contents: `export const useConfig = () => ({
        read: async () => null,
        upsert: async () => undefined,
      });`,
      loader: 'ts',
    }));
  },
};

const ENTRY = `
import * as React from 'react';
import { flushSync } from 'react-dom';
import { createRoot } from 'react-dom/client';
import { ContextWindowIndicator } from '../ContextWindowIndicator';

type Reading = {
  sessionId: string;
  totalTokens?: number;
  tokenLimit: number;
  isTokenLimitLoaded: boolean;
  boundModel?: string | null;
  tokenLimitModel?: string | null;
};

let set: (next: Reading) => void = () => {};
const initial = (window as unknown as { __initial: Reading }).__initial;

function App() {
  const [reading, setReading] = React.useState<Reading>(initial);
  set = setReading;
  return (
    <div style={{ padding: 16 }}>
      <ContextWindowIndicator {...reading} onCompact={() => {}} showRemainingPercent />
    </div>
  );
}

// Committed synchronously, so the caller reads the style of exactly that commit.
(window as unknown as { __set: (next: Reading) => void }).__set = (next) =>
  flushSync(() => set(next));

createRoot(document.getElementById('root')!).render(<App />);
`;

type Reading = {
  sessionId: string;
  totalTokens?: number;
  tokenLimit: number;
  isTokenLimitLoaded: boolean;
  boundModel?: string | null;
  tokenLimitModel?: string | null;
};

/** What the arc does right after a commit: its running transitions and its dashoffset. */
type ArcMotion = {
  running: { property: string; duration: number }[];
  offset: number;
  still: boolean;
};

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

const RADIUS = 6;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;
/** The dashoffset the arc must sit at, with no transition between, for `used` of `limit`. */
const offsetFor = (used: number, limit: number) => CIRCUMFERENCE * (used / limit);

describe('the context ring sweeps only for a live update in the same chat (V2-R3-5)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the ring tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubConfig],
      loader: { '.css': 'empty' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + readTokens() + PICKERS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /**
   * The composer as it mounts on an open or a tab switch. A reading whose window
   * has not loaded draws no ring (V2-R4-1), so then there is no arc to wait for.
   */
  async function open(initial: Reading): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 400, height: 200 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.evaluate((reading) => {
      (window as unknown as { __initial: unknown }).__initial = reading;
    }, initial);
    await tab.addScriptTag({ content: script });
    if (initial.isTokenLimitLoaded && initial.boundModel !== null) {
      await tab.waitForSelector('[data-testid="context-ring-arc"]');
    } else {
      await tab.waitForFunction(() => document.getElementById('root')!.childElementCount > 0);
    }
    await rest(tab);
    return tab;
  }

  /** Two frames, so a still flag has been painted and released. */
  async function rest(tab: Page) {
    await tab.evaluate(
      () =>
        new Promise((done) =>
          requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, 20)))
        )
    );
  }

  /** Commits `next` and reads the arc in that commit's style. */
  async function commit(tab: Page, next: Reading): Promise<ArcMotion> {
    return tab.evaluate((reading) => {
      (window as unknown as { __set: (r: unknown) => void }).__set(reading);
      const arc = document.querySelector<SVGCircleElement>('[data-testid="context-ring-arc"]')!;
      // getAnimations() flushes style, which is where a transition starts or does not.
      const running = arc.getAnimations().map((animation) => ({
        // A CSSTransition names the property it moves.
        property:
          (animation as unknown as { transitionProperty?: string }).transitionProperty ?? '',
        duration: Number(animation.effect?.getComputedTiming().duration ?? 0),
      }));
      return {
        running,
        offset: parseFloat(getComputedStyle(arc).strokeDashoffset),
        still: arc.hasAttribute('data-motion-still'),
      };
    }, next);
  }

  /** Commits `next` and says whether a ring is drawn, and the line's text. */
  async function commitBare(tab: Page, next: Reading): Promise<{ arc: boolean; text: string }> {
    return tab.evaluate((reading) => {
      (window as unknown as { __set: (r: unknown) => void }).__set(reading);
      return {
        arc: document.querySelector('[data-testid="context-ring-arc"]') !== null,
        text: document.getElementById('root')!.textContent ?? '',
      };
    }, next);
  }

  const near = (actual: number, expected: number, what: string) =>
    expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThan(0.01);

  it('lands the count that arrives after an open at rest, at its final arc', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ sessionId: 'chat-a', tokenLimit: 128_000, isTokenLimitLoaded: true });
    try {
      const m = await commit(tab, {
        sessionId: 'chat-a',
        totalTokens: 92_000,
        tokenLimit: 128_000,
        isTokenLimitLoaded: true,
      });
      expect(m.still).toBe(true);
      expect(m.running).toEqual([]);
      near(m.offset, offsetFor(92_000, 128_000), 'arc');
    } finally {
      await tab.close();
    }
  });

  it('draws nothing against the fallback, then lands the window at rest', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({
      sessionId: 'chat-a',
      totalTokens: 92_000,
      tokenLimit: 128_000,
      isTokenLimitLoaded: false,
    });
    try {
      // V2-R4-1: no ring, no figure while the window loads (92k of the 128k
      // fallback would read "28% left").
      expect(
        await tab.evaluate(() => ({
          arc: document.querySelector('[data-testid="context-ring-arc"]') !== null,
          text: document.getElementById('root')!.textContent,
        }))
      ).toEqual({ arc: false, text: '' });
      const m = await commit(tab, {
        sessionId: 'chat-a',
        totalTokens: 92_000,
        tokenLimit: 400_000,
        isTokenLimitLoaded: true,
      });
      expect(m.running).toEqual([]);
      near(m.offset, offsetFor(92_000, 400_000), 'arc');
    } finally {
      await tab.close();
    }
  });

  it('lands another chat’s reading at rest under the same composer', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({
      sessionId: 'chat-a',
      totalTokens: 10_000,
      tokenLimit: 128_000,
      isTokenLimitLoaded: true,
    });
    try {
      const m = await commit(tab, {
        sessionId: 'chat-b',
        totalTokens: 92_000,
        tokenLimit: 128_000,
        isTokenLimitLoaded: true,
      });
      expect(m.running).toEqual([]);
      near(m.offset, offsetFor(92_000, 128_000), 'arc');
    } finally {
      await tab.close();
    }
  });

  // V2-R4-1, round 5: a chat opened for the first time loads for seconds, and
  // until its row lands the composer has looked up the window of the model for
  // NEW chats. Measured on a 200k chat: "0 of 1.1M", a 79ms sweep to "64.6k of
  // 1.1M", then "of 200k". Now: nothing until the chat's own window, then one
  // reading at rest.
  it('draws nothing while a chat loads, then its own model’s reading at rest', async (ctx) => {
    if (!browser) return ctx.skip();
    const loading: Reading = {
      sessionId: 'demo2',
      totalTokens: 0,
      tokenLimit: 1_050_000,
      isTokenLimitLoaded: true,
      tokenLimitModel: 'versa_azure/gpt-5.5',
      boundModel: null,
    };
    const tab = await open(loading);
    try {
      expect(
        await tab.evaluate(() => ({
          arc: document.querySelector('[data-testid="context-ring-arc"]') !== null,
          text: document.getElementById('root')!.textContent,
        }))
      ).toEqual({ arc: false, text: '' });
      // The row lands with its count and its binding; the window is still the
      // new-chat model's, then its re-read starts.
      const landed = { ...loading, totalTokens: 64_600, boundModel: 'anthropic/claude-opus-4-5' };
      expect(await commitBare(tab, landed)).toEqual({ arc: false, text: '' });
      await rest(tab);
      expect(await commitBare(tab, { ...landed, isTokenLimitLoaded: false })).toEqual({
        arc: false,
        text: '',
      });
      await rest(tab);
      const m = await commit(tab, {
        ...landed,
        tokenLimit: 200_000,
        tokenLimitModel: 'anthropic/claude-opus-4-5',
      });
      expect(m.still).toBe(true);
      expect(m.running).toEqual([]);
      near(m.offset, offsetFor(64_600, 200_000), 'arc');
      // And nothing starts once the still flag is released.
      await rest(tab);
      expect(
        await tab.evaluate(
          () =>
            document
              .querySelector<SVGCircleElement>('[data-testid="context-ring-arc"]')!
              .getAnimations().length
        )
      ).toBe(0);
    } finally {
      await tab.close();
    }
  });

  it('sweeps over 250ms for a live update in the same chat, also right after an arrival', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open({ sessionId: 'chat-a', tokenLimit: 128_000, isTokenLimitLoaded: true });
    try {
      // The chat's reading arrives (at rest), then a turn in that chat moves it.
      await commit(tab, {
        sessionId: 'chat-a',
        totalTokens: 10_000,
        tokenLimit: 128_000,
        isTokenLimitLoaded: true,
      });
      await rest(tab);
      const m = await commit(tab, {
        sessionId: 'chat-a',
        totalTokens: 30_000,
        tokenLimit: 128_000,
        isTokenLimitLoaded: true,
      });
      expect(m.still).toBe(false);
      expect(m.running).toEqual([{ property: 'stroke-dashoffset', duration: 250 }]);
    } finally {
      await tab.close();
    }
  });
});
