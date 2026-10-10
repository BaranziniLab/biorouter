// @vitest-environment node
/**
 * The configure dialog's two key-value groups, measured in a real layout engine (spec 3.11).
 *
 * jsdom lays nothing out, so `ExtensionModal.test.tsx` can say the Add button exists and is named,
 * but not that it is as wide as its label. The request-headers group once had a three-column grid
 * whose last column was the 32px of an icon button, so "Add" was squeezed into a box it did not
 * fit and spilled out of the dialog (found by looking at it in a browser, 2026-10-10). Both
 * groups now share one four-column rhythm (key, value, edit, remove; Add spans the last two), and
 * these tests hold them to it.
 *
 * This renders the real `EnvVarsSection` and `HeadersSection` to markup and lays them out in
 * Chromium with `extension-modal.css` at the token values.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { renderToStaticMarkup } from 'react-dom/server';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import EnvVarsSection from './EnvVarsSection';
import HeadersSection from './HeadersSection';

const here = dirname(fileURLToPath(import.meta.url));

/** `main.css`'s values for the tokens the groups read, and the utilities their markup uses. */
const TOKENS = `:root {
  --font-body: Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, sans-serif;
  --control-md: 32px;
  --radius-element: 8px;
  --border-subtle: #e5e3df;
  --border-control: #8a867f;
}
* { box-sizing: border-box; }
body { margin: 0; font: 14px/20px var(--font-body); }
h3, p { margin: 0; }
button, input { font: inherit; margin: 0; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-supporting { font-size: 12px; line-height: 16px; }
.text-text-default { color: #1d1c1a; } .text-text-muted { color: #6b6963; } .text-text-danger { color: #b3261e; }
.flex { display: flex; } .inline-flex { display: inline-flex; } .w-full { width: 100%; }
.items-center { align-items: center; } .justify-center { justify-content: center; }
.gap-2 { gap: 8px; } .whitespace-nowrap { white-space: nowrap; } .shrink-0 { flex-shrink: 0; }
.h-control-md { height: 32px; } .w-control-md { width: 32px; } .p-0 { padding: 0; }
.px-2 { padding-inline: 8px; } .px-3 { padding-inline: 12px; }
.rounded-element { border-radius: 8px; } .border { border: 1px solid var(--border-control); }
.bg-background-default { background: #fff; } .bg-background-medium { background: #ebe9e5; }
.text-text-muted { color: #6b6963; }
svg { flex: none; width: 16px; height: 16px; }
.br-info-tip { display: inline-flex; width: 24px; height: 24px; margin-block: -5px; margin-inline: -1px -5px; border: 0; padding: 0; background: none; }
.br-info-tip > svg { width: 14px; height: 14px; }
.br-row-actions-slot, .br-row-actions { display: inline-flex; }
.sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }`;

const css = readFileSync(resolve(here, 'extension-modal.css'), 'utf8');

const noop = () => {};

function page(): string {
  const env = renderToStaticMarkup(
    <EnvVarsSection
      envVars={[
        { key: 'CDW_PASSWORD', value: '••••••••', isEdited: false },
        { key: 'API_URL', value: 'https://example.com', isEdited: true },
      ]}
      onAdd={noop}
      onRemove={noop}
      onChange={noop}
      submitAttempted={false}
    />
  );
  const headers = renderToStaticMarkup(
    <HeadersSection
      headers={[{ key: 'Authorization', value: 'Bearer abc', isEdited: false }]}
      onAdd={noop}
      onRemove={noop}
      onChange={noop}
      submitAttempted={false}
      onPendingInputChange={noop}
    />
  );
  // The dialog body: 640px wide less 16px of padding on each side.
  return `<!doctype html><html><head><style>${TOKENS}\n${css}</style></head><body>
    <div id="body" style="width:608px;margin:0 16px;display:flex;flex-direction:column;gap:20px">
      <div id="env">${env}</div>
      <div id="headers">${headers}</div>
    </div>
  </body></html>`;
}

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

// 90s a test: a page launch alone takes seconds while other browser suites load the machine.
describe('Extension dialog key-value groups in a real layout engine', { timeout: 90_000 }, () => {
  let browser: Browser | null = null;

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn(
        'No Chromium found (Playwright’s or Google Chrome): skipping the extension dialog ' +
          'geometry tests. Run `npx playwright install chromium` to enable them.'
      );
    }
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 700, height: 800 } });
    await tab.setContent(page());
    return tab;
  }

  /** Every control in a group, by accessible name, with its box. */
  const controls = (tab: Page, group: '#env' | '#headers') =>
    tab.evaluate((root) => {
      const rect = (el: Element) => {
        const r = el.getBoundingClientRect();
        return {
          left: r.left,
          right: r.right,
          width: r.width,
          height: r.height,
          overflows: (el as HTMLElement).scrollWidth > (el as HTMLElement).clientWidth,
        };
      };
      const host = document.querySelector(root) as HTMLElement;
      const add = [...host.querySelectorAll('button')].find(
        (b) => b.textContent?.trim() === 'Add'
      )!;
      const remove = [...host.querySelectorAll('button')].filter((b) =>
        (b.getAttribute('aria-label') ?? '').startsWith('Remove ')
      );
      const inputs = [...host.querySelectorAll('input')];
      return {
        host: rect(host.querySelector('.br-ext-kv') as Element),
        add: rect(add),
        addScrollWidth: (add as HTMLElement).scrollWidth,
        remove: remove.map(rect),
        inputs: inputs.map(rect),
      };
    }, group);

  it('gives Add the room for its label, inside the group, in both groups', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      for (const group of ['#env', '#headers'] as const) {
        const { host, add, addScrollWidth } = await controls(tab, group);
        // The two-column span is 32 + 8 + 32: a label and a glyph fit, and nothing spills out.
        expect(add.width).toBe(72);
        expect(addScrollWidth).toBeLessThanOrEqual(add.width);
        expect(add.height).toBe(32);
        expect(add.right).toBeLessThanOrEqual(host.right + 0.5);
      }
    } finally {
      await tab.close();
    }
  });

  it('puts every remove button in the last 32px column, under the Add button’s right edge', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      for (const group of ['#env', '#headers'] as const) {
        const { host, add, remove } = await controls(tab, group);
        expect(remove.length).toBeGreaterThan(0);
        for (const button of remove) {
          expect(button.width).toBe(32);
          expect(button.height).toBe(32);
          expect(button.right).toBe(host.right);
          expect(button.right).toBe(add.right);
        }
      }
    } finally {
      await tab.close();
    }
  });

  it('draws the two groups on one rhythm: the same key and value columns', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const env = await controls(tab, '#env');
      const headers = await controls(tab, '#headers');
      // The first two inputs of each group are its key and value columns.
      expect(headers.inputs[0].left).toBe(env.inputs[0].left);
      expect(headers.inputs[0].width).toBe(env.inputs[0].width);
      expect(headers.inputs[1].left).toBe(env.inputs[1].left);
      expect(headers.inputs[1].width).toBe(env.inputs[1].width);
      for (const input of [...env.inputs, ...headers.inputs]) expect(input.height).toBe(32);
    } finally {
      await tab.close();
    }
  });
});
