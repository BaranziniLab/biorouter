/**
 * End-to-end: a scheduled workflow's figure reaches the artifact side panel.
 *
 * The scenario is the whole path a user takes, with no shortcuts through the
 * HTTP API:
 *
 *   Scheduler → New schedule (a workflow that asks the Auto Visualiser for one
 *   bar chart) → Run now → open the run's read-only transcript → the figure is
 *   a click-to-open CARD and nothing else → click it → the figure opens in the
 *   artifact side panel.
 *
 * The "and nothing else" step is the point of the test rather than decoration.
 * `docs/desktop-ui/artifact-display-surfaces.md` makes the panel the only
 * surface an artifact is ever displayed on, and makes a *saved* transcript open
 * nothing until the reader clicks — so the spec asserts the panel is ABSENT
 * before the click and PRESENT after it. An inline renderer coming back would
 * pass a test that only looked for the figure.
 *
 * It runs against two variants, gated separately:
 *
 *   BIOROUTER_E2E_LIVE=1                       → the dev bundle in `.vite/build/`
 *   BIOROUTER_E2E_LIVE=1 BIOROUTER_E2E_APP=…   → also the packaged Biorouter.app
 *
 * Both are off by default because both spend a real model turn.
 *
 * See `docs/desktop-ui/e2e-playwright-suite.md` for how to run each gate and
 * what CI would need.
 */

import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test';
import * as fs from 'fs';
import * as path from 'path';
import {
  createSandbox,
  dismissStartupModals,
  EVIDENCE_DIR,
  launchDevBundle,
  launchPackagedApp,
  writeBarChartWorkflow,
  type LaunchedApp,
} from './schedule-artifact.helpers';

/** The card `MCPUIResourceRenderer` renders for a `ui://` resource. */
const ARTIFACT_CARD = 'button[aria-label^="Open "][aria-label$="in the artifact viewer"]';

/**
 * How long a real GPT-5.5 turn plus a figure render is allowed to take. Measured
 * runs land well inside this; the headroom is for a cold provider connection.
 */
const RUN_BUDGET_MS = 300_000;

/** Launch + navigation + the run budget. */
const TEST_TIMEOUT_MS = 420_000;

interface Variant {
  /** Appears in the test title, so keep it short and stable. */
  name: string;
  /** Why this variant is skipped, or `null` when it should run. */
  skipReason: string | null;
  launch: (root: string) => Promise<LaunchedApp>;
}

const live = process.env.BIOROUTER_E2E_LIVE === '1';
const packagedApp = process.env.BIOROUTER_E2E_APP;

/**
 * The port range this spec is allowed to use. It must not collide with a dev
 * instance the developer is already running (9222/9277/9333/9346 are in use on
 * the machine this was written on).
 */
const PACKAGED_CDP_PORT = Number(process.env.BIOROUTER_E2E_CDP_PORT || 9361);

const VARIANTS: Variant[] = [
  {
    name: 'dev bundle',
    skipReason: live ? null : 'Set BIOROUTER_E2E_LIVE=1 to run this live scenario.',
    launch: (root) => launchDevBundle(root),
  },
  {
    name: 'packaged app',
    skipReason: !live
      ? 'Set BIOROUTER_E2E_LIVE=1 to run this live scenario.'
      : !packagedApp
        ? 'Set BIOROUTER_E2E_APP to a built Biorouter.app to run the packaged variant.'
        : null,
    launch: (root) => launchPackagedApp(packagedApp!, root, PACKAGED_CDP_PORT),
  },
];

// A trace that only exists on a retry cannot answer "what did the passing run
// actually show?", which is the question this scenario is here to answer.
// File-scoped rather than per-describe: Playwright refuses `use({ trace })`
// inside a describe group because it would force a new worker.
test.use({ trace: 'on' });

