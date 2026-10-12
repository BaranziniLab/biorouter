// @vitest-environment node
/**
 * The first frames of a Settings tab switch, painted in Chromium (vision rounds 7 and 8, V4-R7-2).
 *
 * Settings mounts only the tab on screen, so every visit to Chat or App is a new mount. A control
 * that has to read its value first (a setting the daemon or the main process holds) was an empty
 * box for the frames the answer took, on every visit: the Approval mode select with no words, Max
 * turns with no number, and the App tab's switches filling in two steps. The values cannot have
 * changed since the last visit, so a later visit starts from the last answer
 * (`settings/lastKnown.ts`), and Settings asks for them while it opens (`warmSettings.ts`), so the
 * first visit is not held either.
 *
 * jsdom has no frames, so the real `SettingsView` is bundled with esbuild and clicked with a real
 * pointer in Chromium. Every commit React makes after the click is sampled by a MutationObserver
 * and every animation frame by `requestAnimationFrame`: a state a quick re-render replaced before
 * the next frame would hide from the frames on a fast machine and show on a loaded one, so the
 * commits are the stricter view and the frames are the one a person sees. The daemon and the main
 * process answer 600 ms late here, so everything on screen before that can only have come from
 * what the app already held.
 *
 * What is stubbed, and why: Models (not what is measured), Configuration, the Usage, Update, Reset
 * and theme sections, the permissions dialog, and the Privacy panel. Privacy is frozen privacy UI,
 * and its switch is the one control on the App tab that still waits for the daemon on a visit. The
 * Chat tab is the Approvals section and the Spellcheck row; the App tab is the real
 * `AppSettingsSection` (General: menu bar, Dock, Prevent sleep, Never open tabs, Show costs).
 *
 * "The instrument": a first visit with nothing warm is held, and a bundle whose controls ignore
 * the last answer is held on a later visit too. A sampler that cannot see a hold would pass
 * anything.
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
  '--chrome-height',
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

/** Tailwind's preflight and the few utilities the sampled boxes depend on. */
const PREFLIGHT = `
*, *::before, *::after { box-sizing: border-box; }
body { margin: 0; font-family: Arial, sans-serif; font-size: 14px; line-height: 20px; }
button { margin: 0; padding: 0; border: 0; background: none; font: inherit; color: inherit; }
h1, h2, h3, p { margin: 0; font-size: inherit; font-weight: inherit; }
svg { display: block; }
button svg { width: 16px; height: 16px; flex: none; }
.sr-only { position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px; overflow: hidden;
  clip: rect(0, 0, 0, 0); white-space: nowrap; border-width: 0; }
.flex { display: flex; } .flex-col { flex-direction: column; } .inline-flex { display: inline-flex; }
.flex-1 { flex: 1 1 0%; } .min-h-0 { min-height: 0; } .min-w-0 { min-width: 0; }
.items-center { align-items: center; } .justify-between { justify-content: space-between; }
.gap-2 { gap: 8px; } .relative { position: relative; } .truncate { overflow: hidden; white-space: nowrap; }
.h-control-sm { height: var(--control-sm); } .w-36 { width: 144px; } .w-20 { width: 80px; }
.px-6 { padding-inline: 24px; } .py-5 { padding-block: 20px; } .pb-8 { padding-bottom: 32px; }
.biorouter-settings-list { display: flex; flex-direction: column; }
.biorouter-settings-section { padding-block: 14px; }
.biorouter-settings-section-header { margin-bottom: 8px; }
:root { --control-track-off: #ebebe8; --background-accent: #b85a32; --border-control: #86847f;
  --background-default: #fff; --background-fill: #f4f4f2; --background-fill-strong: #eaeae6;
  --text-on-accent: #fff; --text-muted: #666; --text-default: #111;
  --text-subtle: #777; --text-danger: #b00020; --background-muted: #f2f2f0;
  --border-focus: #555; --background-focus: #ddd; --shadow-raised: 0 1px 2px rgb(0 0 0 / 0.1);
  --inset-hairline: inset 0 0 0 1px #ddd; --overlay-hover: rgb(0 0 0 / 0.05);
  --border-subtle: #ccc; --sidebar: #f7f7f5; --accent-bar: #c4452d; --control-lg: 36px; }
#viewport { width: 900px; height: 900px; display: flex; flex-direction: column; }
`;

