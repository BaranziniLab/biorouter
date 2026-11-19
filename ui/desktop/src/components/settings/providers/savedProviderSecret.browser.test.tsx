// @vitest-environment node
import { mkdirSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, type Plugin } from 'esbuild';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const here = dirname(fileURLToPath(import.meta.url));
const styles = resolve(here, '../../../styles');
const requireFromDesktop = createRequire(resolve(here, '../../../../package.json'));
const shots = process.env.PROVIDER_SECRET_SHOTS;
const entry = `
import * as React from 'react';
import { createRoot } from 'react-dom/client';
import ProviderConfigurationModal from './modal/ProviderConfiguationModal';
const provider = {
  name: 'google', is_configured: true, provider_type: 'Builtin',
  metadata: { name: 'google', display_name: 'Google Gemini', description: '',
    default_model: '', known_models: [], model_doc_link: '', config_keys: [
      {name: 'GOOGLE_API_KEY', required: true, secret: true, oauth_flow: false},
      {name: 'GOOGLE_HOST', required: false, secret: false, oauth_flow: false},
    ] },
};
createRoot(document.getElementById('root')!).render(
  <ProviderConfigurationModal provider={provider} onClose={() => {}} />
);
`;

const fixtures: Plugin = {
  name: 'saved-key-fixtures',
  setup(builder) {
    builder.onResolve(
      { filter: /(?:ConfigContext|ModelAndProviderContext|providerSecret|providerSetupAccess)$/ },
      ({ path }) => ({
        path: path.split('/').pop()!,
        namespace: 'saved-key-fixture',
      })
    );
    builder.onLoad({ filter: /.*/, namespace: 'saved-key-fixture' }, ({ path }) => {
      const modules: Record<string, string> = {
        ConfigContext: `export const useConfig = () => ({ read: async (key, secret) => secret ? {maskedValue:'••••••••'} : null, upsert: async () => {}, remove: async () => {}, getProviders: async () => [] });`,
        ModelAndProviderContext: `export const useModelAndProvider = () => ({ getCurrentModelAndProvider: async () => ({provider:'other'}) });`,
        providerSecret: `export const SECRET_MASK = '••••••••'; export const loadProviderSecretPreview = async () => 'sk-t••••••••1234'; export const removeProviderSecret = async () => {};`,
        providerSetupAccess: `export const providersAreHostManaged = () => false; export const useProvidersAreHostManaged = () => false;`,
      };
      return { contents: modules[path], loader: 'js' };
    });
  },
};

describe('saved provider key in the real configuration dialog', () => {
  let browser: Browser;
  let script: string;
  let css: string;

  beforeAll(async () => {
    try {
      browser = await chromium.launch();
    } catch {
      browser = await chromium.launch({ channel: 'chrome' });
    }
    const bundle = await build({
      stdin: { contents: entry, loader: 'tsx', resolveDir: here, sourcefile: 'entry.tsx' },
      plugins: [fixtures],
      bundle: true,
      write: false,
      format: 'iife',
      jsx: 'automatic',
      platform: 'browser',
      define: { 'process.env.NODE_ENV': '"production"' },
      loader: { '.css': 'empty', '.svg': 'dataurl', '.png': 'dataurl' },
      logLevel: 'silent',
    });
    script = bundle.outputFiles[0].text;
    const { compile } = await import(requireFromDesktop.resolve('@tailwindcss/node'));
    const { Scanner } = requireFromDesktop('@tailwindcss/oxide');
    const compiler = await compile(readFileSync(resolve(styles, 'main.css'), 'utf8'), {
      base: styles,
      onDependency() {},
    });
    css = compiler.build(new Scanner({}).scanFiles([{ content: script, extension: 'js' }]));
    if (shots) mkdirSync(shots, { recursive: true });
  }, 180_000);

  afterAll(async () => {
    await browser?.close();
  }, 30_000);

  it('keeps the saved key read-only and explains and confirms removal', async () => {
    const page = await browser.newPage({
      viewport: { width: 900, height: 650 },
      deviceScaleFactor: 2,
    });
    await page.setContent(
      `<!doctype html><html><head><style>${css}</style></head><body class="bg-background-canvas"><div id="root"></div></body></html>`
    );
    await page.addScriptTag({ content: script });
    const key = page.locator('#provider-config-GOOGLE_API_KEY');
    await expect.poll(() => key.inputValue()).toBe('sk-t••••••••1234');
    expect(await key.getAttribute('readonly')).not.toBeNull();
    expect(await page.getByRole('button', { name: 'Show Google API key' }).count()).toBe(0);
    await key.press('Backspace');
    expect(await key.inputValue()).toBe('sk-t••••••••1234');
    if (shots) await page.screenshot({ path: resolve(shots, '01-saved-key.png') });

    await key.hover();
    await page
      .locator('[data-slot="tooltip-content"]')
      .filter({ hasText: 'To change this key' })
      .waitFor({ state: 'visible' });
    if (shots) await page.screenshot({ path: resolve(shots, '02-hover-help.png') });

    await page.getByRole('button', { name: 'Remove Google API key' }).click();
    await page
      .locator('[data-slot="tooltip-content"]')
      .filter({ hasText: 'To change this key' })
      .waitFor({ state: 'hidden' });
    await page.getByText(/provider may stop working/).waitFor({ state: 'visible' });
    if (shots) await page.screenshot({ path: resolve(shots, '03-remove-confirm.png') });

    await page.getByRole('button', { name: 'Remove Google API key' }).click();
    await expect.poll(() => key.getAttribute('type')).toBe('password');
    expect(await key.getAttribute('readonly')).toBeNull();
    if (shots) await page.screenshot({ path: resolve(shots, '04-add-new.png') });
    await page.close();
  }, 90_000);
});
