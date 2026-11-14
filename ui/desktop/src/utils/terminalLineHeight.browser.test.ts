// @vitest-environment node
/**
 * The terminal's rows are 20px for 12px type (design.md 9.15), measured on a real xterm in a
 * real layout engine.
 *
 * Phase C, F-12: xterm multiplies its `lineHeight` by the cell height it measures (about 15px
 * for 12px monospace), so `20 / 12` drew 25px rows in the terminal dock in every theme. jsdom
 * lays nothing out, so this runs the real `terminalLineHeight.ts` (transpiled and run as a
 * classic script, as `findClearance.browser.test.ts` does) against the real xterm build in
 * Chromium, and measures the pitch between two drawn rows.
 *
 * Control: the old multiplier, `20 / 12`, measures 25px here, which is the finding.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, type Browser } from 'playwright';
import ts from 'typescript';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const XTERM_JS = readFileSync(require.resolve('@xterm/xterm/lib/xterm.js'), 'utf8');
const XTERM_CSS = readFileSync(require.resolve('@xterm/xterm/css/xterm.css'), 'utf8');

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

/** `terminalLineHeight.ts` as a classic script that sets `terminalLineHeight`. */
const helperScript = (): string => {
  const source = readFileSync(resolve(here, 'terminalLineHeight.ts'), 'utf-8');
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
  });
  return `(() => { const exports = {}; ${outputText}; window.terminalLineHeight = exports.terminalLineHeight; })();`;
};

const FAMILY = 'Menlo, Consolas, "Liberation Mono", monospace';

describe('terminalLineHeight', () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
  });

  afterAll(async () => {
    await browser?.close();
  });

  /** The pitch between the first two drawn rows of a terminal with `lineHeight` of `kind`. */
  const rowPitch = async (fontSize: number, kind: 'helper' | 'old'): Promise<number> => {
    const page = await browser!.newPage({ viewport: { width: 900, height: 600 } });
    try {
      await page.setContent('<div id="host" style="width:640px;height:320px"></div>');
      await page.addStyleTag({ content: XTERM_CSS });
      await page.addScriptTag({ content: XTERM_JS });
      await page.addScriptTag({ content: helperScript() });
      return await page.evaluate(
        async ({ fontSize, kind, family }) => {
          const w = window as unknown as {
            Terminal: new (options: object) => {
              open(host: HTMLElement): void;
              write(data: string, done: () => void): void;
            };
            terminalLineHeight(family: string, size: number): number;
          };
          const lineHeight = kind === 'helper' ? w.terminalLineHeight(family, fontSize) : 20 / 12;
          const term = new w.Terminal({ fontFamily: family, fontSize, lineHeight });
          term.open(document.getElementById('host')!);
          await new Promise<void>((done) => term.write('one\r\ntwo\r\nthree', done));
          await new Promise((done) => requestAnimationFrame(() => done(undefined)));
          const rows = document.querySelectorAll<HTMLElement>('.xterm-rows > div');
          return rows[1].getBoundingClientRect().top - rows[0].getBoundingClientRect().top;
        },
        { fontSize, kind, family: FAMILY }
      );
    } finally {
      await page.close();
    }
  };

  it('needs a real browser', () => {
    expect(browser, 'Playwright Chromium did not launch').not.toBeNull();
  });

  for (const fontSize of [12, 15]) {
    it(`gives ${fontSize}px type a row of ${(fontSize * 20) / 12}px`, async () => {
      const pitch = await rowPitch(fontSize, 'helper');
      expect(Math.abs(pitch - (fontSize * 20) / 12)).toBeLessThanOrEqual(1);
    });
  }

  it('control: the old multiplier draws a taller row', async () => {
    expect(await rowPitch(12, 'old')).toBeGreaterThan(22);
  });
});
