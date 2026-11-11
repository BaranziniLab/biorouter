// @vitest-environment node
/**
 * A label hint never comes back over the menu or popover its own trigger opened (H2, "hover
 * peeks, click pins").
 *
 * Every trigger below wears a Radix label `Tooltip` around a NON-modal menu or popover trigger:
 * the Tools chip, the model chip, the composer's `+` and a row's `⋯`. A non-modal surface leaves
 * pointer events on the page, so a pointer that leaves the open trigger and comes back fires the
 * tooltip's `pointermove` again, and 500ms later the trigger's own name sat over the surface it
 * had just opened (on the two composer chips, over the popover's or menu's bottom edge, because
 * both open upward). Only real pointer input can show it: jsdom has no shared-provider timing and
 * no hit testing. So the REAL components are bundled with esbuild and driven in Chromium inside
 * the app's own `AppTooltipProvider`. Only the modules that read the daemon are stand-ins: the
 * Tools chip's three lists, and the model chip's config and model contexts and its two dialogs.
 *
 * The plain button stands for every other tooltip in the app. A refused hint must leave the
 * shared provider's 500ms delay alone: a refusal written as `onOpenChange(true) → ignore` still
 * tells the provider a tooltip opened, its skip flag then stays set, and the next tooltip
 * anywhere opens with no delay at all (`PreviewTip` records the same trap).
 *
 * Nothing here needs the app's CSS: a hint is judged by `role="tooltip"` in the DOM, and the
 * content unmounts as soon as it closes because no stylesheet gives it an exit animation.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));

const MODEL = 'gpt-5.6-sol-2026-07-09';

const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');

/** A token's first declaration in main.css (its light, base value), as written. */
function declaration(name: string): string {
  const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
  if (!match) throw new Error(`main.css does not declare ${name}`);
  return match[1].trim();
}

/**
 * The kept step a ghost trigger holds while its menu is open (`.br-surface-trigger` in main.css,
 * read verbatim, with the tokens it names), on a page whose buttons rest clear as the ghost
 * button's own utilities make them in the app (Tailwind does not run here).
 */
