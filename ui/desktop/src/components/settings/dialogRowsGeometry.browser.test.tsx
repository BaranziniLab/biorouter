// @vitest-environment node
/**
 * Settings' dialog rows and the settings select, measured in a real layout engine (vision V4-09,
 * V4-22, V4-24; spec 2.2 and 3.13).
 *
 * jsdom lays nothing out and runs no stylesheet, so a component test can say "this class is on
 * this node" and nothing about a pixel. The three findings this file closes are geometry:
 *
 *   - V4-09: the Tool permissions list is one 40px line per extension, whatever the daemon's
 *     description of it says (it used to draw that description as a second line, twelve times);
 *   - V4-22: the Reset data rows are 40px (they were 45) and the count above the list, the
 *     checkboxes and the permanence line below it start on one left edge;
 *   - V4-24: the settings select draws an edge in forced colours. Its trigger is a `Button`
 *     slotted into a `DropdownMenuTrigger`; `Button` writes its `data-slot` after the props it is
 *     handed, so the trigger is still a `[data-slot='button']` and takes the shared edge in
 *     `main.css` (WS-PRIMITIVES-3).
 *
 * The real `PermissionRulesModal`, `ResetPanel`, `ModalShell` and `SettingSelect` are bundled with
 * esbuild (the daemon client, toasts and extension helpers stubbed), rendered by React in
 * Chromium and styled by the authored `@ws WS-PRIMITIVES` blocks of `styles/main.css`, then
 * `settings.css`, with token values read from `main.css`. Tailwind is not run, so only the
 * utilities a measured box depends on are written out below.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const MAIN_CSS = readFileSync(resolve(here, '../../styles/main.css'), 'utf8');
const SETTINGS_CSS = readFileSync(resolve(here, 'settings.css'), 'utf8');

const TOKEN_NAMES = [
  '--control-md',
  '--control-sm',
  '--radius-full',
  '--radius-element',
  '--radius-inner',
  '--dur-fast-min',
  '--dur-fast',
  '--dur-fast-max',
  '--ease-out',
  '--ease-spring',
  '--row-height',
  '--radius-md',
];

/** The first declaration of each token in main.css, so a token change reaches this test. */
function readTokens(): string {
  const lines = TOKEN_NAMES.map((name) => {
    const match = MAIN_CSS.match(new RegExp(`\\n\\s*${name.replace(/-/g, '\\-')}:\\s*([^;]+);`));
    if (!match) throw new Error(`main.css does not declare ${name}`);
    return `  ${name}: ${match[1].trim()};`;
  });
  return `:root {\n${lines.join('\n')}\n}`;
}

/** Every `@ws WS-PRIMITIVES` block of main.css, in file order. */
function primitiveBlocks(): string {
  const out: string[] = [];
  const re =
    /\/\* @ws WS-PRIMITIVES begin: ([^*]+?) \*\/([\s\S]*?)\/\* @ws WS-PRIMITIVES end: \1 \*\//g;
  for (const match of MAIN_CSS.matchAll(re)) out.push(match[2]);
  if (out.length === 0) throw new Error('main.css has no @ws WS-PRIMITIVES block');
  return out.join('\n');
}

/** The forced-colours rule every Button shares, lifted from main.css as it is written there. */
function sharedButtonEdge(): string {
  const match = MAIN_CSS.match(
    /\n {2}\[data-slot='button'\] \{\s*border: 1px solid ButtonText;\s*\}/
  );
  if (!match) throw new Error("main.css has no [data-slot='button'] forced-colours edge");
  return `@media (forced-colors: active) { ${match[0]} }`;
}