/** What the real components import that a bare page cannot serve. */
const STUBS: Array<{ filter: RegExp; contents: string }> = [
  {
    filter: /\/ConfigContext$/,
    contents: `
      // The daemon, 600 ms late (a page sets __daemonDelayMs), answering from __config.
      const api = {
        read: async (key) => {
          await new Promise((done) => setTimeout(done, window.__daemonDelayMs || 0));
          return (window.__config || {})[key];
        },
        upsert: async () => {},
      };
      export const useConfig = () => api;
    `,
  },
  { filter: /\/ModelsSection$/, contents: `export default () => null;` },
  { filter: /\/ConfigSettings$/, contents: `export default () => null;` },
  { filter: /\/PrivacyPanel$/, contents: `export default () => null;` },
  { filter: /\/UpdateSection$/, contents: `export default () => null;` },
  { filter: /\/UsageSection$/, contents: `export default () => null;` },
  { filter: /\/ResetPanel$/, contents: `export default () => null;` },
  { filter: /\/FontSizeSelector$/, contents: `export default () => null;` },
  { filter: /\/ThemeSelector$/, contents: `export default () => null;` },
  { filter: /\/ThemeFamilySelector$/, contents: `export default () => null;` },
  { filter: /\/PermissionRulesModal$/, contents: `export default () => null;` },
  {
    filter: /\/disclosureCopy$/,
    contents: `export const readDisclosureAcknowledgement = async () => null;`,
  },
  {
    // The Chat tab, cut to the rows whose values the daemon and the main process hold.
    filter: /\/ChatSettingsSection$/,
    contents: `
      import * as React from 'react';
      import { ModeSection } from '${here}/mode/ModeSection';
      import { SpellcheckRow } from '${here}/chat/SpellcheckToggle';
      import { SettingSection } from '${here}/../ui/setting-row';
      export default function ChatSettingsSection() {
        return React.createElement('div', { className: 'pb-8' },
          React.createElement(ModeSection),
          React.createElement(SettingSection, { id: 'settings-display', title: 'Display' },
            React.createElement(SpellcheckRow)));
      }
    `,
  },
];

/** Where a mutated bundle puts a defect back. Must match, or throw. */
const NO_SEED: [string, string] = [
  'useState<T | undefined>(() => recall<T>(key))',
  'useState<T | undefined>(undefined)',
];

function stubPlugin(mutation?: [string, string]): Plugin {
  return {
    name: 'stubs',
    setup(b) {
      for (const { filter, contents } of STUBS) {
        b.onResolve({ filter }, (args) =>
          args.importer.includes('/settings/') || args.importer.includes('/privacy/')
            ? { path: args.path, namespace: 'stub', pluginData: contents }
            : undefined
        );
      }
      b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
        contents: args.pluginData as string,
        loader: 'js',
        resolveDir: here,
      }));
      // The stylesheets are added to the page by hand.
      b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
      if (mutation) {
        let hit = false;
        b.onLoad({ filter: /settings\/lastKnown\.ts$/ }, () => {
          const source = readFileSync(resolve(here, 'lastKnown.ts'), 'utf8');
          if (!source.includes(mutation[0])) throw new Error('the mutation matches nothing');
          hit = true;
          return { contents: source.replace(mutation[0], mutation[1]), loader: 'ts' };
        });
        b.onEnd(() => {
          if (!hit) throw new Error('the mutated module was never loaded');
        });
      }
    },
  };
}

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import SettingsView from './SettingsView';