for (const variant of VARIANTS) {
  test.describe(`Scheduled run artifact — ${variant.name}`, () => {
    test.skip(variant.skipReason !== null, variant.skipReason ?? '');

    let app: LaunchedApp;
    let page: Page;
    let root: string;
    let workflowPath: string;
    const scheduleId = `e2e-bar-chart-${Date.now()}`;

    test.beforeAll(async () => {
      // `BIOROUTER_E2E_PATH_ROOT` is honoured only when the packaged variant is
      // off: the two variants run two Electron instances, and pointing both at
      // one root would have them share a sessions database and a schedule file.
      root =
        !packagedApp && process.env.BIOROUTER_E2E_PATH_ROOT
          ? process.env.BIOROUTER_E2E_PATH_ROOT
          : createSandbox('biorouter-e2e-schedule-artifact-');
      workflowPath = writeBarChartWorkflow(root);
      app = await variant.launch(root);
      page = app.page;
      await dismissStartupModals(page);
    });

    test.afterAll(async () => {
      if (app) await app.close();
    });

    test('a scheduled figure opens in the artifact panel, and only on click', async () => {
      test.setTimeout(TEST_TIMEOUT_MS);
      const testInfo = test.info();

      await gotoScheduler(page);
      await createSchedule(page, scheduleId, workflowPath);
      await openScheduleDetail(page, scheduleId);
      await runNow(page);

      const card = await openRunTranscriptOnceTheFigureExists(page, RUN_BUDGET_MS);

      // The contract: a saved transcript shows a card and opens nothing.
      const label = (await card.getAttribute('aria-label')) ?? '(no aria-label)';
      testInfo.annotations.push({ type: 'artifact-card', description: label });
      await expect(
        page.getByTestId('artifact-viewer'),
        'a read-only transcript must not open the artifact panel by itself'
      ).toHaveCount(0);

      await card.click();

      const viewer = page.getByTestId('artifact-viewer');
      await expect(viewer).toBeVisible({ timeout: 30_000 });

      // Not decoration: the panel renders a `loading` placeholder until the
      // artifact resolves to HTML, so asserting only that the panel exists would
      // pass for a figure that never arrives. The named iframe is the branch
      // that means "resolved to renderable HTML" (`ArtifactViewer.tsx:1400`).
      const frame = page.locator('iframe[name="biorouter-artifact-preview"]');
      await expect(frame).toBeAttached({ timeout: 60_000 });

      // And the figure itself: Chart.js draws into a canvas inside the sandboxed
      // frame. This is what separates "the panel opened" from "the figure drew",
      // which is a real failure mode — a report whose libraries did not load
      // renders an empty card.
      const canvas = page
        .frameLocator('iframe[name="biorouter-artifact-preview"]')
        .locator('canvas');
      await expect(canvas.first()).toBeVisible({ timeout: 60_000 });

      await captureEvidence(page, testInfo, variant.name);
    });
  });
}

/**
 * Reach the Scheduler by hash — the sidebar entry is inside a disclosure that is
 * collapsed by default, so navigating by hash avoids having to open it.
 *
 * Two things in this loop are the fix for a measured failure, not caution:
 *
 *  - **Modals are re-dismissed on every pass.** A dialog open anywhere makes the
 *    background `aria-hidden`, and Playwright's role queries skip aria-hidden
 *    subtrees — so a late-arriving notice does not hide the Scheduler, it makes
 *    the Scheduler *unfindable*. See `dismissStartupModals`.
 *  - **The hash is set repeatedly.** A single set races the renderer's boot:
 *    `waitForRenderer` returns as soon as `#root` has any child, which is true
 *    while the shell is still mounting, and a `hashchange` delivered before the
 *    router is listening is simply lost.
 *
 * The page is identified by its "New schedule" button rather than by the
 * `Scheduler` heading. Both are unique to this page; the button is the next
 * thing the test clicks, so waiting on it proves the page is actually usable.
 */
async function gotoScheduler(page: Page): Promise<void> {
  const newSchedule = page.getByRole('button', { name: 'New schedule' });
  const deadline = Date.now() + 120_000;
  for (;;) {
    await dismissStartupModals(page);
    await page.evaluate(() => {
      window.location.hash = '#/schedules';
    });
    try {
      await newSchedule.waitFor({ state: 'visible', timeout: 5_000 });
      await expect(page.getByRole('heading', { name: 'Scheduler' })).toBeVisible();
      return;
    } catch {
      if (Date.now() >= deadline) {
        throw new Error('Scheduler never rendered after repeated #/schedules navigation');
      }
    }
  }
}