/** Tailwind's preflight, the roles and the few utilities the measured boxes use. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
h2, h3, p, ul, li { margin: 0; padding: 0; list-style: none; font-size: inherit; font-weight: inherit; }
svg { display: block; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.text-label { font-size: 14px; line-height: 20px; font-weight: 500; }
.text-secondary { font-size: 13px; line-height: 18px; }
.text-supporting, .text-caps { font-size: 12px; line-height: 16px; }
.tabular-nums { font-variant-numeric: tabular-nums; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.flex { display: flex; } .flex-col { flex-direction: column; } .flex-none { flex: none; }
.flex-1 { flex: 1 1 0%; } .items-center { align-items: center; } .justify-between { justify-content: space-between; }
.justify-end { justify-content: flex-end; } .flex-wrap { flex-wrap: wrap; } .shrink-0 { flex-shrink: 0; }
.min-w-0 { min-width: 0; } .min-h-0 { min-height: 0; } .p-0 { padding: 0; }
.gap-0 { gap: 0; } .gap-1\\.5 { gap: 6px; } .gap-2 { gap: 8px; } .gap-3 { gap: 12px; }
.px-3 { padding-inline: 12px; } .py-2\\.5 { padding-block: 10px; } .px-4 { padding-inline: 16px; } .pl-4 { padding-left: 16px; }
.pt-3 { padding-top: 12px; } .pt-4 { padding-top: 16px; } .pb-2 { padding-bottom: 8px; }
.pb-3 { padding-bottom: 12px; } .pb-4 { padding-bottom: 16px; } .pr-12 { padding-right: 48px; }
.mt-1 { margin-top: 4px; } .ml-auto { margin-left: auto; } .overflow-hidden { overflow: hidden; }
.overflow-y-auto { overflow-y: auto; } .size-4 { width: 16px; height: 16px; }
.w-36 { width: 144px; } .h-control-sm { height: 28px; } .h-control-compact { height: 24px; }
.rounded-element { border-radius: 8px; } .w-full { width: 100%; } .text-left { text-align: left; }
.border-b { border-bottom: 1px solid #ccc; } .border-t { border-top: 1px solid #ccc; }
.inline-flex { display: inline-flex; } .justify-center { justify-content: center; }
.whitespace-nowrap { white-space: nowrap; }
[role='dialog'] { position: fixed; top: 24px; left: 50%; width: 480px; margin-left: -240px;
  max-height: 85vh; display: flex; flex-direction: column; overflow: hidden; background: #fff;
  border-radius: 12px; }
[data-slot='dialog-overlay'] { position: fixed; inset: 0; background: rgb(0 0 0 / 0.2); }
.biorouter-settings-list { display: flex; flex-direction: column; }
.biorouter-settings-section-header { margin-bottom: 8px; }
.bg-background-medium { background-color: #ebebe8; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --text-danger: #b00020; --background-muted: #f2f2f0;
  --border-focus: #555; --background-focus: #ddd; --border-emphasized: #999;
  --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1); --inset-hairline: inset 0 0 0 1px #ddd;
  --overlay-hover: rgb(0 0 0 / 0.05); --border-subtle: #ccc; --ring: #555; }
`;

/** What `import` specifiers the real components reach for that a page cannot serve. */
const STUBS: Array<{ filter: RegExp; contents: string }> = [
  {
    filter: /\/api$/,
    contents: `
      export const previewReset = async () => ({ data: { counts: {
        applications: 0, knowledgeBases: 1, skills: 1, extensions: 0, schedules: 0, workflows: 0,
        conversations: 11780 } } });
      export const resetAppData = async () => ({});
    `,
  },
  {
    filter: /\/toasts$/,
    contents: `export const toastService = { success() {}, error() {} };`,
  },
  {
    filter: /\/utils\/sessionCache$/,
    contents: `export const clearAllSessionCache = () => {};`,
  },
  {
    filter: /\/utils\/sessionListCache$/,
    contents: `export const clearSessionListCache = () => {};`,
  },
  {
    filter: /\/utils\/localMessageStorage$/,
    contents: `export const LocalMessageStorage = { clear() {} };`,
  },
  {
    filter: /\/utils\/userAction$/,
    contents: `export const userActionHeaders = async () => ({});`,
  },
  {
    filter: /\/ConfigContext$/,
    contents: `export const useConfig = () => ({ getExtensions: async () => globalThis.__extensions });`,
  },
  {
    filter: /\/extensions\/subcomponents\/ExtensionList$/,
    contents: `export const getFriendlyTitle = (extension) => extension.display_name || extension.name;`,
  },
  {
    filter: /\/extensions\/utils$/,
    contents: `export const nameToKey = (name) => String(name).toLowerCase().replace(/\\s+/g, '-');`,
  },
  {
    filter: /^\.\/PermissionModal$/,
    contents: `export default function PermissionModal() { return null; }`,
  },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) =>
        // Only the specifiers the components under test import; the stub files do not exist.
        args.importer.includes('/settings/')
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    // The stylesheets are added to the page by hand.
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import PermissionRulesModal from './permission/PermissionRulesModal';
import ResetPanel from './app/ResetPanel';
import { SettingSelect } from './SettingSelect';
import { SettingRow, SettingSection } from '../ui/setting-row';

const scenario = globalThis.__scenario;
const root = createRoot(document.getElementById('root')!);

