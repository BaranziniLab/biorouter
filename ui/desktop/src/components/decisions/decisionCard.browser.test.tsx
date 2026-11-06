// @vitest-environment node
/**
 * The decision card's bars in a real layout engine. jsdom lays nothing out, so
 * the grid that keeps every bar on one track, the fill that is the probability
 * and the threshold mark that sits at the threshold can only be measured here.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const CARD_CSS = readFileSync(resolve(here, 'decisionsChat.css'), 'utf8');

const TOKEN_NAMES = [
  '--radius-container',
  '--radius-element',
  '--radius-full',
  '--text-title',
  '--text-title--line-height',
  '--text-title--letter-spacing',
  '--text-secondary',
  '--text-secondary--line-height',
  '--text-supporting',
  '--text-supporting--line-height',
  '--dur-med-min',
  '--ease-out',
];

function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

/** The progress primitive's own rules: its two base rules and its primitives block. */
function progressRules(): string {
  const base = ['.br-progress {', '.br-progress__fill {'].map((head) => {
    const start = MAIN_CSS.indexOf(`\n${head}`);
    if (start < 0) throw new Error(`main.css has no ${head} rule`);
    return MAIN_CSS.slice(start, MAIN_CSS.indexOf('}', start) + 1);
  });
  const block =
    /\/\* @ws WS-PRIMITIVES begin: progress \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: progress \*\//.exec(
      MAIN_CSS
    );
  if (!block) throw new Error('main.css has no progress primitives block');
  return `${base.join('\n')}\n${block[1]}`;
}

