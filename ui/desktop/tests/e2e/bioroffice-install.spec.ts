/**
 * E2E test: Full BiorOffice .brxt install flow + real agent usage.
 *
 * Launches the dev app (electron-forge) with ENABLE_PLAYWRIGHT=true and
 * connects over the Chrome DevTools Protocol. PLAYWRIGHT_BRXT_FILE bypasses
 * the native file dialog. The install runs the REAL pipeline: unzip to
 * ~/.config/biorouter/extensions/bioroffice/, uv sync, config.yaml
 * registration — then a real chat session asks the agent to create a .pptx
 * via the officecli MCP tool and the test polls the filesystem for the file.
 *
 * Prerequisites:
 *  - /Users/wgu/Desktop/BiorOffice/dist/bioroffice.brxt exists
 *  - No bioroffice extension currently installed (clean state)
 *  - A working LLM provider in ~/.config/biorouter/config.yaml (for test 9)
 */

import { test, expect, chromium } from '@playwright/test';
import { join } from 'path';
import { spawn } from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import type { Page, Browser } from '@playwright/test';

const BRXT_PATH = '/Users/wgu/Desktop/BiorOffice/dist/bioroffice.brxt';
const CDP_PORT = 9224;
const EXT_DIR = join(os.homedir(), '.config', 'biorouter', 'extensions', 'bioroffice');
const CONFIG_YAML = join(os.homedir(), '.config', 'biorouter', 'config.yaml');
const AGENT_OUT_DIR = '/tmp/bioroffice-e2e';
const AGENT_PPTX = join(AGENT_OUT_DIR, 'demo.pptx');

let browser: Browser;
let mainWindow: Page;
let forgeProcess: ReturnType<typeof spawn>;

