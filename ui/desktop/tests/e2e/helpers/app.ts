/**
 * The one way this suite starts the desktop app.
 *
 * Every spec launches through {@link launchApp} so the sandbox invariant in
 * `sandbox.ts` cannot be forgotten at a call site: the root is created here, the
 * env that redirects the app onto it is assembled here, and `--user-data-dir` —
 * the half that keeps the *renderer's* localStorage out of the shared default
 * Electron profile — is appended here rather than left to each spec.
 *
 * The target is the **dev bundle** (`.vite/build/main.js`, produced by
 * `npm run build:e2e`), never a packaged app: the packaged main process resolves
 * its backend and its renderer differently, so a green run against it would say
 * nothing about the tree under test.
 *
 * ⚠ **Never pass `recordVideo` to `electron.launch`.** There is deliberately no
 * option for it here. Under @playwright/test 1.58.2 it does not slow the launch
 * down, it breaks it: `firstWindow()` resolves in ~350 ms to a page whose `url()`
 * is the empty string, `waitForLoadState('domcontentloaded')` then times out, and
 * `close()` throws `Cannot read properties of null (reading 'stop')` from inside
 * the video artifact machinery — which is why the worker also blows its 60 s
 * teardown budget and takes a whole spec file down with it. Measured A/B against
 * this same bundle: without it the window is up and `#root` is mounted in 906 ms;
 * with it, still nothing after 12 minutes. Use `page.screenshot()` for evidence.
 */

import { _electron as electron, type ElectronApplication, type Page } from '@playwright/test';
import * as path from 'node:path';
import { createSandbox, type Sandbox } from './sandbox';

/** `ui/desktop` — the cwd the main process expects and the root of the built bundle. */
const DESKTOP_ROOT = path.join(__dirname, '../../..');
/** The dev bundle's entry point. Rebuilt by `npm run build:e2e`. */
export const MAIN_ENTRY = path.join(DESKTOP_ROOT, '.vite/build/main.js');

export interface LaunchOptions {
  /** Reuse an existing sandbox instead of creating one (the caller keeps ownership). */
  sandbox?: Sandbox;
  /** Extra environment for the main process. Applied last, so it may override the defaults. */
  env?: Record<string, string>;
  /** Extra Electron/Chromium switches, appended after `--user-data-dir`. */
  args?: string[];
  /** Dismiss the first-run modals after mount. Default true. */
  dismissModals?: boolean;
}

export interface LaunchedApp {
  app: ElectronApplication;
  page: Page;
  sandbox: Sandbox;
  /** True when `launchApp` created the sandbox and `closeApp` should delete it. */
  ownsSandbox: boolean;
}

/** Launches the dev bundle against an isolated config root and waits for the React tree. */
export async function launchApp(options: LaunchOptions = {}): Promise<LaunchedApp> {
  const ownsSandbox = options.sandbox === undefined;
  const sandbox = options.sandbox ?? createSandbox();

  const app = await electron.launch({
    args: [
      MAIN_ENTRY,
      // Without this the renderer shares the default Electron profile with every
      // other Biorouter on the machine — including the sidebar's remembered
      // disclosure state, which decides whether half this suite's nav targets
      // are in the DOM at all.
      `--user-data-dir=${path.join(sandbox.root, 'electron')}`,
      ...(options.args ?? []),
    ],
    cwd: DESKTOP_ROOT,
    env: {
      ...process.env,
      ELECTRON_IS_DEV: '1',
      NODE_ENV: 'development',
      BIOROUTER_ALLOWLIST_BYPASS: 'true',
      BIOROUTER_DISABLE_KEYRING: 'true',
      BIOROUTER_PATH_ROOT: sandbox.root,
      // Agent shells commonly export ELECTRON_RUN_AS_NODE=1, which makes Electron
      // exit instantly with no window and no error. Blank it, never inherit it.
      ELECTRON_RUN_AS_NODE: '',
      ...(options.env ?? {}),
    },
  });

  const page = await app.firstWindow();
  await page.waitForLoadState('domcontentloaded');
  await page.waitForFunction(() => {
    const root = document.getElementById('root');
    return !!root && root.children.length > 0;
  });

  if (options.dismissModals !== false) {
    await dismissFirstRunModals(page);
  }

  return { app, page, sandbox, ownsSandbox };
}

