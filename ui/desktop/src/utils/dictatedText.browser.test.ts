// @vitest-environment node
/**
 * Dictated text into every kind of field, in a real Chromium (DD-63, DD-98; "Inserting text" in
 * `docs/dictation/surfaces.md`).
 *
 * jsdom has no `execCommand`, no editing host and no window focus, so `dictatedText.test.ts` can
 * only fake what the browser does. Here the real `utils/dictatedText.ts` is bundled with esbuild
 * into a page holding a text `input`, a `textarea`, a `contenteditable` region and a React
 * controlled `input`, and each takes a transcript:
 *
 *  - with the window focused and the field focused: the undoable path, one `input` event, and
 *    the text where it was planned (read back);
 *  - after the window lost focus (another page in front, as when the person switched apps while
 *    the text was being finished): the field still gets the text, still fires `input`, and the
 *    read back still passes;
 *  - with focus in another field: the field's own state path (the prototype's value setter and an
 *    `input` event), which a React controlled field keeps because its `onChange` runs.
 *
 * It needs a Chromium; with none installed it warns and passes, like the other `.browser.test`
 * files.
 */
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { chromium, type Browser, type BrowserContext, type Page } from 'playwright';
import { afterAll, beforeAll, beforeEach, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

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

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { insertDictatedText, setFieldTextByState } from './dictatedText';

function Controlled() {
  const [value, setValue] = React.useState('Draft');
  return <input id="controlled" value={value} onChange={(event) => setValue(event.target.value)} />;
}
flushSync(() => createRoot(document.getElementById('root')).render(<Controlled />));

const inputs = {};
for (const id of ['plain', 'area', 'region', 'controlled', 'elsewhere']) {
  inputs[id] = 0;
  document.getElementById(id).addEventListener('input', () => inputs[id]++);
}

window.dictation = {
  inputs,
  insert(id, at, text) {
    const field = document.getElementById(id);
    const control = field.localName === 'input' || field.localName === 'textarea';
    return insertDictatedText(
      field,
      { start: at, end: at },
      text,
      (value, caret) => (control ? setFieldTextByState(field, value, caret) : false)
    );
  },
  text(id) {
    const field = document.getElementById(id);
    return 'value' in field && field.localName !== 'div' ? field.value : field.textContent;
  },
};
`;

const PAGE = `<!doctype html><html><body>
<input id="plain" value="Check">
<textarea id="area">Check</textarea>
<div id="region" contenteditable="true">Check</div>
<div id="root"></div>
<input id="elsewhere" value="">
</body></html>`;

/** What a field shows, with the no-break spaces Chromium writes at an edge read as spaces. */
const plain = (text: string) => text.replace(/\u00a0/g, ' ');

describe('dictated text in a real browser', () => {
  let browser: Browser | null = null;
  let context: BrowserContext | null = null;
  let page: Page | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the dictated text browser tests.');
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
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
  }, 60_000);

  afterAll(async () => {
    await browser?.close();
  });

  beforeEach(async () => {
    if (!browser) return;
    await context?.close();
    context = await browser.newContext();
    page = await context.newPage();
    await page.setContent(PAGE);
    await page.addScriptTag({ content: script });
  });

  /** Another page comes to the front, so this one's window loses focus; its field keeps focus. */
  async function blurWindow(): Promise<void> {
    const cdp = await context!.newCDPSession(page!);
    await cdp.send('Emulation.setFocusEmulationEnabled', { enabled: false });
    const other = await context!.newPage();
    await other.bringToFront();
    await page!.waitForFunction(() => !document.hasFocus());
  }

  const focus = (id: string, at?: number) =>
    page!.evaluate(
      ([target, caret]) => {
        const field = document.getElementById(target as string) as HTMLElement;
        field.focus();
        if (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) {
          const end = caret ?? field.value.length;
          field.setSelectionRange(end as number, end as number);
        }
        return document.activeElement?.id;
      },
      [id, at] as const
    );

  const insert = (id: string, at: number, text: string) =>
    page!.evaluate(
      ([target, caret, words]) =>
        (
          window as unknown as {
            dictation: { insert(id: string, at: number, text: string): string };
          }
        ).dictation.insert(target as string, caret as number, words as string),
      [id, at, text] as const
    );

  const read = (id: string) =>
    page!.evaluate(
      (target) =>
        (window as unknown as { dictation: { text(id: string): string } }).dictation.text(target),
      id
    );

  const inputs = (id: string) =>
    page!.evaluate(
      (target) =>
        (window as unknown as { dictation: { inputs: Record<string, number> } }).dictation.inputs[
          target
        ],
      id
    );

  for (const id of ['plain', 'area', 'region']) {
    it(`${id}: the undoable path while focused, with one input event and the text read back`, async () => {
      if (!browser) return;
      expect(await focus(id)).toBe(id);
      expect(await insert(id, 5, 'EPAS1 levels')).toBe('undoable');
      expect(plain(await read(id))).toBe('Check EPAS1 levels');
      expect(await inputs(id)).toBe(1);
    });

    it(`${id}: still gets the text after the window lost focus, firing input`, async () => {
      if (!browser) return;
      await focus(id);
      await blurWindow();
      expect(await page!.evaluate(() => document.hasFocus())).toBe(false);
      const result = await insert(id, 5, 'EPAS1 levels');
      expect(['undoable', 'state']).toContain(result);
      expect(plain(await read(id))).toBe('Check EPAS1 levels');
      expect(await inputs(id)).toBe(1);
    });
  }

  it('a field without focus takes its own state path, with an input event', async () => {
    if (!browser) return;
    await focus('elsewhere');
    expect(await insert('plain', 0, 'EPAS1')).toBe('state');
    expect(await read('plain')).toBe('EPAS1 Check');
    expect(await inputs('plain')).toBe(1);
    expect(await insert('region', 5, 'gene')).toBe('state');
    expect(plain(await read('region'))).toBe('Check gene');
    expect(await inputs('region')).toBe(1);
    // Focus stayed where the person put it.
    expect(await page!.evaluate(() => document.activeElement?.id)).toBe('elsewhere');
  });

  it('a React controlled field keeps the text, focused or not: its onChange runs', async () => {
    if (!browser) return;
    await focus('controlled');
    expect(await insert('controlled', 5, 'notes')).toBe('undoable');
    expect(await read('controlled')).toBe('Draft notes');
    await focus('elsewhere');
    expect(await insert('controlled', 11, 'today')).toBe('state');
    expect(await read('controlled')).toBe('Draft notes today');
    expect(await inputs('controlled')).toBe(2);
  });
});