if (scenario === 'rules') {
  root.render(<PermissionRulesModal isOpen onClose={() => {}} />);
} else if (scenario === 'reset') {
  root.render(<div style={{ width: 712, padding: 16 }}><ResetPanel /></div>);
} else {
  root.render(
    <div style={{ width: 712, padding: 16 }}>
      <SettingSection title="Approvals">
        <SettingRow label="Approval mode">
          <SettingSelect
            options={[{ value: 'auto', label: 'Autonomous' }, { value: 'approve', label: 'Manual' }]}
            value="auto"
            onValueChange={() => {}}
            aria-label="Approval mode"
            triggerClassName="w-36"
          />
        </SettingRow>
      </SettingSection>
    </div>
  );
}
`;

const EXTENSIONS = [
  {
    type: 'builtin',
    name: 'agent_drafter',
    display_name: 'Agent Drafter',
    description:
      'Build interactive artifacts — static pages or apps with an embedded BioRouter agent — and export them to share with the people you work with.',
    enabled: true,
  },
  {
    type: 'builtin',
    name: 'developer',
    display_name: 'Developer',
    description: 'General development tools useful for software engineering.',
    enabled: true,
  },
  { type: 'builtin', name: 'memory', display_name: 'Memory', description: '', enabled: true },
];

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

const EPSILON = 0.5;
const near = (actual: number, expected: number, what: string) =>
  expect(Math.abs(actual - expected), `${what}: ${actual} vs ${expected}`).toBeLessThanOrEqual(
    EPSILON
  );

describe('Settings dialog rows and the settings select in a real layout engine', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the dialog rows.');
      return;
    }
    const bundle = await build({
      stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      plugins: [stubPlugin],
      define: { 'process.env.NODE_ENV': '"production"' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    css = PREFLIGHT + readTokens() + primitiveBlocks() + sharedButtonEdge() + SETTINGS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Let every entrance finish: a dialog scales in, and a rect read mid-motion is off. */
  const settle = (tab: Page) =>
    tab.evaluate(() =>
      Promise.all(document.getAnimations().map((animation) => animation.finished))
    );

  async function open(scenario: 'rules' | 'reset' | 'select', stylesheet = css): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${stylesheet}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.evaluate(
      ([name, extensions]) => {
        Object.assign(globalThis, { __scenario: name, __extensions: extensions });
      },
      [scenario, EXTENSIONS] as const
    );
    await tab.addScriptTag({ content: script });
    return tab;
  }

  it('draws every Tool permissions row as one 40px line, whatever the daemon says (V4-09)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('rules');
    try {
      await tab.waitForSelector('[role="dialog"] .biorouter-settings-row');
      await settle(tab);
      const rows = await tab.evaluate(() =>
        [...document.querySelectorAll('[role="dialog"] .biorouter-settings-row')].map((row) => ({
          height: row.getBoundingClientRect().height,
          text: row.textContent,
          lines: row.querySelectorAll('.br-setting-row-label').length,
        }))
      );
      expect(rows.map((row) => row.text)).toEqual(['Agent Drafter', 'Developer', 'Memory']);
      for (const row of rows) near(row.height, 40, `${row.text} row`);
    } finally {
      await tab.close();
    }
  });

  it('draws the Reset data rows at 40px and starts the count, the boxes and the permanence line on one edge (V4-22)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('reset');
    try {
      await tab.getByRole('button', { name: 'Reset data', exact: true }).click();
      await tab.waitForSelector('[role="dialog"] [data-testid="reset-option-applications"]');
      await settle(tab);
      const m = await tab.evaluate(() => {
        const dialog = document.querySelector('[role="dialog"]')!;
        const rect = (el: Element | null) => el!.getBoundingClientRect().toJSON();
        // Where the TEXT starts: the element's own box includes the padding that insets it.
        const textLeft = (el: Element) => {
          const range = document.createRange();
          range.selectNodeContents(el);
          return range.getBoundingClientRect().left;
        };
        const rows = [...dialog.querySelectorAll('[data-testid^="reset-option-"]')];
        const count = [...dialog.querySelectorAll('span')].find((el) =>
          /of \d+ selected/.test(el.textContent ?? '')
        );
        const permanence = [...dialog.querySelectorAll('p')].find((el) =>
          /permanent/i.test(el.textContent ?? '')
        );
        return {
          rowHeights: rows.map((row) => row.getBoundingClientRect().height),
          boxLeft: rect(rows[0].querySelector('.br-checkbox-box')).left,
          countLeft: textLeft(count!),
          permanenceLeft: textLeft(permanence!),
          labelLeft: rect(rows[0].querySelector('label')).left,
          boxCentre:
            rect(rows[0].querySelector('.br-checkbox-box')).top +
            rect(rows[0].querySelector('.br-checkbox-box')).height / 2,
          rowCentre: rect(rows[0]).top + rect(rows[0]).height / 2,
        };
      });
      expect(m.rowHeights).toHaveLength(7);
      for (const height of m.rowHeights) near(height, 40, 'reset option row');
      near(m.countLeft, m.boxLeft, 'the count against the checkbox box');
      near(m.permanenceLeft, m.boxLeft, 'the permanence line against the checkbox box');
      // The box still sits on the row's centre line; the 1px transparent edge is the bottom.
      near(m.boxCentre, m.rowCentre - 0.5, 'the box on the row centre');
      expect(m.labelLeft).toBeGreaterThan(m.boxLeft);
    } finally {
      await tab.close();
    }
  });

  /**
   * The select's trigger is a Button slotted into a DropdownMenuTrigger. `Button` keeps its own
   * `data-slot` over the trigger's, so the shared `[data-slot='button']` edge in `main.css` is
   * the rule that draws it in forced colours; at rest it is a fill with no outline.
   */
  it('draws the settings select with an edge in forced colours and none at rest (V4-24)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open('select');
    try {
      await tab.waitForSelector('button[aria-haspopup="menu"]');
      const edge = () =>
        tab.evaluate(() => {
          const trigger = document.querySelector('button[aria-haspopup="menu"]')!;
          const cs = getComputedStyle(trigger);
          const box = trigger.getBoundingClientRect();
          return {
            width: cs.borderTopWidth,
            style: cs.borderTopStyle,
            colour: cs.borderTopColor,
            size: [box.width, box.height],
          };
        });
      const rest = await edge();
      expect(rest.width, 'at rest the select is a fill with no outline (R-27)').toBe('0px');

      await tab.emulateMedia({ forcedColors: 'active' });
      const forced = await edge();
      expect(forced.width, 'forced colours draw the edge').toBe('1px');
      expect(forced.style).toBe('solid');
      expect(forced.colour, 'in a visible system colour, not clear').not.toMatch(
        /^rgba\(.*,\s*0\)$/
      );
      expect(forced.colour).not.toBe('transparent');
      expect(forced.size, 'the edge costs no geometry').toEqual(rest.size);
    } finally {
      await tab.close();
    }
  });

  /**
   * The band's title shares an ancestor with the tabs, and the Tabs root paints its text muted.
   * jsdom sees neither the cascade nor `:has()`, so the ink is read in Chromium (V4-06).
   */
  it('draws the band title in the default ink, not the muted ink of the inactive tabs (V4-06)', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await browser.newPage({ viewport: { width: 900, height: 300 } });
    try {
      await tab.setContent(`<!doctype html><html><head><style>${css}
        .text-text-muted { color: var(--text-muted); } .text-text-default { color: var(--text-default); }
      </style></head><body>
        <div class="text-text-muted">
          <header class="biorouter-page-header">
            <div class="biorouter-page-header-bar">
              <div class="biorouter-page-header-title"><h1 class="text-label">Settings</h1></div>
              <div class="biorouter-page-header-tabs">
                <div role="tablist" class="biorouter-settings-tabs">
                  <button role="tab" data-state="active" class="text-label text-text-default">Models</button>
                  <button role="tab" data-state="inactive" class="text-label text-text-muted">Chat</button>
                </div>
              </div>
            </div>
          </header>
          <header class="biorouter-page-header">
            <div class="biorouter-page-header-bar">
              <div class="biorouter-page-header-title"><h1 class="text-label">Workflows</h1></div>
            </div>
          </header>
        </div>
      </body></html>`);
      const ink = await tab.evaluate(() => {
        const color = (el: Element | null) => getComputedStyle(el!).color;
        const [settings, other] = [...document.querySelectorAll('h1')];
        return {
          title: color(settings),
          activeTab: color(document.querySelector('[data-state="active"]')),
          inactiveTab: color(document.querySelector('[data-state="inactive"]')),
          otherTitle: color(other),
        };
      });
      expect(ink.title, 'the title is not the inactive tabs’ ink').not.toBe(ink.inactiveTab);
      expect(ink.title, 'the title wears the active tab’s ink').toBe(ink.activeTab);
      // Scoped to Settings: a page header with no settings tab list keeps what it inherits.
      expect(ink.otherTitle).toBe(ink.inactiveTab);
    } finally {
      await tab.close();
    }
  });
});
