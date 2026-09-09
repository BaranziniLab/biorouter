/**
 * E2E test: Full SPOKEAgent .brxt install flow
 *
 * Connects to the forge-launched Electron app via Chrome DevTools Protocol on
 * port 9222 (enabled by ENABLE_PLAYWRIGHT=true → app.commandLine.appendSwitch).
 * PLAYWRIGHT_BRXT_FILE bypasses the native file dialog in main.ts.
 *
 * Prerequisites:
 *  - biorouterd backend running on port 3000 (start with: just debug-server)
 *  - /tmp/bundle-work/spokeagent.brxt must exist
 */

import { test, expect, chromium } from '@playwright/test';
import { join } from 'path';
import { spawn } from 'child_process';
import type { Page, Browser } from '@playwright/test';

const BRXT_PATH = '/tmp/bundle-work/spokeagent.brxt';
const SPOKE_PASSCODE = 'spoke4ucsf';
const CDP_PORT = 9223; // use non-standard port to avoid colliding with existing apps

let browser: Browser;
let mainWindow: Page;
let forgeProcess: ReturnType<typeof spawn>;

test.describe('SPOKEAgent .brxt install flow', () => {
  test.skip(
    process.env.BIOROUTER_E2E_EXTERNAL !== '1',
    'Set BIOROUTER_E2E_EXTERNAL=1 with its backend and bundle prerequisites.'
  );

  test.setTimeout(300_000); // 5 min — uv sync downloads deps

  test.beforeAll(async () => {
    console.log('Starting electron-forge dev server…');

    forgeProcess = spawn('npm', ['run', 'start-gui'], {
      cwd: join(__dirname, '../..'),
      stdio: 'pipe',
      shell: true,
      env: {
        ...process.env,
        ELECTRON_IS_DEV: '1',
        NODE_ENV: 'development',
        BIOROUTER_ALLOWLIST_BYPASS: 'true',
        BIOROUTER_EXTERNAL_BACKEND: 'true',
        BIOROUTER_EXTERNAL_PORT: '3000',
        // Enables app.commandLine.appendSwitch('remote-debugging-port', CDP_PORT)
        ENABLE_PLAYWRIGHT: 'true',
        PLAYWRIGHT_CDP_PORT: String(CDP_PORT),
        // Bypasses native file dialog for Browse file button
        PLAYWRIGHT_BRXT_FILE: BRXT_PATH,
      },
    });

    forgeProcess.stdout?.on('data', (d) => process.stdout.write('[forge] ' + d));
    forgeProcess.stderr?.on('data', (d) => process.stderr.write('[forge] ' + d));

    // Poll until the CDP port is available (forge builds Vite + Electron starts)
    console.log(`Waiting for Electron CDP port ${CDP_PORT}…`);
    const deadline = Date.now() + 90_000;
    while (Date.now() < deadline) {
      try {
        const b = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
        await b.close();
        break;
      } catch {
        await new Promise((r) => setTimeout(r, 2000));
      }
    }

    // Connect to the running Electron app
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);

    // Find the main renderer page (the React app window)
    const pages = browser.contexts().flatMap((ctx) => ctx.pages());
    mainWindow =
      pages.find((p) => p.url().includes('localhost') || p.url().startsWith('file://')) ?? pages[0];

    await mainWindow.waitForLoadState('domcontentloaded');

    try {
      await mainWindow.waitForLoadState('networkidle', { timeout: 15000 });
    } catch {
      // Backend activity prevents networkidle — continue anyway
    }

    await mainWindow.waitForFunction(() => {
      const root = document.getElementById('root');
      return root && root.children.length > 0;
    });
    await mainWindow.waitForTimeout(2000);

    console.log('App ready. PLAYWRIGHT_BRXT_FILE hook active → returns:', BRXT_PATH);
    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-0-initial.png' });
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
    await mainWindow.waitForTimeout(1000);
    await expect(
      mainWindow.locator('h1:has-text("Extensions"), [data-testid="extensions-heading"]').first()
    ).toBeVisible({ timeout: 10000 });
    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-1-extensions-tab.png' });
    console.log('✓ Extensions tab');
  });

  test('2. Click "Add extension" — modal opens', async () => {
    await mainWindow.click('button:has-text("Add extension")', { timeout: 5000 });
    // Verify the modal dialog is open (avoid strict-mode by targeting the dialog role)
    await expect(mainWindow.locator('[role="dialog"]')).toBeVisible({ timeout: 5000 });
    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-2-modal-open.png' });
    console.log('✓ Modal opened');
  });

  test('3. Click "Browse file…" — PLAYWRIGHT_BRXT_FILE hook returns SPOKEAgent .brxt', async () => {
    // The PLAYWRIGHT_BRXT_FILE env var makes the main-process IPC handler return
    // the preset path without opening a native dialog.
    await mainWindow.click('button:has-text("Browse file")', { timeout: 5000 });

    // Validation runs; manifest preview card should appear
    await expect(mainWindow.locator('text=SPOKEAgent').first()).toBeVisible({ timeout: 20000 });
    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-3-manifest-loaded.png' });
    console.log('✓ Manifest preview loaded — Browse file button works correctly');
  });

  test('4. Manifest info card shows name, version, description', async () => {
    // Use .first() to avoid strict-mode on multiple SPOKEAgent text nodes
    await expect(mainWindow.locator('text=SPOKEAgent').first()).toBeVisible();
    await expect(mainWindow.locator('text=0.1.0').first()).toBeVisible();
    const nextBtn = mainWindow.locator('button:has-text("Next")');
    await expect(nextBtn).toBeEnabled({ timeout: 5000 });
    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-4-manifest-card.png' });
    console.log('✓ Manifest card OK, Next enabled');
  });

  test('5. Click Next → env var configure step shows SPOKEAGENT_PASSCODE', async () => {
    await mainWindow.click('button:has-text("Next")', { timeout: 5000 });
    await expect(mainWindow.locator('text=Configure SPOKEAgent')).toBeVisible({ timeout: 5000 });
    await expect(mainWindow.locator('text=SPOKEAGENT_PASSCODE')).toBeVisible();
    const installBtn = mainWindow.locator('button:has-text("Install Extension")');
    await expect(installBtn).toBeDisabled();
    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-5-env-form.png' });
    console.log('✓ Configure step shown, Install disabled');
  });

  test('6. Enter passcode → Install enabled', async () => {
    await mainWindow.fill('input[type="password"]', SPOKE_PASSCODE);
    const installBtn = mainWindow.locator('button:has-text("Install Extension")');
    await expect(installBtn).toBeEnabled({ timeout: 3000 });
    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-6-passcode-entered.png' });
    console.log('✓ Passcode entered, Install enabled');
  });

  test('7. Install SPOKEAgent (uv sync runs)', async () => {
    await mainWindow.click('button:has-text("Install Extension")');
    await expect(mainWindow.locator('button:has-text("Installing")')).toBeVisible({
      timeout: 5000,
    });
    console.log('Installing… (uv sync may take ~60s)');

    // Wait for modal to close (success) or error banner
    const closed = mainWindow
      .waitForSelector('dialog', { state: 'hidden', timeout: 180000 })
      .catch(() => null);
    const errBanner = mainWindow
      .waitForSelector('.bg-red-50', { timeout: 180000 })
      .catch(() => null);
    await Promise.race([closed, errBanner]);

    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-7-post-install.png' });

    const hasError = await mainWindow
      .locator('.bg-red-50')
      .isVisible()
      .catch(() => false);
    if (hasError) {
      const msg = await mainWindow
        .locator('.bg-red-50')
        .innerText()
        .catch(() => '?');
      throw new Error('Install failed: ' + msg);
    }
    console.log('✓ Install succeeded');
  });

  test('8. SPOKEAgent appears in Extensions list', async () => {
    // After modal closes, the extension appears as "Spokeagent" in the list
    await expect(mainWindow.locator('text=/spokeagent/i').first()).toBeVisible({
      timeout: 10000,
    });
    await mainWindow.screenshot({ path: 'test-results/brxt-spoke-8-extension-listed.png' });
    console.log('✓ SPOKEAgent in Extensions list');
  });
});
