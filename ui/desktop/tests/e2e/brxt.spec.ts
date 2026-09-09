/**
 * E2E tests for the .brxt extension bundle feature.
 *
 * Tests 13-15 and 22: Button layout, drag-and-drop modal, invalid bundle error,
 * env var form, and no-env-var bundle skip.
 *
 * Notes:
 * - Tests that call brxt:install are skipped because they require `uv` to be
 *   installed and an actual Python environment set up.
 * - All .brxt fixtures are real zip archives created on disk via AdmZip so that
 *   the Electron main-process IPC handler (brxt:validate-and-read) can read them.
 */

import { test, expect, Page } from '@playwright/test';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';
import { closeApp, launchApp, type LaunchedApp } from './helpers/app';
import { openSidebarEntry } from './helpers/sidebar';

// AdmZip is a production dependency in ui/desktop/package.json
// eslint-disable-next-line @typescript-eslint/no-require-imports
const AdmZip = require('adm-zip');

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

const VALID_MANIFEST = {
  name: 'testextension',
  display_name: 'Test Extension',
  description: 'A test extension for E2E tests',
  version: '1.0.0',
  entry_point: 'testextension',
  repository: 'https://github.com/example/testextension',
  env_vars: [
    {
      key: 'TEST_KEY',
      required: true,
      auto_propagate: false,
      description: 'A required API key',
      secret: true,
    },
    {
      key: 'OPTIONAL_KEY',
      required: false,
      auto_propagate: true,
      default: 'default-value',
      description: 'An optional setting',
      secret: false,
    },
  ],
};

const VALID_MANIFEST_NO_ENV = {
  name: 'noenvextension',
  display_name: 'No Env Extension',
  description: 'An extension with no env vars',
  version: '0.1.0',
  entry_point: 'noenvextension',
  repository: 'https://github.com/example/noenvextension',
  env_vars: [],
};

/** Create a well-formed .brxt zip archive at `outPath`. */
function createValidBrxt(manifest: object, outPath: string): void {
  const zip = new AdmZip();
  zip.addFile('manifest.json', Buffer.from(JSON.stringify(manifest)));
  zip.addFile('README.md', Buffer.from('# Test extension'));
  zip.addFile('pyproject.toml', Buffer.from('[project]\nname = "test"\nversion = "0.1.0"'));
  zip.addFile('src/__init__.py', Buffer.from(''));
  zip.writeZip(outPath);
}

/** Create an invalid .brxt that is missing manifest.json. */
function createInvalidBrxt(outPath: string): void {
  const zip = new AdmZip();
  zip.addFile('README.md', Buffer.from('# Missing manifest'));
  zip.writeZip(outPath);
}

/** Create a .brxt with bundled skills in skills/<slug>/SKILL.md entries. */
function createValidBrxtWithSkills(
  manifest: object,
  skills: Array<{ slug: string; name: string; description: string }>,
  outPath: string
): void {
  const zip = new AdmZip();
  zip.addFile('manifest.json', Buffer.from(JSON.stringify(manifest)));
  zip.addFile('README.md', Buffer.from('# Test extension'));
  zip.addFile('pyproject.toml', Buffer.from('[project]\nname = "test"\nversion = "0.1.0"'));
  zip.addFile('src/__init__.py', Buffer.from(''));
  for (const skill of skills) {
    zip.addFile(
      `skills/${skill.slug}/SKILL.md`,
      Buffer.from(`---\nname: ${skill.name}\ndescription: ${skill.description}\n---\n\nSkill body.`)
    );
  }
  zip.writeZip(outPath);
}

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

let launched: LaunchedApp;
let page: Page;

/** Temporary directory for fixture files — cleaned up in afterAll. */
let tmpDir: string;