function keptTriggerCss(): string {
  const rules = MAIN_CSS.match(/\n\.br-surface-trigger\[[^{]*\{[^}]*\}/g) ?? [];
  if (rules.length === 0) throw new Error('main.css has no .br-surface-trigger rule');
  const tokens = ['--background-fill-strong', '--text-default', '--text-muted', '--overlay-pressed']
    .map((name) => `${name}: ${declaration(name)};`)
    .join(' ');
  return `:root { ${tokens} } button { background: transparent; color: var(--text-muted); }${rules.join('')}`;
}

/** Stand-ins for the modules that read the daemon. Everything else is the app's own code. */
const STUBS: Record<string, string> = {
  tools: `
    export function useExtensionPicker() {
      return { activeCount: 3, visibleCount: 3, answered: true, setQuery() {} };
    }
    export function useSkillPicker() {
      return { activeCount: 2, visibleCount: 2, answered: true, setQuery() {} };
    }
    export function useKnowledgePicker() {
      return { activeCount: 1, visibleCount: 1, answered: true, setQuery() {} };
    }
    export function ExtensionsPanel() { return <p>Extensions list</p>; }
    export function SkillsPanel() { return <p>Skills list</p>; }
    export function KnowledgePanel() { return <p>Knowledge list</p>; }
  `,
  config: `
    const providers = [
      {
        name: 'versa_azure',
        is_configured: true,
        provider_type: 'Builtin',
        metadata: { name: 'versa_azure', display_name: 'Versa API Azure', tier: 'private', runs_locally: false },
        affiliation: null,
        resolved_tier: 'private',
      },
    ];
    const config = { read: async () => '', getProviders: async () => providers, providersList: providers };
    export function useConfig() { return config; }
    export function usePrivacyTiersEnabled() { return true; }
    export function usePrivacyTiersRecord() { return null; }
  `,
  model: `
    const value = {
      currentModel: '${MODEL}',
      currentProvider: 'versa_azure',
      getCurrentModelAndProviderForDisplay: async () => ({ model: '${MODEL}', provider: 'Versa API Azure' }),
      getCurrentModelDisplayName: async () => '${MODEL}',
      getCurrentProviderDisplayName: async () => 'Versa API Azure',
    };
    export function useModelAndProvider() { return value; }
  `,
  dialogs: `
    export function SwitchModelModal() { return null; }
  `,
};

const stubDaemon: Plugin = {
  name: 'stub-daemon-modules',
  setup(builder) {
    const route: Array<[RegExp, keyof typeof STUBS]> = [
      [/^\.\/BottomMenu(Extension|Skill|Knowledge)Selection$/, 'tools'],
      [/(^|\/)ConfigContext$/, 'config'],
      [/(^|\/)ModelAndProviderContext$/, 'model'],
      [/^\.\.\/subcomponents\/SwitchModelModal$/, 'dialogs'],
    ];
    builder.onResolve({ filter: /.*/ }, (args) => {
      const hit = route.find(([filter]) => filter.test(args.path));
      return hit ? { path: hit[1], namespace: 'daemon-stub' } : undefined;
    });
    builder.onLoad({ filter: /.*/, namespace: 'daemon-stub' }, ({ path }) => ({
      contents: STUBS[path],
      loader: 'tsx',
      resolveDir: here,
    }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { AppTooltipProvider, Tooltip, TooltipContent, TooltipTrigger } from './Tooltip';
import { RowActions } from './row-actions';
import { ToolsChip } from '../bottom_menu/ToolsChip';
import { ComposerPlusMenu } from '../composer/ComposerPlusMenu';
import ModelsBottomBar from '../settings/models/bottom_bar/ModelsBottomBar';

function Spot({ style, children }: { style: React.CSSProperties; children: React.ReactNode }) {
  return <div style={{ position: 'fixed', ...style }}>{children}</div>;
}

function App() {
  return (
    <AppTooltipProvider>
      <Spot style={{ left: 40, top: 40 }}>
        <Tooltip>
          <TooltipTrigger asChild>
            <button type="button" data-testid="plain">Plain</button>
          </TooltipTrigger>
          <TooltipContent>Plain hint</TooltipContent>
        </Tooltip>
      </Spot>
      {/* The composer's control row, near the window's bottom edge as in a chat: both
          chips' surfaces open upward over the space their hints would take. */}
      <Spot style={{ left: 40, bottom: 64 }}>
        <ToolsChip sessionId="chat-1" />
      </Spot>
      <Spot style={{ left: 200, bottom: 64 }}>
        <ModelsBottomBar
          sessionId="s1"
          privacyTier="private"
          setView={() => {}}
          reasoningScope="session:s1"
          hideAlertPopover
        />
      </Spot>
      <Spot style={{ left: 520, bottom: 64 }}>
        <ComposerPlusMenu onAttachFiles={() => {}} onInsertTrigger={() => {}} />
      </Spot>
      <Spot style={{ left: 700, top: 200 }}>
        <RowActions
          menuTestId="row-more"
          menu={[
            { label: 'Rename', onSelect() {} },
            { label: 'Delete', onSelect() {} },
          ]}
        />
      </Spot>
    </AppTooltipProvider>
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

const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));

/** Past the 500ms label delay, with room for a loaded machine. */
const PAST_THE_DELAY_MS = 900;

/** Far from every trigger and every surface they open. */
const AWAY = { x: 980, y: 690 };

interface Site {
  /** What the case is called. */
  name: string;
  /** The trigger. */
  trigger: string;
  /** The surface the trigger opens, once it is open. */
  surface: string;
}

const SITES: Site[] = [
  {
    name: 'the Tools chip',
    trigger: '[data-testid="composer-tools-chip"]',
    surface: '[data-testid="composer-tools-popover"]',
  },
  { name: 'the model chip', trigger: '[data-testid="model-chip"]', surface: '[role="menu"]' },
  { name: 'the composer’s +', trigger: '[data-testid="composer-plus"]', surface: '[role="menu"]' },
  { name: 'a row’s ⋯', trigger: '[data-testid="row-more"]', surface: '[role="menu"]' },
];

describe('a label hint never comes back over its own open menu or popover (H2)', () => {
  let browser: Browser | null = null;
  let script = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the hint tests.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubDaemon],
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function load(extraCss = ''): Promise<Page> {
    const page = await browser!.newPage({ viewport: { width: 1000, height: 700 } });
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.setContent(
      `<!doctype html><html><head><style>
        body { margin: 0; font: 14px Arial, sans-serif; }
        button { min-width: 28px; min-height: 28px; }
        ${extraCss}
      </style></head><body><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    for (const site of SITES) await page.waitForSelector(site.trigger);
    expect(errors, 'the page renders without an error').toEqual([]);
    return page;
  }

  /** The pointer onto the centre of `selector` in one move, then a 2px nudge inside it. */
  async function pointAt(page: Page, selector: string) {
    const box = await page.locator(selector).first().boundingBox();
    if (!box) throw new Error(`nothing at ${selector}`);
    const x = box.x + box.width / 2;
    const y = box.y + box.height / 2;
    await page.mouse.move(x, y);
    await sleep(30);
    await page.mouse.move(x + 2, y);
  }

  const hints = (page: Page) => page.locator('[role="tooltip"]').count();

  for (const site of SITES) {
    it(`${site.name}: the hint shows on a hover, closes on the press, and stays shut over the open surface`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await load();
      try {
        // A hover still shows the name: what this guards is ONLY the open surface.
        await page.mouse.move(AWAY.x, AWAY.y);
        await pointAt(page, site.trigger);
        await sleep(PAST_THE_DELAY_MS);
        expect(await hints(page), 'the hover shows the hint').toBe(1);

        await page.locator(site.trigger).click();
        await page.waitForSelector(site.surface, { timeout: 3000 });
        expect(await hints(page), 'the press closes the hint').toBe(0);

        // Off the trigger and back onto it, the surface still open.
        await page.mouse.move(AWAY.x, AWAY.y);
        await sleep(100);
        await pointAt(page, site.trigger);
        await sleep(PAST_THE_DELAY_MS);
        expect(await page.locator(site.surface).count(), 'the hover leaves the surface open').toBe(
          1
        );
        expect(
          await page.locator(site.trigger).getAttribute('aria-expanded'),
          'the trigger still says its surface is open'
        ).toBe('true');
        expect(await hints(page), 'no hint over the open surface').toBe(0);

        // Closed again, the name comes back on the next hover.
        await page.keyboard.press('Escape');
        await page.waitForSelector(site.surface, { state: 'detached', timeout: 3000 });
        await page.mouse.move(AWAY.x, AWAY.y);
        await sleep(100);
        await pointAt(page, site.trigger);
        await sleep(PAST_THE_DELAY_MS);
        expect(await hints(page), 'the hint comes back once the surface has closed').toBe(1);
      } finally {
        await page.close();
      }
    });
  }

  for (const site of SITES) {
    it(`${site.name}: a second click closes the surface and the hint waits for the pointer to leave`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await load();
      try {
        await page.mouse.move(AWAY.x, AWAY.y);
        await pointAt(page, site.trigger);
        await page.mouse.down();
        await page.mouse.up();
        await page.waitForSelector(site.surface, { timeout: 3000 });
        // Off and back on while the surface is open: that return is the hover this file refuses,
        // so Radix never records that the pointer opened the hint, and only the press hold
        // stands between the next click and a hint under the pointer.
        await page.mouse.move(AWAY.x, AWAY.y);
        await sleep(100);
        await pointAt(page, site.trigger);
        await sleep(100);
        // The same pointer presses again without leaving: the surface closes.
        await page.mouse.down();
        await page.mouse.up();
        await page.waitForSelector(site.surface, { state: 'detached', timeout: 3000 });
        await sleep(30);
        const box = (await page.locator(site.trigger).boundingBox())!;
        await page.mouse.move(box.x + box.width / 2 - 2, box.y + box.height / 2);
        await sleep(PAST_THE_DELAY_MS);
        expect(await hints(page), 'a press holds the hint shut while the pointer stays').toBe(0);

        await page.mouse.move(AWAY.x, AWAY.y);
        await sleep(100);
        await pointAt(page, site.trigger);
        await sleep(PAST_THE_DELAY_MS);
        expect(await hints(page), 'off and back on, the hint shows again').toBe(1);
      } finally {
        await page.close();
      }
    });
  }

  for (const site of SITES) {
    it(`${site.name}: a hint held shut over the open surface leaves the next tooltip its delay`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await load();
      try {
        await page.locator(site.trigger).click();
        await page.waitForSelector(site.surface, { timeout: 3000 });
        await page.mouse.move(AWAY.x, AWAY.y);
        await sleep(100);
        await pointAt(page, site.trigger);
        await sleep(PAST_THE_DELAY_MS);
        // Closed by a press outside, which hands focus back to nobody. Escape, which hands it
        // back to the trigger, is the next case.
        await page.mouse.click(AWAY.x, AWAY.y);
        await page.waitForSelector(site.surface, { state: 'detached', timeout: 3000 });

        // Straight to the plain button. With the provider told that a tooltip opened, its
        // skip flag is still set and this opens at once; with the delay intact it waits 500ms.
        await page.mouse.move(AWAY.x, AWAY.y);
        await sleep(400);
        await pointAt(page, '[data-testid="plain"]');
        await sleep(150);
        expect(await hints(page), 'no tooltip before the delay').toBe(0);
        await sleep(PAST_THE_DELAY_MS);
        expect(await hints(page), 'the plain hint after its delay').toBe(1);
      } finally {
        await page.close();
      }
    });
  }

  // Escape hands focus back to the trigger by script. `Tooltip`'s Tab gate declined the open that
  // focus asked for, but inside `onOpenChange`, after Radix had told the shared provider a tooltip
  // opened and with no close to follow, so the next label tooltip anywhere opened in 12ms instead
  // of 500 (measured in the app on the model chip and the `+`). The hook refuses that focus at the
  // trigger.
  for (const site of SITES) {
    it(`${site.name}: closed with Escape, which hands focus back to it, it leaves the next tooltip its delay`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await load();
      try {
        await page.locator(site.trigger).click();
        await page.waitForSelector(site.surface, { timeout: 3000 });
        await page.mouse.move(AWAY.x, AWAY.y);
        await sleep(100);
        await page.keyboard.press('Escape');
        await page.waitForSelector(site.surface, { state: 'detached', timeout: 3000 });
        expect(
          await page.evaluate((sel) => document.activeElement?.matches(sel) ?? false, site.trigger),
          'Escape put focus back on the trigger'
        ).toBe(true);
        expect(await hints(page), 'the focus Escape put back shows no hint').toBe(0);

        await sleep(1000);
        await pointAt(page, '[data-testid="plain"]');
        await sleep(150);
        expect(await hints(page), 'no tooltip before the delay').toBe(0);
        await sleep(PAST_THE_DELAY_MS);
        expect(await hints(page), 'the plain hint after its delay').toBe(1);
      } finally {
        await page.close();
      }
    });
  }

  // Only a click leaves a mark (H4): while its menu is open the trigger holds the kept step, with
  // the pointer off it, as the Tools chip beside the `+` does. The `+` and a row's `⋯` rested clear
  // and muted under their open menus.
  for (const site of SITES.filter((s) => s.trigger.includes('plus') || s.trigger.includes('row'))) {
    it(`${site.name}: holds the kept step while its menu is open, and rests clear once it closes`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await load(keptTriggerCss());
      try {
        const look = () =>
          page.evaluate((sel) => {
            const node = document.querySelector(sel)!;
            const probe = document.createElement('div');
            probe.style.backgroundColor = 'var(--background-fill-strong)';
            probe.style.color = 'var(--text-default)';
            document.body.append(probe);
            const kept = getComputedStyle(probe);
            const out = {
              background: getComputedStyle(node).backgroundColor,
              color: getComputedStyle(node).color,
              keptBackground: kept.backgroundColor,
              keptColor: kept.color,
            };
            probe.remove();
            return out;
          }, site.trigger);

        await page.mouse.move(AWAY.x, AWAY.y);
        const rest = await look();
        expect(rest.background, 'at rest, clear').toBe('rgba(0, 0, 0, 0)');

        await page.locator(site.trigger).click();
        await page.waitForSelector(site.surface, { timeout: 3000 });
        await page.mouse.move(AWAY.x, AWAY.y);
        await sleep(100);
        const kept = await look();
        expect(kept.background, 'open, the strong fill').toBe(kept.keptBackground);
        expect(kept.color, 'open, the default ink').toBe(kept.keptColor);

        await page.mouse.click(AWAY.x, AWAY.y);
        await page.waitForSelector(site.surface, { state: 'detached', timeout: 3000 });
        expect((await look()).background, 'closed, clear again').toBe('rgba(0, 0, 0, 0)');
      } finally {
        await page.close();
      }
    });
  }

  /** Tab, from wherever focus is, until `selector` holds it. */
  async function tabTo(page: Page, selector: string) {
    for (let step = 0; step < 30; step++) {
      await page.keyboard.press('Tab');
      const there = await page.evaluate(
        (sel) => document.activeElement?.matches(sel) ?? false,
        selector
      );
      if (there) return;
    }
    throw new Error(`Tab never reached ${selector}`);
  }

  /**
   * The keyboard's way in: Tab shows the hint, then a key opens the surface. A DropdownMenu opens
   * on the key's keydown (Enter, Space, ArrowDown) and no click follows, so nothing on the trigger
   * closes the hint before the menu takes focus; the Tools chip is a Popover, which a key opens
   * through the button's own click. A hint hidden there by its parent, rather than closed by
   * Radix, never told the shared provider that a tooltip closed, and the next label tooltip
   * anywhere opened at once (measured on the model chip, the `+` and a row's `⋯`, both keys).
   */
  const KEYBOARD_OPENS: Array<[Site, string]> = SITES.flatMap((site) =>
    (site.surface === '[role="menu"]' ? ['Enter', 'Space', 'ArrowDown'] : ['Enter', 'Space']).map(
      (key): [Site, string] => [site, key]
    )
  );

  for (const [site, key] of KEYBOARD_OPENS) {
    it(`${site.name}: opened with ${key} over the hint Tab showed, it leaves the next tooltip its delay`, async (ctx) => {
      if (!browser) return ctx.skip();
      const page = await load();
      try {
        await page.mouse.move(AWAY.x, AWAY.y);
        await tabTo(page, site.trigger);
        await sleep(100);
        expect(await hints(page), 'Tab shows the hint').toBe(1);

        await page.keyboard.press(key === 'Space' ? ' ' : key);
        await page.waitForSelector(site.surface, { timeout: 3000 });
        await sleep(100);
        expect(await hints(page), 'no hint over the open surface').toBe(0);

        // Closed by a press outside, which hands focus back to nobody.
        await page.mouse.click(AWAY.x, AWAY.y);
        await page.waitForSelector(site.surface, { state: 'detached', timeout: 3000 });
        expect(await hints(page), 'the hint does not come back by itself').toBe(0);

        await sleep(600);
        await pointAt(page, '[data-testid="plain"]');
        await sleep(150);
        expect(await hints(page), 'no tooltip before the delay').toBe(0);
        await sleep(PAST_THE_DELAY_MS);
        expect(await hints(page), 'the plain hint after its delay').toBe(1);
      } finally {
        await page.close();
      }
    });
  }
});
