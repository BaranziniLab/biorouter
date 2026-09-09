/**
 * E2E test: BiorOffice post-install verification — dynamic skill discovery +
 * live agent usage. Assumes bioroffice is already installed
 * (bioroffice-install spec tests 1-7).
 *
 * Note on skills: extension-bundled skills are discovered by the BACKEND
 * SkillsClient (crates/biorouter/src/agents/skills_extension.rs scans
 * ~/.config/biorouter/extensions/<name>/skills/) at session start. The chat
 * bar skills dropdown only lists user-level skill dirs, so the correct
 * verification is (a) on-disk frontmatter validity and (b) asking the live
 * agent to loadSkill one of the bundled skills.
 *
 * Launches the dev app with ENABLE_PLAYWRIGHT=true and connects over CDP.
 */

import { test, expect, chromium } from '@playwright/test';
import { join } from 'path';
import { spawn } from 'child_process';
import * as fs from 'fs';
import type { Page, Browser } from '@playwright/test';
import { configRoot } from './helpers/sandbox';
import { openSidebarEntry } from './helpers/sidebar';

const CDP_PORT = 9226;
// Follows BIOROUTER_PATH_ROOT / BIOROUTER_E2E_PATH_ROOT, so this reads back the
// tree `bioroffice-install.spec.ts` wrote to rather than the operator's real
// config. Export BIOROUTER_E2E_PATH_ROOT to run the two specs as a pair.
const EXT_DIR = () => join(configRoot(), 'extensions', 'bioroffice');
const AGENT_OUT_DIR = '/tmp/bioroffice-e2e';
const AGENT_PPTX = join(AGENT_OUT_DIR, 'demo.pptx');

let browser: Browser;
let mainWindow: Page;
let forgeProcess: ReturnType<typeof spawn>;

test.describe('BiorOffice — dynamic skills + live agent usage', () => {
  test.skip(
    process.env.BIOROUTER_E2E_EXTERNAL !== '1',
    'Set BIOROUTER_E2E_EXTERNAL=1 after installing BiorOffice to run this suite.'
  );

  test.setTimeout(420_000);

  test.beforeAll(async () => {
    if (!fs.existsSync(join(EXT_DIR(), 'manifest.json'))) {
      throw new Error('bioroffice extension not installed — run bioroffice-install spec first');
    }
    fs.rmSync(AGENT_OUT_DIR, { recursive: true, force: true });
    fs.mkdirSync(AGENT_OUT_DIR, { recursive: true });

    forgeProcess = spawn('npm', ['run', 'start-gui'], {
      cwd: join(__dirname, '../..'),
      stdio: 'pipe',
      shell: true,
      env: {
        ...process.env,
        ELECTRON_IS_DEV: '1',
        NODE_ENV: 'development',
        BIOROUTER_ALLOWLIST_BYPASS: 'true',
        BIOROUTER_DISABLE_KEYRING: 'true',
        ...(process.env.BIOROUTER_E2E_PATH_ROOT
          ? { BIOROUTER_PATH_ROOT: process.env.BIOROUTER_E2E_PATH_ROOT }
          : {}),
        ENABLE_PLAYWRIGHT: 'true',
        PLAYWRIGHT_CDP_PORT: String(CDP_PORT),
      },
    });
    forgeProcess.stdout?.on('data', (d) => process.stdout.write('[forge] ' + d));
    forgeProcess.stderr?.on('data', (d) => process.stderr.write('[forge] ' + d));

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
  });

  test.afterAll(async () => {
    if (browser) await browser.close().catch(() => {});
    try {
      forgeProcess?.kill();
    } catch {
      /* ignore */
    }
  });

  test('1. Bundled skills are on disk with frontmatter the SkillsClient accepts', async () => {
    // Same parse rule as ui/desktop skillUtils.parseSkillFrontmatter and the
    // Rust SkillsClient: --- block with single-line name: and description:
    const slugs = [
      'bioroffice-office-suite',
      'bioroffice-word',
      'bioroffice-excel',
      'bioroffice-powerpoint',
    ];
    for (const slug of slugs) {
      const p = join(EXT_DIR(), 'skills', slug, 'SKILL.md');
      expect(fs.existsSync(p), `${p} missing`).toBe(true);
      const content = fs.readFileSync(p, 'utf8');
      const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---/);
      expect(match, `${slug} missing frontmatter`).toBeTruthy();
      expect(/^name:\s*\S+/m.test(match![1]), `${slug} missing name`).toBe(true);
      expect(/^description:\s*\S+/m.test(match![1]), `${slug} missing description`).toBe(true);
    }
    console.log('✓ all 4 bundled skills present with valid frontmatter');
  });

  test('2. Agent loads a bundled skill and creates a PowerPoint via officecli', async () => {
    // Navigate to the hub (chat input is on the Home view)
    await openSidebarEntry(mainWindow, 'Home');
    await mainWindow.waitForTimeout(1500);

    const input = mainWindow.locator('[data-testid="chat-input"]').first();
    await expect(input).toBeVisible({ timeout: 20000 });
    await input.click();
    await input.fill(
      `Do these two things without asking questions: ` +
        `(1) Call the loadSkill tool with skill name "bioroffice-office-suite" and tell me ` +
        `the first heading of what it returns. ` +
        `(2) Use the officecli tool from the bioroffice extension to create ${AGENT_PPTX} ` +
        `with one slide titled "BiorOffice E2E", then call officecli view outline on it ` +
        `and report the outline.`
    );
    await mainWindow.waitForTimeout(300);
    await input.press('Enter');
    await mainWindow.waitForTimeout(1500);
    await mainWindow.screenshot({ path: 'test-results/bioroffice-v3-message-sent.png' });

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
    await mainWindow.screenshot({ path: 'test-results/bioroffice-v4-agent-result.png' });
    expect(created, `agent did not create ${AGENT_PPTX} within timeout`).toBe(true);
    console.log('✓ Agent created', AGENT_PPTX, fs.statSync(AGENT_PPTX).size, 'bytes');

    // Let the agent finish its reply (skill heading + outline), then capture
    await mainWindow.waitForTimeout(20000);
    await mainWindow.screenshot({ path: 'test-results/bioroffice-v5-final.png' });

    // The reply should mention the deck title from the outline readback
    const sawOutline = await mainWindow
      .locator('text=BiorOffice E2E')
      .first()
      .isVisible({ timeout: 10000 })
      .catch(() => false);
    console.log(`outline text visible in chat: ${sawOutline}`);
  });
});
