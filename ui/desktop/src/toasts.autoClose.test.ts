/**
 * **A notification the user has walked away from is the one that most needs to
 * expire**, and until this test it was the one that never did.
 *
 * `react-toastify` defaults `pauseOnFocusLoss` to true: every dismissal timer
 * stops while the window is not frontmost. In an Electron app that is most of
 * the time — the user reads a toast, switches to their editor, and comes back
 * to a stack still sitting there. Observed on a fresh install: six "Extension
 * installed" toasts, minutes old, none of them expiring, which reads as "these
 * need dismissing" rather than as the FYI they are.
 *
 * `pauseOnHover` is deliberately left ON, and the pair is the whole point:
 * hovering pauses because the user is reading, which is a reason to wait;
 * losing focus is a reason to go.
 */
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const source = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), 'toasts.tsx'), 'utf8');

/** The shared options block every toast in the app is built from. */
function commonToastOptionsBlock(): string {
  const start = source.indexOf('const commonToastOptions: ToastOptions = {');
  expect(start, 'commonToastOptions was renamed; this test cannot find it').toBeGreaterThan(-1);
  const end = source.indexOf('};', start);
  return source.slice(start, end);
}

describe('the shared toast options', () => {
  it('do not let an unfocused window freeze every dismissal timer', () => {
    expect(commonToastOptionsBlock()).toContain('pauseOnFocusLoss: false');
  });

  it('still pause while the user is hovering, which is a reason to wait', () => {
    expect(commonToastOptionsBlock()).toContain('pauseOnHover: true');
  });

  it('expire inside the window a person will actually wait', () => {
    const ms = /export const TOAST_AUTO_CLOSE_MS = (\d+);/.exec(source);
    expect(ms, 'TOAST_AUTO_CLOSE_MS was renamed or removed').not.toBeNull();
    const value = Number(ms![1]);
    // Long enough to read a two-line message, short enough that a stack of
    // them clears itself rather than becoming a chore.
    expect(value).toBeGreaterThanOrEqual(3000);
    expect(value).toBeLessThanOrEqual(15000);
  });
});