/** Closes the app and disposes the sandbox this launch created. Never throws. */
export async function closeApp(launched: LaunchedApp | undefined): Promise<void> {
  if (!launched) return;
  await launched.app.close().catch(() => {});
  if (launched.ownsSandbox) launched.sandbox.cleanup();
}

/** Modals a fresh profile raises, each with the control that closes it. */
const FIRST_RUN_MODALS: Array<{ title: RegExp; button: RegExp }> = [
  // DependencySetupModal — 'Dismiss', or 'Done' once an install has finished.
  // Never 'Install': this suite downloads no toolchain and no model.
  {
    title: /Install missing dependencies|Update the Biorouter CLI|Repair the Biorouter CLI/i,
    button: /^(Dismiss|Done)$/,
  },
  // FirstRunPrivacyNotice.
  { title: /chats are now marked private/i, button: /^Got it$/ },
  // AnnouncementModal — its title is whatever the announcement is called, so it
  // is matched by its button alone (the fallback below).
];

/** How long to wait for the launch dialogs to appear, be dismissed and unmount. */
const MODAL_SETTLE_BUDGET_MS = 30_000;
/**
 * The floor on how long the app is WATCHED before it is declared modal-free.
 *
 * Not a politeness pause — it is the whole correctness of the check. These
 * dialogs open on the resolution of an async probe, not on mount, so "no overlay
 * right now" is worth nothing until enough time has passed for the probe to have
 * finished. Measured on this bundle: nothing at mount, nothing at +1 s, the
 * privacy notice open at +4 s, gone again by +10 s.
 */
const MODAL_OBSERVE_FLOOR_MS = 9_000;
/** Consecutive dialog-free polls that count as settled, once past the floor. */
const QUIET_POLLS = 5;
const POLL_MS = 400;

/**
 * Waits out the modals a *fresh* profile raises, dismissing any that need it.
 *
 * ⚠ **This has to POLL, not check once.** Two things make a single check wrong,
 * and both were measured on this bundle: the dialogs do not exist yet when
 * `#root` first has children (`FirstRunPrivacyNotice` appeared at ~4 s), and one
 * of them closes itself once its backing fetch resolves. A check at mount time
 * therefore sees nothing, returns, and the modal opens behind the test.
 *
 * ⚠ **The thing that blocks a click is the OVERLAY, not the dialog.** Radix
 * renders `[data-slot="dialog-overlay"]` with `aria-hidden="true"`, so it is
 * invisible to the accessibility tree — a page snapshot of a blocked app shows a
 * perfectly ordinary sidebar and no dialog at all, while every click times out
 * with "intercepts pointer events". Settling is defined against the overlay for
 * that reason.
 */
export async function dismissFirstRunModals(page: Page): Promise<void> {
  const started = Date.now();
  const deadline = started + MODAL_SETTLE_BUDGET_MS;
  let quiet = 0;
  let lastTitle = '';

  const settled = () => quiet >= QUIET_POLLS && Date.now() - started >= MODAL_OBSERVE_FLOOR_MS;

  while (Date.now() < deadline && !settled()) {
    const overlay = page.locator('[data-slot="dialog-overlay"][data-state="open"]').first();
    if (!(await visible(overlay))) {
      quiet += 1;
      await page.waitForTimeout(POLL_MS);
      continue;
    }
    quiet = 0;

    const dialog = page.locator('[role="dialog"]').first();
    const text = (await dialog.textContent().catch(() => '')) ?? '';
    lastTitle = text.slice(0, 120);
    const match = FIRST_RUN_MODALS.find((entry) => entry.title.test(text));
    const button = dialog.getByRole('button', { name: match?.button ?? /^Got it$/ }).first();
    if (await visible(button)) {
      await button.click().catch(() => {});
    }
    await page.waitForTimeout(POLL_MS);
  }

  if (!settled()) {
    // Naming the dialog is the whole point: the alternative is every later click
    // failing on an `aria-hidden` overlay with no clue what put it there.
    throw new Error(
      `A modal is still open after ${MODAL_SETTLE_BUDGET_MS}ms and blocks every click. ` +
        `Add it to FIRST_RUN_MODALS in tests/e2e/helpers/app.ts. Dialog text: ${lastTitle}`
    );
  }
}

/** `locator.isVisible()` without the throw, so a missing modal reads as "not there". */
function visible(locator: ReturnType<Page['locator']>): Promise<boolean> {
  return locator.isVisible().catch(() => false);
}
