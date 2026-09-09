/**
 * End-to-end smoke of the desktop app: chat against a real provider, and the
 * custom-extension flow with a real MCP server.
 *
 * ⚠ **This drives the SEED's configured provider; it does not choose one.** The
 * spec used to walk Databricks onboarding — Settings → Models → "Reset provider
 * and model", then the provider grid, then Launch. That walk cannot pass here on
 * its own terms: it *starts* by unconfiguring the working provider, and then
 * needs a Databricks host and token that the seed does not carry and that no
 * amount of repair can invent. Asserting the seed's own provider is what makes
 * this file runnable with no secret beyond the seed's — which is the bar for
 * every spec in this directory.
 *
 * The corollary is that the seed is part of the fixture: it configures
 * `versa_azure` (GPT-5.5) with `BIOROUTER_MODE: auto`, so the app opens straight
 * into chat and the turns below are real model calls. Budget accordingly —
 * `test.setTimeout` is raised on each of them.
 */

import { test, expect, type Page } from '@playwright/test';
import { join } from 'path';
import { closeApp, launchApp, type LaunchedApp } from './helpers/app';
import { openSidebarEntry } from './helpers/sidebar';

// eslint-disable-next-line @typescript-eslint/no-require-imports, @typescript-eslint/no-var-requires
const { runningQuotes } = require('./basic-mcp');

/**
 * A warm turn: the provider round-trip plus whatever tools Auto mode reaches for.
 * Measured on the seeded sandbox: 5-10 s for a one-word answer.
 */
const LIVE_TURN_TIMEOUT_MS = 180_000;
/**
 * The FIRST turn of a session, which gets a far larger budget than a warm one.
 *
 * ⚠ Not padding — a measurement. A session's first turn intermittently stalls for
 * MINUTES between `agent action 1/100 this turn` (`agents/agent.rs`) and the
 * provider call, with nothing logged in between; the provider call itself, when
 * it finally happens, takes ~2.5 s. Observed first turns on this machine, same
 * bundle and same seed: 460 s, >200 s, 10 s, 10 s. It was not isolated, and the
 * likeliest reason is the fixture rather than the app — the seed carries a 236 MB
 * `sessions.db` (11,780 sessions / 48,252 messages) that a platform extension
 * reads before the first turn can be assembled. Trimming the seed would probably
 * retire this constant; guessing at it instead would produce a suite that goes
 * red for reasons no one can reproduce.
 */
const FIRST_TURN_TIMEOUT_MS = 600_000;
/** How long a turn may take to *start*, from Enter to the Stop button appearing. */
const TURN_START_TIMEOUT_MS = 30_000;

let launched: LaunchedApp;
let mainWindow: Page;

/** The extension this suite installs, and the MCP server behind it. */
const MCP_EXTENSION_NAME = 'Running Quotes';
const MCP_SERVER_SCRIPT = join(__dirname, 'basic-mcp.ts');

