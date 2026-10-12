// @vitest-environment node
/**
 * The emphasised words in privacy prose are 600, not 700 (vision V4-R7-6; principle 3), measured in
 * Chromium with the whole stylesheet.
 *
 * A bare `<strong>` takes Tailwind preflight's `font-weight: bolder`, which computes to 700 on
 * Arial, and the type system draws only 400 and 600. Arial draws both as bold, so nothing moves on
 * screen and a screenshot cannot tell them apart; the computed weight is the whole of the defect.
 * `firstRunPrivacyNotice.browser.test.tsx` measures the day-one notice (FR-R2-8); this file measures
 * the other privacy surfaces that carry a `<strong>`:
 *
 * - `DisclosureProse`, the served long form shown by the non-private model disclosure and by
 *   Settings, App, Privacy (the clauses that say what is NOT protected),
 * - `PrivacyTiersOffNote`, the standing statement above a composer while the tiers are off,
 * - `CrossAffiliationAcceptCard`'s strict-policy sentence.
 *
 * jsdom has no preflight and computes no cascade, so `DisclosureProse.test.tsx` and the component
 * tests can only say which class is on the element. This file bundles the three real components
 * with esbuild (the config context and the cross-institution helpers stubbed), compiles
 * `styles/main.css` with Tailwind over the classes the bundle uses, and reads `getComputedStyle`.
 *
 * ⚠ The instrument is checked first: the page also carries one bare `<strong>`, and the first case
 * asserts it reads 700. A stylesheet without preflight would make every other case pass for the
 * wrong reason.
 *
 * To see these cases fail on another version of the components, point `EMPHASIS_SOURCE_DIR` at a
 * directory holding that version's `DisclosureProse.tsx`, `PrivacyTiersOffNote.tsx` and
 * `CrossAffiliationAcceptCard.tsx`: they are loaded in place of the ones beside this file, their
 * imports resolved from this directory.
 */
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import postcss from 'postcss';
import { chromium, type Browser, type Page } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { fontScale } from '../../../vite-plugins/fontScale.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const STYLES = resolve(here, '../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../package.json'));

const COMPONENTS = ['DisclosureProse', 'PrivacyTiersOffNote', 'CrossAffiliationAcceptCard'];
const COMPONENT_FILE = new RegExp(`(${COMPONENTS.join('|')})\\.tsx$`);

const STUBS: { filter: RegExp; contents: string }[] = [
  {
    // The switch is off, with no record: the note's plainest headline.
    filter: /^\.\.\/ConfigContext$/,
    contents: `
      export const usePrivacyTiersEnabled = () => false;
      export const usePrivacyTiersRecord = () => null;`,
  },
  {
    filter: /^\.\.\/\.\.\/utils\/crossAffiliation$/,
    contents: `
      export const GRANT_SCOPE_COPY =
        'Approving records your acceptance for this chat, this extension and this model\\u2019s institution only.';
      export const readMixingMode = async () => 'strict';
      export const acceptCrossAffiliationFlow = async () => '';`,
  },
];

const stubPlugin: Plugin = {
  name: 'stubs',
  setup(b) {
    for (const { filter, contents } of STUBS) {
      b.onResolve({ filter }, (args) =>
        COMPONENT_FILE.test(args.importer) || args.namespace === 'source-override'
          ? { path: args.path, namespace: 'stub', pluginData: contents }
          : undefined
      );
    }
    b.onLoad({ filter: /.*/, namespace: 'stub' }, (args) => ({
      contents: args.pluginData as string,
      loader: 'js',
    }));
    const dir = process.env.EMPHASIS_SOURCE_DIR;
    if (dir) {
      b.onResolve({ filter: new RegExp(`^\\./(${COMPONENTS.join('|')})$`) }, (args) => ({
        path: resolve(dir, `${args.path.slice(2)}.tsx`),
        namespace: 'source-override',
      }));
      b.onLoad({ filter: /.*/, namespace: 'source-override' }, (args) => ({
        contents: readFileSync(args.path, 'utf8'),
        resolveDir: here,
        loader: 'tsx',
      }));
    }
    b.onLoad({ filter: /\.css$/ }, () => ({ contents: '', loader: 'js' }));
  },
};

/** `COPY_LONG` from `crates/biorouter/src/privacy/disclosure.rs`, as the daemon serves it. */
const COPY_LONG = [
  'It is not HIPAA-compliant, is not hosted on-premise, and does not run on this machine. It can read **files on this computer**. Anything a chat on this model can reach, it can send there: the contents of your working directory, and whatever a command you approve prints.',
  "Biorouter does stop three things: this model cannot read another chat's transcript, cannot read a knowledge base marked private, and cannot use an extension marked private or switch this chat to a private model to reach one.",
  "It **does not** stop it reading ordinary files on this computer through the shell, including files an earlier private chat wrote outside Biorouter's own storage. If the work involves patient data, use a local model or an institutional one.",
].join('\n\n');