async function createSchedule(page: Page, id: string, workflowPath: string): Promise<void> {
  await page.getByRole('button', { name: 'New schedule' }).click();
  await expect(page.locator('#schedule-form')).toBeVisible();

  await page.locator('#scheduleId-modal').fill(id);
  // An absolute path: the daemon copies the file into
  // `<data>/scheduled_workflows/<id>.<ext>` when the schedule is created.
  await page.locator('#workflowSource-modal').fill(workflowPath);
  await page.getByRole('button', { name: 'Create schedule' }).click();

  await expect(page.locator('#schedule-form')).toHaveCount(0, { timeout: 30_000 });
}

async function openScheduleDetail(page: Page, id: string): Promise<void> {
  const row = page.getByRole('button', { name: `View schedule ${id}` });
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.click();
  await expect(page.getByRole('button', { name: 'Run now' })).toBeVisible({ timeout: 30_000 });
}

async function runNow(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Run now' }).click();
  // The run row appears as soon as `handleRunNow` refetches the session list.
  await expect(page.locator('button[aria-label^="Open run"]').first()).toBeVisible({
    timeout: 120_000,
  });
}

/**
 * Open the run's transcript, and keep re-opening it until the figure is there.
 *
 * ⚠ `ScheduleDetailView` does NOT poll. `fetchSessions` / `fetchSchedule` run
 * once on mount and once after "Run now", so a transcript opened while the job
 * is still working shows a partial conversation and never updates itself. What
 * *does* refresh is leaving and re-entering: the effect keyed on
 * `[scheduleId, selectedSession]` refetches when `selectedSession` returns to
 * null, which is exactly what the Back button does. So the wait is a
 * Back/re-open loop rather than a `waitFor` on a card that would never arrive.
 */
async function openRunTranscriptOnceTheFigureExists(
  page: Page,
  budgetMs: number
): Promise<Locator> {
  const deadline = Date.now() + budgetMs;
  let attempts = 0;

  for (;;) {
    attempts += 1;
    await page.locator('button[aria-label^="Open run"]').first().click();
    // The transcript surface: a read-only `SessionHistoryView` with a Back button.
    await expect(page.getByRole('button', { name: 'Back' }).first()).toBeVisible({
      timeout: 30_000,
    });

    const card = page.locator(ARTIFACT_CARD).first();
    try {
      await card.waitFor({ state: 'visible', timeout: 5_000 });
      return card;
    } catch {
      if (Date.now() >= deadline) {
        throw new Error(
          `No artifact card in the run transcript after ${attempts} attempts within ${Math.round(
            budgetMs / 1000
          )}s. The scheduled run either did not call render_figure or did not finish.`
        );
      }
    }

    await page.getByRole('button', { name: 'Back' }).first().click();
    await expect(page.getByRole('button', { name: 'Run now' })).toBeVisible({ timeout: 30_000 });
    await page.waitForTimeout(5_000);
  }
}

/**
 * Save a screenshot of the panel beside the transcript.
 *
 * Written to `EVIDENCE_DIR` rather than Playwright's `outputDir`, because
 * `playwright.config.ts` sets `preserveOutput: 'failures-only'` and would delete
 * a passing run's output — and a pass is precisely when this evidence matters.
 * It is also attached, so it reaches the HTML report.
 */
async function captureEvidence(page: Page, testInfo: TestInfo, variantName: string): Promise<void> {
  const slug = variantName.replace(/[^a-z0-9]+/gi, '-').toLowerCase();
  fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
  const file = path.join(EVIDENCE_DIR, `schedule-artifact-panel-${slug}.png`);
  const png = await page.screenshot({ path: file, fullPage: false });
  await testInfo.attach(`artifact-panel-${slug}`, { body: png, contentType: 'image/png' });
}
