// @vitest-environment node
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
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

const entry = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { useRestoredDraftCaret } from './Composer';
import { insertDictatedText } from '../../../utils/dictatedText';

function Card() {
  const [body, setBody] = React.useState('Dr aft');
  const field = React.useRef(null);
  const caret = useRestoredDraftCaret(field, body);
  return <div className="biorouter-composer-card">
    <textarea ref={field} value={body} onChange={(event) => {
      caret.typed(event.target.value);
      setBody(event.target.value);
    }} />
    <button onMouseDown={(event) => event.preventDefault()} onClick={() => {
      insertDictatedText(field.current, { start: 3, end: 3 }, 'EGLN1', (value, at) => {
        caret.typed(value);
        setBody(value);
        requestAnimationFrame(() => field.current?.setSelectionRange(at, at));
      });
    }}>Dictate</button>
  </div>;
}
createRoot(document.getElementById('root')).render(<Card />);
`;

let browser: Browser | null = null;
let script = '';

beforeAll(async () => {
  for (const options of [{}, { channel: 'chromium' }, { channel: 'chrome' }] as const) {
    try {
      browser = await chromium.launch(options);
      break;
    } catch {
      // Try the next installed Chromium build.
    }
  }
  if (!browser) throw new Error('No Chromium or Chrome browser could launch');
  const result = await build({
    stdin: {
      contents: entry,
      loader: 'tsx',
      resolveDir: here,
      sourcefile: 'crew-dictation-caret.tsx',
    },
    bundle: true,
    write: false,
    platform: 'browser',
    format: 'iife',
    jsx: 'automatic',
    define: {
      'process.env.NODE_ENV': '"production"',
      'import.meta.env.MODE': '"production"',
      'import.meta.env.DEV': 'false',
      'import.meta.env.PROD': 'true',
    },
    loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
    plugins: [workletUrl],
  });
  script = result.outputFiles[0].text;
});

afterAll(async () => {
  await browser?.close();
});

describe('Crew caret in Chromium', () => {
  it('stays after the dictated text in a controlled field', async () => {
    const page = await browser!.newPage();
    page.setDefaultTimeout(5000);
    const pageErrors: string[] = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    try {
      await page.setContent('<div id="root"></div>');
      await page.addScriptTag({ content: script });
      const field = page.locator('textarea');
      await field.waitFor().catch(() => {
        throw new Error(`Crew caret field did not mount: ${pageErrors[0] ?? 'no page error'}`);
      });
      await field.focus();
      await field.evaluate((element) => (element as HTMLTextAreaElement).setSelectionRange(3, 3));
      await page.getByRole('button', { name: 'Dictate' }).click();
      await expect.poll(() => field.inputValue()).toBe('Dr EGLN1 aft');
      expect(
        await field.evaluate((element) => ({
          start: (element as HTMLTextAreaElement).selectionStart,
          end: (element as HTMLTextAreaElement).selectionEnd,
        }))
      ).toEqual({ start: 9, end: 9 });
    } finally {
      await page.close();
    }
  });
});