const ENTRY = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { DisclosureProse } from './DisclosureProse';
import { PrivacyTiersOffNote } from './PrivacyTiersOffNote';
import { CrossAffiliationAcceptCard } from './CrossAffiliationAcceptCard';

const COPY_LONG = ${JSON.stringify(COPY_LONG)};

createRoot(document.getElementById('root')!).render(
  <MemoryRouter>
    <div id="disclosure" className="space-y-3 p-4 text-body text-text-muted">
      <DisclosureProse text={COPY_LONG} paragraphClassName="min-w-0 [overflow-wrap:anywhere]" />
    </div>
    <div id="off-note" className="p-4">
      <PrivacyTiersOffNote />
    </div>
    <div id="accept" className="p-4">
      <CrossAffiliationAcceptCard sessionId="chat_7" extension="ucsfomopagent" />
    </div>
    <p id="control" className="p-4 text-body">
      <strong id="bare">A bare strong, the instrument check</strong>
    </p>
  </MemoryRouter>
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

type Weights = {
  strong: { text: string | null; weight: string }[];
  /** The heaviest computed weight of anything inside the container. */
  max: number;
};

describe('Privacy prose emphasis is 600, never 700 (V4-R7-6)', () => {
  let browser: Browser | null = null;
  let script = '';
  let css = '';

  beforeAll(async () => {
    browser = await launchChromium();
    if (!browser) {
      console.warn('No Chromium found: skipping the privacy emphasis weight tests.');
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
      loader: { '.svg': 'dataurl', '.png': 'dataurl' },
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
    const tailwind = compiler.build(
      new Scanner({}).scanFiles([{ content: script, extension: 'js' }])
    );
    css = (await postcss([fontScale()]).process(tailwind, { from: undefined })).css;
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 120_000);

  async function open(dark = false): Promise<Page> {
    const tab = await browser!.newPage({ viewport: { width: 900, height: 900 } });
    await tab.setContent(
      `<!doctype html><html${dark ? ' class="dark"' : ''}><head><style>${css}</style></head>` +
        `<body style="margin:0;background:var(--background-default)"><div id="root"></div></body></html>`
    );
    await tab.addScriptTag({ content: script });
    // The card renders once the policy is read, so it is the last thing to arrive.
    await tab.waitForSelector('[data-testid="cross-affiliation-strict-notice"]');
    await tab.waitForSelector('[data-testid="privacy-tiers-off-note"]');
    return tab;
  }

  const weights = (tab: Page, selector: string): Promise<Weights> =>
    tab.evaluate((sel) => {
      const root = document.querySelector(sel)!;
      const strong = [...root.querySelectorAll('strong')].map((el) => ({
        text: el.textContent,
        weight: getComputedStyle(el).fontWeight,
      }));
      const all = [root, ...root.querySelectorAll('*')].map((el) =>
        parseInt(getComputedStyle(el).fontWeight, 10)
      );
      return { strong, max: Math.max(...all) };
    }, selector);

  it('reads a bare strong as 700, so the other cases are measuring something', async (ctx) => {
    if (!browser) return ctx.skip();
    const tab = await open();
    try {
      const control = await weights(tab, '#control');
      expect(control.strong.map((s) => s.weight)).toEqual(['700']);
    } finally {
      await tab.close();
    }
  });

  for (const dark of [false, true]) {
    const mode = dark ? 'dark' : 'light';

    it(`draws the disclosure's two clauses at 600 (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(dark);
      try {
        const w = await weights(tab, '#disclosure');
        expect(w.strong.map((s) => s.text)).toEqual(['files on this computer', 'does not']);
        for (const { text, weight } of w.strong) expect(weight, text ?? '').toBe('600');
        expect(w.max).toBeLessThanOrEqual(600);
      } finally {
        await tab.close();
      }
    });

    it(`draws the tiers-off headline at 600 (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(dark);
      try {
        const w = await weights(tab, '#off-note');
        expect(w.strong.map((s) => s.text)).toEqual(['Privacy tiers are off.']);
        for (const { text, weight } of w.strong) expect(weight, text ?? '').toBe('600');
        expect(w.max).toBeLessThanOrEqual(600);
      } finally {
        await tab.close();
      }
    });

    it(`draws the strict-policy word at 600 (${mode})`, async (ctx) => {
      if (!browser) return ctx.skip();
      const tab = await open(dark);
      try {
        const w = await weights(tab, '#accept');
        expect(w.strong.map((s) => s.text)).toEqual(['strict']);
        for (const { text, weight } of w.strong) expect(weight, text ?? '').toBe('600');
        expect(w.max).toBeLessThanOrEqual(600);
      } finally {
        await tab.close();
      }
    });
  }
});
