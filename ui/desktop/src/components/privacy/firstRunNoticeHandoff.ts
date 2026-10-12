import { useEffect, useState, useSyncExternalStore } from 'react';

/**
 * When the day-one notice may OPEN, as opposed to whether it is due (FR-R5-1).
 *
 * `firstRunNoticeDecision.ts` answers "is a notice due this launch?", and since FR-R4-4 it answers
 * from the boot, in parallel with the app chunk and the first page's provider read. On a slower
 * launch the count wins that race, and a gate that opened the moment the answer was `show` opened
 * the dialog while the boot splash was still fading (its wordmark drawn across the dialog's body
 * text for 100 to 200 ms), over the guard's empty loader, with the first-run band and its skeleton
 * popping in behind the blur a moment later. Measured at 3x and 4x CPU, in light and dark.
 *
 * The dialog therefore waits for the two things a launch hands off through, and for nothing else:
 *
 * - the boot splash has left the screen (`window.__brBootSplash.isGone()`, with an event for the
 *   moment it does), read from the splash itself because "is it still up?" is a fact about the
 *   page and not something to register; and
 * - the first page has drawn: `ProviderGuard` stops showing its empty loader and shows the
 *   first-run band, or lets the application through ({@link markFirstPageDrawn}). The boot
 *   declares that a first page is coming ({@link holdFirstRunNoticeForFirstPage}); the guard
 *   cannot declare it itself, because it mounts in the same commit as the gate and a hold taken
 *   in an effect would arrive after the gate had already drawn its dialog.
 *
 * Both orders of the race come out the same. Count first: the dialog waits for the page. Page
 * first (a slow count): the page has drawn already, and the dialog opens the moment the answer
 * lands, as it did before this module.
 *
 * ⚠ **A hold that can be missed must expire, because this is a disclosure.** A route outside the
 * guard (`#/welcome`, `#/configure-providers`), a splash that is never dismissed, or a guard that
 * never leaves its loader would otherwise keep a one-way-door notice from ever opening on that
 * window. {@link FIRST_RUN_NOTICE_HANDOFF_WAIT_MS} bounds the wait from the moment the gate has a
 * dialog to open; past it the dialog opens as it did before this module. The worst outcome is the
 * old overlap on a launch where something never answered, not a notice that stays hidden.
 *
 * Module state like `firstRunNoticeDecision`: it is the launch's state, and a launch is a page
 * load. A leaf module on purpose (it imports React and nothing else): the boot imports it before
 * the lazy app chunk has loaded.
 */

/**
 * The event the boot splash dispatches on `window` when it leaves the screen. The inline script in
 * `index.html` cannot import this, so it restates the spelling and `boot-splash.test.ts` runs the
 * shipped script and listens for this constant.
 */
export const BOOT_SPLASH_GONE_EVENT = 'br-boot-splash-gone';

/**
 * The longest the dialog waits for the splash and the first page, counted from the moment the
 * gate has a dialog to open. A splash fades in 260 ms and the guard reads one config key, so a
 * healthy launch is through in well under a second; this is the bound for a stall, not a
 * target.
 */
export const FIRST_RUN_NOTICE_HANDOFF_WAIT_MS = 3000;

let pageExpected = false;
let pageDrawn = false;
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function pageHeld(): boolean {
  return pageExpected && !pageDrawn;
}

/**
 * The boot declares that a first page is coming and the notice should wait for it. Called once,
 * by `renderer.tsx`, next to the notice's count and before React renders. Safe to call again.
 *
 * Nothing in a render or an effect can take this hold: see the module comment. A surface the boot
 * did not reach (a test, a story) never declares one, so nothing waits there.
 */
export function holdFirstRunNoticeForFirstPage(): void {
  if (pageExpected) return;
  pageExpected = true;
  notify();
}

/**
 * The first page has drawn. `ProviderGuard` calls this from an effect once it has stopped showing
 * its empty loader (its first-run band is committed, or the application is let through), which is
 * after the commit that drew it. A latch: a guard that remounts later (a return from Crew) does
 * not take the hold back.
 */
export function markFirstPageDrawn(): void {
  if (pageDrawn) return;
  pageDrawn = true;
  notify();
}

/** For tests: a new launch. Forgets the boot's declaration and the page's mark. */
export function resetFirstRunNoticeHandoffForTests(): void {
  pageExpected = false;
  pageDrawn = false;
  notify();
}

/**
 * Is the boot splash still on screen, or still to be taken down?
 *
 * Asked of the splash itself. No controller (a surface the inline script did not run on: a test,
 * a story) or one that predates `isGone` means there is nothing to wait for.
 */
function splashUp(): boolean {
  if (typeof window === 'undefined') return false;
  const splash = window.__brBootSplash;
  return Boolean(splash) && typeof splash?.isGone === 'function' && splash.isGone() === false;
}

function subscribeSplash(listener: () => void): () => void {
  window.addEventListener(BOOT_SPLASH_GONE_EVENT, listener);
  return () => window.removeEventListener(BOOT_SPLASH_GONE_EVENT, listener);
}

/**
 * May the notice open now?
 *
 * `wanted` is true while the gate has a dialog to show. The answer can only go from "no" to
 * "yes": once the dialog may open it stays allowed, so a splash or page that is reset or
 * re-declared under an open dialog never takes it away. The budget starts when `wanted` does and
 * a hold is in force, not before.
 */
export function useFirstRunNoticeMayOpen(
  wanted: boolean,
  waitMs: number = FIRST_RUN_NOTICE_HANDOFF_WAIT_MS
): boolean {
  const waitingForPage = useSyncExternalStore(subscribe, pageHeld, pageHeld);
  const waitingForSplash = useSyncExternalStore(subscribeSplash, splashUp, splashUp);
  const [released, setReleased] = useState(false);
  const holding = wanted && !released && (waitingForPage || waitingForSplash);

  useEffect(() => {
    if (!wanted || released) return;
    if (!holding) {
      setReleased(true);
      return;
    }
    const timer = window.setTimeout(() => setReleased(true), waitMs);
    return () => window.clearTimeout(timer);
  }, [wanted, holding, released, waitMs]);

  return wanted && !holding;
}