test.describe('Biorouter App', () => {
  test.skip(
    process.env.BIOROUTER_E2E_LIVE !== '1',
    'Set BIOROUTER_E2E_LIVE=1 to run the Electron end-to-end suite.'
  );

  test.beforeAll(async () => {
    // No `recordVideo` — see the warning in helpers/app.ts. It is what used to
    // take this whole file down in `beforeAll`.
    launched = await launchApp();
    mainWindow = launched.page;
    await mainWindow.screenshot({ path: 'test-results/initial-load.png' });
  });

  test.afterAll(async () => {
    await closeApp(launched);
  });

  test.describe('General UI', () => {
    test('opens straight into chat on the seeded provider', async () => {
      // The one assertion that pins the fixture: if the seed ever stops carrying
      // a configured provider, every chat test below fails on a provider-picker
      // it cannot answer, and this says so first.
      await expect(mainWindow.getByTestId('chat-input')).toBeVisible({ timeout: 30_000 });
    });

    test('dark mode toggle', async () => {
      // Settings sits outside the `Components` disclosure, so it needs no expand.
      await openSidebarEntry(mainWindow, 'Settings');

      const appTab = mainWindow.getByTestId('settings-app-tab');
      await expect(appTab).toBeVisible({ timeout: 10_000 });
      await appTab.click();

      const darkModeButton = mainWindow.getByTestId('dark-mode-button');
      const lightModeButton = mainWindow.getByTestId('light-mode-button');
      const systemModeButton = mainWindow.getByTestId('system-mode-button');
      await expect(darkModeButton).toBeVisible();

      const isDark = () =>
        mainWindow.evaluate(() => document.documentElement.classList.contains('dark'));
      const before = await isDark();

      await (before ? lightModeButton : darkModeButton).click();
      await expect.poll(isDark, { timeout: 5_000 }).toBe(!before);
      await mainWindow.screenshot({ path: 'test-results/dark-mode-toggle.png' });

      // System mode is clickable, then return the app to a known state.
      await systemModeButton.click();
      await lightModeButton.click();
      await expect.poll(isDark, { timeout: 5_000 }).toBe(false);

      await openSidebarEntry(mainWindow, 'Home');
    });
  });

  test.describe('Chat', () => {
    test('chat interaction', async () => {
      // The session's first turn — see FIRST_TURN_TIMEOUT_MS.
      test.setTimeout(FIRST_TURN_TIMEOUT_MS + 60_000);

      const chatInput = mainWindow.getByTestId('chat-input');
      await expect(chatInput).toBeVisible();
      await chatInput.fill('Hello, can you help me with a simple task?');
      await mainWindow.screenshot({ path: 'test-results/chat-before-send.png' });

      await chatInput.press('Enter');
      await waitForTurn(mainWindow, FIRST_TURN_TIMEOUT_MS);

      const response = mainWindow.getByTestId('message-container').last();
      await expect(response).toBeVisible();
      expect((await response.textContent())?.length ?? 0).toBeGreaterThan(0);
      await mainWindow.screenshot({ path: 'test-results/chat-response.png' });
    });

    test('verify chat history', async () => {
      test.setTimeout(LIVE_TURN_TIMEOUT_MS + 60_000);

      const chatInput = mainWindow.getByTestId('chat-input');
      await chatInput.fill('What is 2+2?');
      await chatInput.press('Enter');
      await waitForTurn(mainWindow);

      const messages = mainWindow.getByTestId('message-container');
      expect(await messages.count()).toBeGreaterThanOrEqual(2);
      expect((await messages.last().textContent())?.length ?? 0).toBeGreaterThan(0);
      await mainWindow.screenshot({ path: 'test-results/chat-history.png' });

      // Composer history. `handleHistoryNavigation` (ChatInput.tsx) only fires
      // with Cmd/Ctrl held, and bails if the user has typed into a non-empty
      // box — which is why this runs against the cleared composer after a send.
      await chatInput.press('Control+ArrowUp');
      await expect(chatInput).toHaveValue('What is 2+2?');
    });
  });

  test.describe('MCP Integration', () => {
    test('adds the Running Quotes MCP server as a custom extension', async () => {
      test.setTimeout(120_000);

      await openSidebarEntry(mainWindow, 'Extensions');
      await expect(mainWindow.locator('h1:has-text("Extensions")')).toBeVisible({
        timeout: 10_000,
      });

      await removeExtensionIfPresent(mainWindow, MCP_EXTENSION_NAME);

      await mainWindow.getByRole('button', { name: 'Add custom extension' }).first().click();
      const dialog = mainWindow.locator('[role="dialog"]');
      await expect(dialog).toBeVisible({ timeout: 10_000 });

      await dialog.locator('input[placeholder="Enter extension name..."]').fill(MCP_EXTENSION_NAME);
      await dialog
        .locator('input[placeholder="Optional description..."]')
        .fill('Inspirational running quotes MCP server');
      await dialog
        .locator('input[placeholder="e.g. npx -y @modelcontextprotocol/my-extension <filepath>"]')
        .fill(`node ${MCP_SERVER_SCRIPT}`);
      await mainWindow.screenshot({ path: 'test-results/mcp-filled-form.png' });

      await mainWindow.getByTestId('extension-submit-btn').click();

      // The row is found by the toggle's own aria-label rather than by
      // `div.flex:has-text(...)`, which matched every ancestor of the text.
      const toggle = mainWindow.getByRole('switch', {
        name: `Toggle ${MCP_EXTENSION_NAME} extension`,
      });
      await expect(toggle).toBeVisible({ timeout: 60_000 });
      await expect(toggle).toHaveAttribute('data-state', 'checked', { timeout: 15_000 });
      await mainWindow.screenshot({ path: 'test-results/mcp-extension-added.png' });

      await openSidebarEntry(mainWindow, 'Home');
    });

    test('the agent can call the runningQuote tool', async () => {
      test.setTimeout(LIVE_TURN_TIMEOUT_MS + 60_000);

      const chatInput = mainWindow.getByTestId('chat-input');
      await expect(chatInput).toBeVisible({ timeout: 30_000 });
      await chatInput.fill(
        'Call the runningQuote tool from the Running Quotes extension and reply with ' +
          'exactly what it returns, verbatim and unedited.'
      );
      await chatInput.press('Enter');
      await waitForTurn(mainWindow);

      await mainWindow.screenshot({ path: 'test-results/mcp-quote-response.png' });

      // Assert on the transcript rather than on the tool card's internals: the
      // quote reaches the reply either way, and the card's markup is the part
      // that keeps moving.
      const transcript = (await mainWindow.getByTestId('message-container').allTextContents()).join(
        '\n'
      );
      const quoted = runningQuotes.filter(({ quote }: { quote: string }) =>
        transcript.includes(quote)
      );
      expect(
        quoted.length,
        `No known running quote appeared in the transcript. Last 600 chars:\n${transcript.slice(-600)}`
      ).toBeGreaterThan(0);
    });
  });
});

/**
 * Waits for one turn: the composer swaps Send for Stop while a turn runs, and
 * swaps back when it ends.
 *
 * `chat-stop-button` replaced the `loading-indicator` test id this spec used to
 * wait on, which has had zero hits in `src/` for long enough that the wait was
 * failing at its 2 s budget rather than measuring anything. The button is
 * conditional on `isLoading && !hasSubmittableContent` (ChatInput.tsx), so it is
 * only a reliable signal once the composer has been cleared by the send.
 */
async function waitForTurn(page: Page, budgetMs = LIVE_TURN_TIMEOUT_MS): Promise<void> {
  const stop = page.getByTestId('chat-stop-button');
  await expect(stop).toBeVisible({ timeout: TURN_START_TIMEOUT_MS });
  await expect(stop).toHaveCount(0, { timeout: budgetMs });
}

/** Removes a previously installed extension so the add flow starts from a clean state. */
async function removeExtensionIfPresent(page: Page, name: string): Promise<void> {
  const configure = page.getByRole('button', { name: `Configure ${name} extension` });
  if ((await configure.count()) === 0) return;

  await configure.first().click();
  const dialog = page.locator('[role="dialog"]');
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await dialog.getByRole('button', { name: /^Remove extension$/i }).click();

  // Confirmation modal.
  await page
    .getByRole('button', { name: /^(Remove|Delete)$/i })
    .last()
    .click();
  await expect(configure).toHaveCount(0, { timeout: 30_000 });
}