const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
p, ul { margin: 0; }
svg { display: block; width: 14px; height: 14px; }
:root { --background-fill: #f4f4f2; --background-fill-strong: #eaeae6; --background-default: #fff;
  --background-muted: #f2f2f0; --text-default: #111; --text-muted: #666; --text-success: #05662d;
  --text-danger: #a51a14; --text-accent: #b85a32; }
.bg-background-accent { background-color: #b85a32; }
.br-fill-surface > * { --br-track-ground: var(--background-fill-strong); }
#root { width: 520px; padding: 16px; }
`;

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { DecisionCardView } from './DecisionCard';
import { parseDecisionRecord } from '../../utils/sessionDecisions';
import { wireBinaryDetail, wireChoiceDetail } from './decisionFixtures';

const choice = wireChoiceDetail().record;
// A long option name, to hold the label column to its cap.
choice.questions.course.kind.choice.options = {
  rrms: null,
  'secondary progressive, with superimposed relapses since onset': null,
  ppms: null,
};
choice.decisions[0].probabilities = {
  rrms: 0.2,
  'secondary progressive, with superimposed relapses since onset': 0.5,
  ppms: 0.3,
};
choice.decisions[0].answer = {
  type: 'choice',
  option: 'secondary progressive, with superimposed relapses since onset',
};

function App() {
  return (
    <div>
      <div id="binary">
        <DecisionCardView record={parseDecisionRecord(wireBinaryDetail().record)} sessionId="chat-1" onOpenArtifact={() => {}} />
      </div>
      <div id="choice" style={{ marginTop: 16 }}>
        <DecisionCardView record={parseDecisionRecord(choice)} sessionId="chat-1" onOpenArtifact={() => {}} />
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

const EPSILON = 1;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

type Bar = {
  key: string;
  label: { left: number; right: number };
  track: { left: number; right: number; width: number; top: number; bottom: number };
  fill: number;
  value: { left: number; right: number };
  mark: { center: number; top: number; bottom: number } | null;
};

describe('the decision card’s bars in a real layout engine', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the card tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      loader: { '.css': 'empty' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + readTokens() + progressRules() + CARD_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 800, height: 900 } });
    await tab.emulateMedia({ reducedMotion: 'reduce' });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#choice [data-testid="decision-card"]');
    await tab.waitForTimeout(100);
    return tab;
  }

  const measure = (tab: Page, root: string) =>
    tab.evaluate((selector) => {
      const box = (element: Element) => element.getBoundingClientRect();
      const card = box(document.querySelector(`${selector} [data-testid="decision-card"]`)!);
      const bars = Array.from(
        document.querySelectorAll(`${selector} [data-testid="decision-bar"]`)
      ).map((row) => {
        const progress = box(row.querySelector('.br-progress')!);
        const mark = row.querySelector('[data-testid="decision-threshold-mark"]');
        const markBox = mark ? box(mark) : null;
        return {
          key: (row as HTMLElement).dataset.key!,
          label: box(row.querySelector('.br-decision-bars__label')!),
          track: {
            left: progress.left,
            right: progress.right,
            width: progress.width,
            top: progress.top,
            bottom: progress.bottom,
          },
          fill: box(row.querySelector('.br-progress__fill')!).width,
          value: box(row.querySelector('.br-decision-bars__value')!),
          mark: markBox
            ? {
                center: markBox.left + markBox.width / 2,
                top: markBox.top,
                bottom: markBox.bottom,
              }
            : null,
          labelOverflows:
            (row.querySelector('.br-decision-bars__label') as HTMLElement).scrollWidth >
            (row.querySelector('.br-decision-bars__label') as HTMLElement).clientWidth,
        };
      });
      return { card: { left: card.left, width: card.width }, bars };
    }, root) as Promise<{
      card: { left: number; width: number };
      bars: Array<Bar & { labelOverflows: boolean }>;
    }>;

  it('puts every bar on one track, its fill the probability, the threshold at its value', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const { bars } = await measure(tab, '#binary');
      expect(bars.map((bar) => bar.key)).toEqual(['yes', 'no']);
      const [yes, no] = bars;
      // One grid: both tracks start and end at the same x, and are visible.
      near(yes.track.left, no.track.left, 'track left');
      near(yes.track.right, no.track.right, 'track right');
      expect(yes.track.width).toBeGreaterThan(150);
      near(yes.track.bottom - yes.track.top, 8, 'track height');
      // The fill IS the probability.
      near(yes.fill, 0.93 * yes.track.width, 'P(yes) fill');
      near(no.fill, 0.07 * no.track.width, 'P(no) fill');
      // The threshold mark sits at 0.80 of the yes track, and stands proud of it.
      expect(yes.mark).not.toBeNull();
      near(yes.mark!.center, yes.track.left + 0.8 * yes.track.width, 'threshold mark');
      expect(yes.mark!.top).toBeLessThan(yes.track.top);
      expect(yes.mark!.bottom).toBeGreaterThan(yes.track.bottom);
      expect(no.mark).toBeNull();
      // Values right-aligned in one column, clear of the track.
      near(yes.value.right, no.value.right, 'value right edge');
      expect(yes.value.left).toBeGreaterThanOrEqual(yes.track.right);
    } finally {
      await tab.close();
    }
  });

  it('caps a long option’s label and keeps the tracks aligned under it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const { card, bars } = await measure(tab, '#choice');
      expect(bars.map((bar) => bar.key)).toEqual([
        'rrms',
        'secondary progressive, with superimposed relapses since onset',
        'ppms',
      ]);
      const long = bars[1];
      // The label column is held to 40% of the card's content box, and the long
      // label is cut with an ellipsis rather than pushing its bar aside.
      expect(long.label.right - long.label.left).toBeLessThanOrEqual(0.4 * (card.width - 32) + 1);
      expect(long.labelOverflows).toBe(true);
      for (const bar of bars) {
        near(bar.track.left, bars[0].track.left, `${bar.key} track left`);
        near(bar.track.right, bars[0].track.right, `${bar.key} track right`);
        near(bar.value.right, bars[0].value.right, `${bar.key} value right`);
      }
      near(long.fill, 0.5 * long.track.width, 'answer fill');
      // A choice threshold is drawn on the option it names, rrms at 0.60.
      expect(bars[0].mark).not.toBeNull();
      near(bars[0].mark!.center, bars[0].track.left + 0.6 * bars[0].track.width, 'rrms mark');
      expect(card.width).toBeLessThanOrEqual(560);
    } finally {
      await tab.close();
    }
  });
});