createRoot(document.getElementById('root')!).render(
  <div id="viewport">
    <SettingsView onClose={() => {}} setView={() => {}} viewOptions={{}} />
  </div>
);
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

async function bundle(mutation?: [string, string]): Promise<string> {
  const out = await build({
    stdin: { contents: ENTRY, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
    bundle: true,
    write: false,
    format: 'iife',
    jsx: 'automatic',
    platform: 'browser',
    plugins: [stubPlugin(mutation)],
    define: { 'process.env.NODE_ENV': '"production"' },
    logLevel: 'silent',
  });
  return out.outputFiles[0].text;
}

/**
 * What the page's two kinds of answer are, set before the app mounts. The main process holds its
 * settings and answers 600 ms late with what it holds when it answers, so a write the person made
 * is what a later read returns, as it is in the app.
 */
const SETUP = `
  window.__daemonDelayMs = 600;
  window.__config = { BIOROUTER_MODE: 'approve', BIOROUTER_MAX_TURNS: 25, WORKSPACE_ANNOUNCE_ONLY: true };
  const held = { menuBar: false, dock: true, wakelock: false, spellcheck: false };
  const late = (key) => async () => {
    await new Promise((done) => setTimeout(done, 600));
    return held[key];
  };
  const store = (key) => async (value) => { held[key] = value; return true; };
  window.electron = {
    platform: 'darwin',
    getMenuBarIconState: late('menuBar'),
    getDockIconState: late('dock'),
    getWakelockState: late('wakelock'),
    getSpellcheckState: late('spellcheck'),
    setMenuBarIcon: store('menuBar'), setDockIcon: store('dock'),
    setWakelock: store('wakelock'), setSpellcheck: store('spellcheck'),
    openNotificationsSettings: async () => {},
  };
  window.appConfig = { get: () => undefined };
`;

/** Samples the Chat and App tabs' controls, in every commit and every frame from a click on. */
const PROBE = `
  window.__probe = { commits: [], frames: [], on: false };
  const label = (el) => {
    const ids = (el.getAttribute('aria-labelledby') || '').split(' ').filter(Boolean);
    return ids.map((id) => (document.getElementById(id) || {}).textContent || '').join(' ').trim();
  };
  const sample = () => {
    const mode = document.querySelector('[data-testid="approval-mode-trigger"]');
    const turns = document.querySelector('input[type="number"]');
    return {
      at: performance.now(),
      chat: Boolean(mode),
      mode: mode ? { text: mode.textContent.replace(/\\u00a0/g, '').trim(), disabled: mode.disabled,
        busy: mode.getAttribute('aria-busy') } : null,
      turns: turns ? { value: turns.value, disabled: turns.disabled } : null,
      switches: [...document.querySelectorAll('[data-slot="switch"]')].map((el) => ({
        name: label(el), pending: el.hasAttribute('data-pending'),
        hidden: getComputedStyle(el).visibility === 'hidden', checked: el.getAttribute('aria-checked'),
        disabled: el.disabled,
      })),
    };
  };
  new MutationObserver(() => { if (window.__probe.on) window.__probe.commits.push(sample()); })
    .observe(document.body, { subtree: true, childList: true, characterData: true, attributes: true });
  const tick = () => { if (window.__probe.on) window.__probe.frames.push(sample()); requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
`;

type Switch = {
  name: string;
  pending: boolean;
  hidden: boolean;
  checked: string | null;
  disabled: boolean;
};
type Sample = {
  at: number;
  chat: boolean;
  mode: { text: string; disabled: boolean; busy: string | null } | null;
  turns: { value: string; disabled: boolean } | null;
  switches: Switch[];
};
type Recording = { commits: Sample[]; frames: Sample[] };

/** A switch is on screen at rest: painted, answering, and not waiting for a value. */
const atRest = (control: Switch) => !control.pending && !control.hidden && !control.disabled;

const APP_SWITCHES: Array<[string, string]> = [
  ['Show in menu bar', 'false'],
  ['Show in Dock', 'true'],
  ['Prevent sleep while running', 'false'],
  ['Never open tabs automatically', 'true'],
];

describe('the first frames of a Settings tab switch, in a real browser (V4-R7-2)', () => {
  let browser: Browser | null = null;
  let script = '';
  let noSeedScript = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found (Playwright’s or Google Chrome): skipping the tab frames.');
      return;
    }
    script = await bundle();
    noSeedScript = await bundle(NO_SEED);
    css = PREFLIGHT + readTokens() + primitiveBlocks() + SETTINGS_CSS;
  }, 120_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  /** Settings, open on Models, with the daemon and the main process 600 ms late. */
  async function open(bundleScript = script): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    await tab.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: SETUP });
    await tab.addScriptTag({ content: PROBE });
    await tab.addScriptTag({ content: bundleScript });
    await tab.getByRole('tab', { name: 'Models' }).waitFor();
    return tab;
  }

  /** Click a tab with the real pointer and record every commit and frame for 400 ms. */
  async function visit(tab: Page, name: string): Promise<Recording> {
    await tab.evaluate(() => {
      const probe = (window as unknown as { __probe: Recording & { on: boolean } }).__probe;
      probe.commits = [];
      probe.frames = [];
      probe.on = true;
    });
    await tab.getByRole('tab', { name }).click();
    await tab.waitForTimeout(400);
    return tab.evaluate(() => {
      const probe = (window as unknown as { __probe: Recording & { on: boolean } }).__probe;
      probe.on = false;
      return { commits: probe.commits, frames: probe.frames };
    });
  }

  /** Both views, from the first one that holds the tab's content on. */
  const fromFirst = (recording: Recording, has: (sample: Sample) => boolean) => ({
    commits: recording.commits.slice(recording.commits.findIndex(has)),
    frames: recording.frames.slice(recording.frames.findIndex(has)),
  });

  function expectChatAtRest(sample: Sample) {
    expect(sample.mode, 'the Approval mode select').not.toBeNull();
    expect(sample.mode!.text).toBe('Manual');
    expect(sample.mode!.disabled).toBe(false);
    expect(sample.mode!.busy).toBeNull();
    expect(sample.turns).toEqual({ value: '25', disabled: false });
    const spellcheck = sample.switches.find((control) => control.name === 'Spellcheck');
    expect(spellcheck, 'the Spellcheck switch').toBeDefined();
    expect(atRest(spellcheck!)).toBe(true);
    expect(spellcheck!.checked).toBe('false');
  }

  function expectAppAtRest(sample: Sample) {
    for (const [name, checked] of APP_SWITCHES) {
      const control = sample.switches.find((candidate) => candidate.name === name);
      expect(control, `the "${name}" switch`).toBeDefined();
      expect(atRest(control!), `"${name}" held or unpainted: ${JSON.stringify(control)}`).toBe(
        true
      );
      expect(control!.checked, `"${name}"`).toBe(checked);
    }
    const costs = sample.switches.find((candidate) => candidate.name === 'Show costs');
    expect(costs && atRest(costs), 'the Show costs switch').toBe(true);
  }

  it('paints Chat with its values in the first commit and every frame, Models to Chat to Models to Chat', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Settings has been open for a moment, and the warm-up's answers (600 ms) have landed.
      await tab.waitForTimeout(900);
      for (const round of ['first', 'second']) {
        const chat = fromFirst(await visit(tab, 'Chat'), (sample) => sample.chat);
        expect(chat.commits.length, `${round} visit: commits`).toBeGreaterThan(0);
        expect(chat.frames.length, `${round} visit: frames`).toBeGreaterThan(5);
        for (const sample of chat.commits) expectChatAtRest(sample);
        for (const sample of chat.frames) expectChatAtRest(sample);
        await visit(tab, 'Models');
      }
    } finally {
      await tab.close();
    }
  });

  it('paints every App switch, with its value, in the first commit and every frame', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.waitForTimeout(900);
      for (const round of ['first', 'second']) {
        const app = fromFirst(await visit(tab, 'App'), (sample) => sample.switches.length > 0);
        expect(app.commits.length, `${round} visit: commits`).toBeGreaterThan(0);
        expect(app.frames.length, `${round} visit: frames`).toBeGreaterThan(5);
        for (const sample of app.commits) expectAppAtRest(sample);
        for (const sample of app.frames) expectAppAtRest(sample);
        await visit(tab, 'Models');
      }
    } finally {
      await tab.close();
    }
  });

  it('does not paint the column of switches in steps: every switch is there from the first frame', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.waitForTimeout(900);
      const app = fromFirst(await visit(tab, 'App'), (sample) => sample.switches.length > 0);
      const visible = app.frames.map(
        (sample) => sample.switches.filter((control) => !control.hidden).length
      );
      expect(new Set(visible).size, `visible switches per frame: ${visible.join(' ')}`).toBe(1);
      expect(visible[0]).toBe(APP_SWITCHES.length + 1);
    } finally {
      await tab.close();
    }
  });

  it('starts a later visit from a change the person made, not the answer before it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      await tab.waitForTimeout(900);
      await visit(tab, 'App');
      await tab.getByRole('switch', { name: 'Prevent sleep while running' }).click();
      await visit(tab, 'Models');
      const app = fromFirst(await visit(tab, 'App'), (sample) => sample.switches.length > 0);
      for (const sample of [...app.commits, ...app.frames]) {
        const sleep = sample.switches.find(
          (control) => control.name === 'Prevent sleep while running'
        );
        expect(sleep && atRest(sleep)).toBe(true);
        expect(sleep!.checked).toBe('true');
      }
    } finally {
      await tab.close();
    }
  });

  it('the instrument: a first Chat visit before any answer has landed is held, and the sampler sees it', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      // Within the 600 ms nothing has answered: nothing is known, so nothing may be painted.
      const chat = fromFirst(await visit(tab, 'Chat'), (sample) => sample.chat);
      expect(chat.commits[0].mode!.busy).toBe('true');
      expect(chat.commits[0].mode!.disabled).toBe(true);
      expect(chat.commits[0].mode!.text).toBe('');
      expect(chat.commits[0].turns).toEqual({ value: '', disabled: true });
    } finally {
      await tab.close();
    }
  });

  it('the instrument: a first App visit before any answer has landed holds its switches, unpainted', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const app = fromFirst(await visit(tab, 'App'), (sample) => sample.switches.length > 0);
      const held = app.commits[0].switches.filter((control) => control.pending);
      expect(held.map((control) => control.name).sort()).toEqual(
        [
          'Never open tabs automatically',
          'Prevent sleep while running',
          'Show in Dock',
          'Show in menu bar',
        ].sort()
      );
      expect(held.every((control) => control.hidden)).toBe(true);
    } finally {
      await tab.close();
    }
  });

  it('the instrument: controls that ignore the last answer are held on a later visit too', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open(noSeedScript);
    try {
      await tab.waitForTimeout(900);
      const chat = fromFirst(await visit(tab, 'Chat'), (sample) => sample.chat);
      expect(chat.commits[0].mode!.busy, 'a seedless select is held').toBe('true');
      expect(chat.commits[0].turns!.disabled, 'a seedless Max turns is held').toBe(true);
      await visit(tab, 'Models');
      const app = fromFirst(await visit(tab, 'App'), (sample) => sample.switches.length > 0);
      expect(app.commits[0].switches.some((control) => control.pending)).toBe(true);
    } finally {
      await tab.close();
    }
  });
});