test.describe('BrxtInstallModal — .brxt extension bundle feature', () => {
  test.beforeAll(async () => {
    // Create temp directory for .brxt fixture files
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'brxt-e2e-'));

    // Launch against an isolated config root. This spec used to launch with no
    // BIOROUTER_PATH_ROOT of its own, so on a developer's machine it drove — and
    // wrote to — the real ~/.config/biorouter.
    launched = await launchApp();
    page = launched.page;

    // Allow the app to settle (animations, IPC handshakes, etc.)
    await page.waitForTimeout(3000);
  });

  test.afterAll(async () => {
    await closeApp(launched);
    // Clean up temp fixture files
    if (tmpDir && fs.existsSync(tmpDir)) {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  // -------------------------------------------------------------------------
  // Helper: navigate to the Extensions tab
  // -------------------------------------------------------------------------
  async function goToExtensions(): Promise<void> {
    // Extensions lives behind the sidebar's `Components` disclosure, which a
    // fresh profile renders collapsed — so the nav button is absent from the
    // DOM, not merely hidden. See helpers/sidebar.ts.
    await openSidebarEntry(page, 'Extensions');
    // Wait for the Extensions heading to confirm navigation
    await page.waitForSelector('h1:has-text("Extensions")', {
      timeout: 10000,
      state: 'visible',
    });
    await page.waitForTimeout(500);
  }

  // -------------------------------------------------------------------------
  // Helper: open the BrxtInstallModal via "Add extension" button
  // -------------------------------------------------------------------------
  async function openBrxtModal(): Promise<void> {
    const addBtn = await page.waitForSelector('button:has-text("Add extension")', {
      timeout: 10000,
      state: 'visible',
    });
    await addBtn.click();
    // Wait for the dialog to appear
    await page.waitForSelector('[role="dialog"]', { timeout: 5000, state: 'visible' });
    await page.waitForTimeout(300);
  }

  // -------------------------------------------------------------------------
  // Helper: close modal if open (press Escape)
  // -------------------------------------------------------------------------
  async function closeModalIfOpen(): Promise<void> {
    const dialog = await page.$('[role="dialog"]');
    if (dialog) {
      await page.keyboard.press('Escape');
      await page
        .waitForSelector('[role="dialog"]', { state: 'hidden', timeout: 3000 })
        .catch(() => {});
    }
  }

  // ---------------------------------------------------------------------------
  // Test 1: Button layout in the Extensions tab header
  // ---------------------------------------------------------------------------
  test('Extensions tab has correct button layout — Add extension, Browse extensions, Add custom extension', async () => {
    await goToExtensions();

    // Verify all three buttons are visible (UI copy is sentence case)
    const addExtBtn = page.locator('button:has-text("Add extension")').first();
    const browseBtn = page.locator('button:has-text("Browse extensions")').first();
    const addCustomBtn = page.locator('button:has-text("Add custom extension")').first();

    await expect(addExtBtn).toBeVisible();
    await expect(browseBtn).toBeVisible();
    await expect(addCustomBtn).toBeVisible();

    // Verify DOM order in the header button row. The strip is
    // `PageHeader`'s `.biorouter-settings-control-strip`; the `.flex.gap-3.mt-5`
    // this used to name is the hand-rolled row that component replaced, and
    // `styles/measures.test.ts` now bans it at the source.
    const buttonTexts: string[] = await page.$$eval(
      '.biorouter-settings-control-strip button',
      (btns: Element[]) => btns.map((b) => b.textContent?.trim() ?? '')
    );
    const addIdx = buttonTexts.findIndex((t) => t.includes('Add extension'));
    const browseIdx = buttonTexts.findIndex((t) => t.includes('Browse extensions'));
    const customIdx = buttonTexts.findIndex((t) => t.includes('Add custom extension'));

    expect(addIdx).toBeGreaterThanOrEqual(0);
    expect(browseIdx).toBeGreaterThan(addIdx);
    expect(customIdx).toBeGreaterThan(browseIdx);

    await page.screenshot({ path: 'test-results/brxt-button-layout.png' });
  });

  // ---------------------------------------------------------------------------
  // Test 2: Selecting a valid .brxt file opens the modal with manifest info
  // ---------------------------------------------------------------------------
  test('Selecting a valid .brxt file shows manifest info card in modal', async () => {
    await goToExtensions();
    await openBrxtModal();

    // Create valid .brxt fixture
    const brxtPath = path.join(tmpDir, 'valid.brxt');
    createValidBrxt(VALID_MANIFEST, brxtPath);

    // The BrxtInstallModal renders a hidden <input type="file" accept=".brxt">.
    // We target it with setInputFiles, which triggers the onChange handler.
    const fileInput = page.locator('input[type="file"][accept=".brxt"]');
    await fileInput.setInputFiles(brxtPath);

    // Wait for the manifest preview card to appear
    await page.waitForSelector('text=Detected from bundle', { timeout: 10000 });

    // Scope assertions to the modal to avoid strict-mode violations
    const dialog = page.locator('[role="dialog"]');

    // Verify display_name and version are shown
    await expect(dialog.getByText('Test Extension', { exact: true })).toBeVisible();
    await expect(dialog.locator('text=1.0.0')).toBeVisible();

    // The modal title should still say "Add extension" (step === 'drop')
    await expect(page.locator('[role="dialog"] [data-slot="dialog-title"]')).toHaveText(
      /Add extension/i
    );

    await page.screenshot({ path: 'test-results/brxt-valid-manifest.png' });

    await closeModalIfOpen();
  });

  // ---------------------------------------------------------------------------
  // Test 3: Invalid .brxt (missing manifest.json) shows error banner
  // ---------------------------------------------------------------------------
  test('Selecting an invalid .brxt shows an error banner', async () => {
    await goToExtensions();
    await openBrxtModal();

    // Create invalid .brxt fixture (no manifest.json)
    const invalidPath = path.join(tmpDir, 'invalid.brxt');
    createInvalidBrxt(invalidPath);

    const fileInput = page.locator('input[type="file"][accept=".brxt"]');
    await fileInput.setInputFiles(invalidPath);

    // The error banner offers the recovery action as well as the validation text.
    const debugButton = page.getByRole('button', {
      name: /Open a new chat with this error/i,
    });
    await expect(debugButton).toBeVisible({ timeout: 10000 });
    const errorBanner = debugButton.locator(
      'xpath=ancestor::div[contains(@class, "rounded-lg")][1]'
    );
    const errorText = await errorBanner.textContent();
    expect(errorText).toBeTruthy();

    // The error should mention missing manifest.json or similar validation failure
    const lowerError = (errorText ?? '').toLowerCase();
    const mentionsMissing =
      lowerError.includes('manifest') ||
      lowerError.includes('missing') ||
      lowerError.includes('invalid') ||
      lowerError.includes('bundle') ||
      lowerError.includes('valid');
    expect(mentionsMissing).toBe(true);

    await page.screenshot({ path: 'test-results/brxt-invalid-error.png' });

    await closeModalIfOpen();
  });

  // ---------------------------------------------------------------------------
  // Test 4: Env var form — clicking "Next: Configure →" shows required/optional fields
  // ---------------------------------------------------------------------------
  test('Next: Configure → shows required field with asterisk and optional with default', async () => {
    await goToExtensions();
    await openBrxtModal();

    const brxtPath = path.join(tmpDir, 'env-vars.brxt');
    createValidBrxt(VALID_MANIFEST, brxtPath);

    const fileInput = page.locator('input[type="file"][accept=".brxt"]');
    await fileInput.setInputFiles(brxtPath);

    // Wait for manifest preview to confirm successful read
    await page.waitForSelector('text=Detected from bundle', { timeout: 10000 });

    // Click "Next: Configure →"
    const nextBtn = page.locator('button:has-text("Next: Configure")');
    await expect(nextBtn).toBeEnabled({ timeout: 5000 });
    await nextBtn.click();

    // Step 2: configure — modal title should change
    await page.waitForSelector('[role="dialog"] [data-slot="dialog-title"]:has-text("Configure")', {
      timeout: 5000,
    });

    // Required field label should have an asterisk. Color is covered by theme tests.
    const requiredLabel = page.locator('label:has-text("TEST_KEY")');
    await expect(requiredLabel).toBeVisible();
    await expect(requiredLabel).toContainText('*');

    // "Install Extension" button should be disabled because required field is empty
    const installBtn = page.getByRole('button', { name: /Install extension/i });
    await expect(installBtn).toBeDisabled({ timeout: 3000 });

    // The optional var (OPTIONAL_KEY) should have a "Show N optional variables" toggle
    const showOptionalToggle = page
      .locator('button:has-text("Show")')
      .or(page.locator('button:has-text("optional variable")'));
    await expect(showOptionalToggle).toBeVisible();

    await page.screenshot({ path: 'test-results/brxt-configure-step.png' });

    await closeModalIfOpen();
  });

  // ---------------------------------------------------------------------------
  // Test 5: Bundle with no env vars — clicking Next triggers install immediately
  //         (no configure step shown)
  //
  // NOTE: The actual install step calls brxt:install which spawns `uv sync`.
  // We only verify that the UI skips the configure step and moves toward install.
  // The install itself is skipped to avoid requiring `uv` in the test environment.
  // ---------------------------------------------------------------------------
  test.skip('No-env-var bundle skips configure step and goes directly to install (requires uv)', async () => {
    await goToExtensions();
    await openBrxtModal();

    const brxtPath = path.join(tmpDir, 'no-env.brxt');
    createValidBrxt(VALID_MANIFEST_NO_ENV, brxtPath);

    const fileInput = page.locator('input[type="file"][accept=".brxt"]');
    await fileInput.setInputFiles(brxtPath);

    // Wait for manifest preview
    await page.waitForSelector('text=Detected from bundle', { timeout: 10000 });

    // Click "Next: Configure →" — for a no-env bundle this calls handleInstall directly
    const nextBtn = page.locator('button:has-text("Next: Configure")');
    await expect(nextBtn).toBeEnabled({ timeout: 5000 });
    await nextBtn.click();

    // The configure step should NOT appear — the dialog title should NOT change to "Configure"
    // Instead we should see "Installing…" spinner or the dialog closes
    await page.waitForTimeout(1000);

    const configureTitle = await page
      .$('[role="dialog"] [data-slot="dialog-title"]:has-text("Configure")')
      .catch(() => null);
    expect(configureTitle).toBeNull();

    await page.screenshot({ path: 'test-results/brxt-no-env-install.png' });

    await closeModalIfOpen();
  });

  // ---------------------------------------------------------------------------
  // Bonus: No-env-var bundle — verify configure step is skipped (without install)
  // We check that handleNext would call handleInstall immediately by confirming
  // the "Configure" title never appears after clicking Next.
  // ---------------------------------------------------------------------------
  test.skip('No-env-var bundle: clicking Next does not show configure step (requires uv)', async () => {
    await goToExtensions();
    await openBrxtModal();

    const brxtPath = path.join(tmpDir, 'no-env-check.brxt');
    createValidBrxt(VALID_MANIFEST_NO_ENV, brxtPath);

    const fileInput = page.locator('input[type="file"][accept=".brxt"]');
    await fileInput.setInputFiles(brxtPath);

    // Wait for manifest preview
    await page.waitForSelector('text=Detected from bundle', { timeout: 10000 });
    await page.waitForTimeout(300);

    // Confirm the manifest shows 0 required env vars
    const infoCard = page.locator('text=0 required env var');
    await expect(infoCard).toBeVisible();

    // Click Next — this will trigger handleInstall internally (no configure step).
    // We do NOT wait for install completion (requires uv), but we verify the
    // "Configure" title never appears, confirming the step was skipped.
    const nextBtn = page.locator('button:has-text("Next: Configure")');
    await expect(nextBtn).toBeEnabled({ timeout: 5000 });
    await nextBtn.click();

    // Give the UI a moment to react
    await page.waitForTimeout(800);

    // The dialog either shows "Installing…" or closes — it must NOT show "Configure …"
    const configTitle = await page
      .$('[data-slot="dialog-title"]:has-text("Configure")')
      .catch(() => null);
    expect(configTitle).toBeNull();

    await page.screenshot({ path: 'test-results/brxt-no-env-skips-configure.png' });

    await closeModalIfOpen();
  });

  // ---------------------------------------------------------------------------
  // Test: Bundle with skills — verify skills preview section is shown
  // ---------------------------------------------------------------------------
  test('Bundle with skills shows "Skills included" section in manifest preview', async () => {
    await goToExtensions();
    await openBrxtModal();

    const brxtPath = path.join(tmpDir, 'with-skills.brxt');
    createValidBrxtWithSkills(
      VALID_MANIFEST_NO_ENV,
      [
        {
          slug: 'cdw-query-cohorts',
          name: 'cdw-query-cohorts',
          description: 'Build patient cohorts from CDW data',
        },
        {
          slug: 'cdw-explore-schema',
          name: 'cdw-explore-schema',
          description: 'Explore the CDW schema',
        },
      ],
      brxtPath
    );

    const fileInput = page.locator('input[type="file"][accept=".brxt"]');
    await fileInput.setInputFiles(brxtPath);

    // Wait for manifest preview to appear
    await page.waitForSelector('text=Detected from bundle', { timeout: 10000 });

    // The skills count should appear in the metadata line: "2 skills"
    await expect(page.locator('text=2 skills')).toBeVisible({ timeout: 5000 });

    // The "Skills included" section header should be visible
    await expect(page.locator('text=Skills included')).toBeVisible({ timeout: 5000 });

    // Both skill names should be listed
    await expect(page.locator('text=cdw-query-cohorts')).toBeVisible();
    await expect(page.locator('text=cdw-explore-schema')).toBeVisible();

    await page.screenshot({ path: 'test-results/brxt-skills-preview.png' });

    await closeModalIfOpen();
  });
});
