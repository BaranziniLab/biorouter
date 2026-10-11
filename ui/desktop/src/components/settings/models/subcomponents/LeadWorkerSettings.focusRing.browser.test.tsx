// @vitest-environment node
/**
 * The Lead/worker dialog opened from the model chip's menu, measured with real input in Chromium
 * (vision round 4, V4-R4-1). jsdom has no `:focus-visible`, and that is the whole defect.
 *
 * The dialog's models load after it mounts, so it rests focus on its box and hands it to the first
 * switch once the form is in (`../dialogFocus.ts`). After a mouse click on the menu item, Chromium
 * matches `:focus-visible` on that script focus, so the switch wore the 2px inset grey ring the
 * person never asked for. The real `LeadWorkerSettings`, `DropdownMenu` and `ModalShell` are
 * bundled with esbuild (the config and model contexts are stubbed), `styles/main.css` is compiled
 * by Tailwind with the classes the bundle uses so the app's own focus rules apply, and Playwright
 * drives the mouse and the keyboard:
 *
 * - **Pointer pick.** Focus stays on the dialog box: the switch is not focused, nothing in the
 *   dialog matches `:focus-visible`, and the first Tab reaches the first control in the body, not
 *   the close button.
 * - **Keyboard pick.** The switch is handed focus and does wear the ring, because there the ring
 *   is the point.
 *
 * Set `WS_SCREENSHOT_DIR` to also save a PNG of each.
 */
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../../package.json'));
const DIALOGS_CSS = readFileSync(resolve(here, 'model-dialogs.css'), 'utf8');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { Button } from '../../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../../ui/dropdown-menu';
import { LeadWorkerSettings } from './LeadWorkerSettings';

function Chip() {
  const [open, setOpen] = React.useState(false);
  return (
    <div style={{ padding: 16 }}>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button id="model-chip" variant="outline">gpt-4o</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem data-testid="lead-worker-item" onSelect={() => setOpen(true)}>
            Lead and worker...
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {open && <LeadWorkerSettings isOpen onClose={() => setOpen(false)} />}
    </div>
  );
}

createRoot(document.getElementById('root')!).render(<Chip />);
`;

/** What the dialog imports that is not its business: the config and the model contexts. */
const STUBS: Record<string, string> = {
  // Stable references, like the real context's: the dialog's load effect depends on them.
  ConfigContext: `
    const api = {
      read: async (key) => {
        if (key === 'BIOROUTER_MODEL') return 'gpt-4o';
        if (key === 'BIOROUTER_PROVIDER') return 'openai';
        return null;
      },
      upsert: async () => {},
      remove: async () => {},
      getProviders: async () => [],
      getProviderModels: async () => [],
    };
    export const useConfig = () => api;
  `,
  ModelAndProviderContext: `export const useModelAndProvider = () => ({ currentModel: null });`,
};

const stubPlugin: Plugin = {
  name: 'lead-worker-stubs',
  setup(b) {
    b.onResolve({ filter: /(\/ConfigContext|\/ModelAndProviderContext)$/ }, (args) => {
      const name = Object.keys(STUBS).find((key) => args.path.endsWith(key));
      return name ? { path: name, namespace: 'stub' } : undefined;
    });
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: STUBS[args.path],
      loader: 'tsx',
      resolveDir: here,
    }));
  },
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

interface Focus {
  inDialog: boolean;
  isDialogBox: boolean;
  isClose: boolean;
  isSwitch: boolean;
  focusVisible: boolean;
  /** A control inside the dialog matches `:focus-visible`. */
  controlRing: boolean;
  /** The dialog box paints an outline (it matches `:focus-visible` too, and must draw nothing). */
  boxOutline: string;
}

describe('the Lead/worker dialog opened from the model menu (V4-R4-1)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the Lead/worker focus-ring tests.');
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
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      plugins: [stubPlugin],
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(STYLES, 'main.css'), 'utf8'), {
      base: STYLES,
      onDependency() {},
    });
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function openPage(): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    tab.setDefaultTimeout(8000);
    await tab.setContent(
      `<!doctype html><html><head><style>${css + DIALOGS_CSS}</style></head>` +
        `<body style="margin:0" class="bg-background-canvas"><div id="root"></div></body></html>`
    );
    await tab.evaluate(() => {
      (window as unknown as { appConfig: unknown }).appConfig = {
        get: (key: string) =>
          key === 'BIOROUTER_PREDEFINED_MODELS'
            ? JSON.stringify([
                { name: 'gpt-4o', alias: 'GPT-4o', provider: 'openai' },
                { name: 'claude-sonnet', alias: 'Claude Sonnet', provider: 'anthropic' },
              ])
            : undefined,
      };
    });
    await tab.addScriptTag({ content: script });
    await tab.waitForSelector('#model-chip');
    return tab;
  }

  /** The form is in once the number fields exist; the hand-off effect runs in the same commit. */
  const loaded = async (tab: Page) => {
    await tab.waitForSelector('#lead-turns');
    // Let Radix's entrance animation and any deferred focus move finish.
    await tab.waitForTimeout(500);
  };

  const focus = (tab: Page): Promise<Focus> =>
    tab.evaluate(() => {
      const el = document.activeElement as HTMLElement;
      const dialog = document.querySelector('[role="dialog"]') as HTMLElement;
      return {
        inDialog: dialog.contains(el),
        isDialogBox: el === dialog,
        isClose: !!el.closest('[data-slot="dialog-content"] > button'),
        isSwitch: el.getAttribute('role') === 'switch',
        focusVisible: el.matches(':focus-visible'),
        controlRing: !!dialog.querySelector(':focus-visible'),
        boxOutline: getComputedStyle(dialog).outlineStyle,
      };
    });

  const shot = async (tab: Page, name: string) => {
    const dir = process.env.WS_SCREENSHOT_DIR;
    if (!dir) return;
    mkdirSync(dir, { recursive: true });
    await tab.screenshot({ path: resolve(dir, `${name}.png`) });
  };

  it('leaves focus on the dialog box after a mouse pick, so the switch wears no ring', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await tab.click('#model-chip');
      await tab.click('[data-testid="lead-worker-item"]');
      await loaded(tab);
      const now = await focus(tab);
      expect(now.inDialog).toBe(true);
      expect(now.isDialogBox, 'focus rests on the dialog box').toBe(true);
      expect(now.isSwitch).toBe(false);
      expect(now.controlRing, 'no control in the dialog matches :focus-visible').toBe(false);
      expect(now.boxOutline, 'the dialog box draws no ring of its own').toBe('none');
      await shot(tab, 'lead-worker-pointer-open');

      // The next Tab lands in the body, on the first control, and never on the close button.
      await tab.keyboard.press('Tab');
      const afterTab = await focus(tab);
      expect(afterTab.inDialog).toBe(true);
      expect(afterTab.isDialogBox).toBe(false);
      expect(afterTab.isClose).toBe(false);
    } finally {
      await tab.close();
    }
  });

  it('hands the switch focus, ring and all, after a keyboard pick', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await openPage();
    try {
      await tab.focus('#model-chip');
      await tab.keyboard.press('Enter');
      const item = tab.locator('[data-testid="lead-worker-item"]');
      await item.waitFor();
      await item.focus();
      await tab.keyboard.press('Enter');
      await loaded(tab);
      const now = await focus(tab);
      expect(now.isSwitch, 'the first switch holds focus').toBe(true);
      expect(now.focusVisible, 'and wears the keyboard ring').toBe(true);
      await shot(tab, 'lead-worker-keyboard-open');
    } finally {
      await tab.close();
    }
  });
});