test.describe('BiorOffice .brxt — real install + agent usage', () => {
  test.skip(
    process.env.BIOROUTER_E2E_EXTERNAL !== '1' || !fs.existsSync(BRXT_PATH),
    'Set BIOROUTER_E2E_EXTERNAL=1 and build BiorOffice to run this destructive live suite.'
  );

  test.setTimeout(420_000);

  test.beforeAll(async () => {
    if (!fs.existsSync(BRXT_PATH)) {
      throw new Error(`bioroffice.brxt not found at ${BRXT_PATH} — run scripts/build_brxt.sh first`);
    }
    fs.rmSync(AGENT_OUT_DIR, { recursive: true, force: true });
    fs.mkdirSync(AGENT_OUT_DIR, { recursive: true });

    console.log('Starting electron-forge dev server (embedded backend)…');
    forgeProcess = spawn('npm', ['run', 'start-gui'], {
      cwd: join(__dirname, '../..'),
      stdio: 'pipe',
      shell: true,
      env: {
        ...process.env,
        ELECTRON_IS_DEV: '1',
        NODE_ENV: 'development',
        BIOROUTER_ALLOWLIST_BYPASS: 'true',
        ENABLE_PLAYWRIGHT: 'true',
        PLAYWRIGHT_CDP_PORT: String(CDP_PORT),
        PLAYWRIGHT_BRXT_FILE: BRXT_PATH,
      },
    });
    forgeProcess.stdout?.on('data', (d) => process.stdout.write('[forge] ' + d));
    forgeProcess.stderr?.on('data', (d) => process.stderr.write('[forge] ' + d));

    console.log(`Waiting for Electron CDP port ${CDP_PORT}…`);
    const deadline = Date.now() + 120_000;
    while (Date.now() < deadline) {
      try {
        const b = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
        await b.close();
        break;
      } catch {
        await new Promise((r) => setTimeout(r, 2000));
      }
    }

    browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
    const pages = browser.contexts().flatMap((ctx) => ctx.pages());
    mainWindow =
      pages.find((p) => p.url().includes('localhost') || p.url().startsWith('file://')) ?? pages[0];

    await mainWindow.waitForLoadState('domcontentloaded');
    await mainWindow.waitForFunction(() => {
      const root = document.getElementById('root');
      return root && root.children.length > 0;
    });
    await mainWindow.waitForTimeout(3000);
    await mainWindow.screenshot({ path: 'test-results/bioroffice-0-initial.png' });
  });

  test.afterAll(async () => {
    if (browser) await browser.close().catch(() => {});
    try {
      forgeProcess?.kill();
    } catch {
      /* ignore */
    }
  });

  test('1. Navigate to Extensions tab', async () => {
    const sidebarBtn = mainWindow.locator('[data-testid="sidebar-extensions-button"]');
    if (await sidebarBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
      await sidebarBtn.click();
    } else {
      await mainWindow.click('button:has-text("Extensions")', { timeout: 10000 });
    }
    await expect(
      mainWindow.locator('h1:has-text("Extensions"), [data-testid="extensions-heading"]').first()
    ).toBeVisible({ timeout: 10000 });
    await mainWindow.screenshot({ path: 'test-results/bioroffice-1-extensions.png' });
  });

  test('2. Open Add extension modal and load bioroffice.brxt', async () => {
    await mainWindow.click('button:has-text("Add extension")', { timeout: 5000 });
    await expect(mainWindow.locator('[role="dialog"]')).toBeVisible({ timeout: 5000 });
    // PLAYWRIGHT_BRXT_FILE hook returns our path without a native dialog
    await mainWindow.click('button:has-text("Browse file")', { timeout: 5000 });
    await expect(mainWindow.locator('text=BiorOffice').first()).toBeVisible({ timeout: 20000 });
    await mainWindow.screenshot({ path: 'test-results/bioroffice-2-manifest.png' });
  });

  test('3. Manifest preview shows name, version, and 4 bundled skills', async () => {
    const dialog = mainWindow.locator('[role="dialog"]');
    await expect(dialog.locator('text=BiorOffice').first()).toBeVisible();
    await expect(dialog.locator('text=1.0.0').first()).toBeVisible();
    await expect(dialog.locator('text=Skills included').first()).toBeVisible({ timeout: 5000 });
    await expect(dialog.locator('text=bioroffice-office-suite').first()).toBeVisible();
    await mainWindow.screenshot({ path: 'test-results/bioroffice-3-skills-preview.png' });
  });

  test('4. Proceed to configure step — Install enabled (no required env vars)', async () => {
    const nextBtn = mainWindow.locator('button:has-text("Next")');
    if (await nextBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await nextBtn.click();
    }
    const installBtn = mainWindow.locator('button:has-text("Install Extension")');
    await expect(installBtn).toBeEnabled({ timeout: 10000 });
    await mainWindow.screenshot({ path: 'test-results/bioroffice-4-configure.png' });
  });

  test('5. Install BiorOffice (unzip + uv sync + config registration)', async () => {
    await mainWindow.click('button:has-text("Install Extension")');
    // Wait for the modal to close (success) or an error banner
    const closed = mainWindow
      .waitForSelector('[role="dialog"]', { state: 'hidden', timeout: 180000 })
      .catch(() => null);
    const errBanner = mainWindow
      .waitForSelector('.bg-red-50', { timeout: 180000 })
      .catch(() => null);
    await Promise.race([closed, errBanner]);
    await mainWindow.screenshot({ path: 'test-results/bioroffice-5-post-install.png' });

    const hasError = await mainWindow.locator('.bg-red-50').isVisible().catch(() => false);
    if (hasError) {
      const msg = await mainWindow.locator('.bg-red-50').innerText().catch(() => '?');
      throw new Error('Install failed: ' + msg);
    }
  });

  test('6. Extension files installed on disk and registered in config.yaml', async () => {
    expect(fs.existsSync(join(EXT_DIR, 'manifest.json'))).toBe(true);
    expect(fs.existsSync(join(EXT_DIR, 'bin', 'officecli-mac-arm64'))).toBe(true);
    expect(fs.existsSync(join(EXT_DIR, '.venv'))).toBe(true); // uv sync ran
    expect(
      fs.existsSync(join(EXT_DIR, 'skills', 'bioroffice-office-suite', 'SKILL.md'))
    ).toBe(true);
    const config = fs.readFileSync(CONFIG_YAML, 'utf8');
    expect(config).toContain('bioroffice');
  });

  test('7. BiorOffice appears in the Extensions list', async () => {
    await expect(mainWindow.locator('text=/bioroffice/i').first()).toBeVisible({ timeout: 15000 });
    await mainWindow.screenshot({ path: 'test-results/bioroffice-7-listed.png' });
  });

  test('8. Bundled skills appear in the Skills tab', async () => {
    const btn = mainWindow.locator('[data-testid="sidebar-skills-button"]');
    if (await btn.isVisible({ timeout: 5000 }).catch(() => false)) {
      await btn.click();
    } else {
      await mainWindow.locator('text=Skills').first().click();
    }
    await mainWindow.waitForSelector('h1:has-text("Skills")', { timeout: 10000 });
    await mainWindow.waitForTimeout(1500);
    await expect(mainWindow.locator('text=bioroffice-office-suite').first()).toBeVisible({
      timeout: 15000,
    });
    await expect(mainWindow.locator('text=bioroffice-word').first()).toBeVisible();
    await expect(mainWindow.locator('text=bioroffice-excel').first()).toBeVisible();
    await expect(mainWindow.locator('text=bioroffice-powerpoint').first()).toBeVisible();
    await mainWindow.screenshot({ path: 'test-results/bioroffice-8-skills.png' });
  });

  test('9. Agent creates a PowerPoint via the officecli tool in a real chat', async () => {
    // Navigate home / new chat
    const homeBtn = mainWindow.locator('[data-testid="sidebar-home-button"]');
    if (await homeBtn.isVisible({ timeout: 3000 }).catch(() => false)) {
      await homeBtn.click();
    } else {
      const chatBtn = mainWindow.locator('[data-testid="sidebar-chat-button"]');
      if (await chatBtn.isVisible({ timeout: 3000 }).catch(() => false)) await chatBtn.click();
    }
    await mainWindow.waitForTimeout(2000);

    const input = mainWindow.locator('[data-testid="chat-input"]');
    await expect(input).toBeVisible({ timeout: 20000 });
    await input.click();
    await input.fill(
      `Use the officecli tool (bioroffice extension) to create ${AGENT_PPTX} ` +
        `and add one slide with title "BiorOffice E2E". Do not ask questions, just do it, ` +
        `then verify with a view outline call and tell me the outline.`
    );
    await input.press('Enter');
    await mainWindow.screenshot({ path: 'test-results/bioroffice-9a-message-sent.png' });

    // Poll for the file; click any tool-confirmation Allow buttons that appear
    const deadline = Date.now() + 300_000;
    let created = false;
    while (Date.now() < deadline) {
      if (fs.existsSync(AGENT_PPTX) && fs.statSync(AGENT_PPTX).size > 1000) {
        created = true;
        break;
      }
      for (const label of ['Always Allow', 'Allow Once', 'Allow']) {
        const allowBtn = mainWindow.locator(`button:has-text("${label}")`).first();
        if (await allowBtn.isVisible({ timeout: 200 }).catch(() => false)) {
          await allowBtn.click().catch(() => {});
          console.log(`clicked "${label}" tool confirmation`);
        }
      }
      await mainWindow.waitForTimeout(2000);
    }
    await mainWindow.screenshot({ path: 'test-results/bioroffice-9b-agent-result.png' });
    expect(created, `agent did not create ${AGENT_PPTX} within timeout`).toBe(true);
    console.log('✓ Agent created', AGENT_PPTX, fs.statSync(AGENT_PPTX).size, 'bytes');
  });
});
