import { describe, expect, it, vi, beforeEach } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';

// Capture the toast without rendering it (BaseChat.createSessionError.test.ts idiom).
const mockToastWarning = vi.fn();
vi.mock('../toasts', () => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  toastWarning: (...args: unknown[]) => mockToastWarning(...args),
}));

import { returnInitialMessageToComposer, runInitialMessageAutoSubmit } from './BaseChat';

/**
 * REGRESSION GATE (BaseChat half) — duplicate submission on tab close.
 *
 * A chat tab carries `pendingInitialMessage`: the message that created its
 * session. `ChatGroupsShell` hands it to `BaseChat` as `initialMessage` and
 * `BaseChat` auto-submits it on mount. Only a group's ACTIVE tab is mounted, so
 * closing a tab REMOUNTS its successor's `BaseChat` — and the only guards were a
 * component-local ref (dies with the unmount) and a `navigate()` that clears
 * router state but not the tab record. Measured live on 2026-07-18: one message
 * plus three open/close cycles produced four real LLM turns, 23,584 input tokens.
 *
 * The fix is `onConsumed` — the owner's chance to drop the cargo. These pin
 * WHEN it fires, which is the whole subtlety: on the submitting branch only, and
 * only after the submit. Fire it on the `!hasSession` bail and the legitimate
 * FIRST submission is silently dropped instead.
 *
 * SECOND GATE, same helper: message loss on a REFUSED mount-time submit. The
 * submit now reports whether it took the message, and the mount used to discard
 * that answer: a refusal cleared the router state and marked the cargo spent
 * anyway, destroying the first message of a brand-new chat. It is now handed to
 * the composer and the cargo is kept. The tests below pin both halves, because
 * the fix's failure mode (spend the cargo late, once the turn's promise
 * resolves) is the duplicate-submission bug above coming back.
 *
 * The companion test chatGroups/duplicateSubmissionWiring.test.tsx pins the
 * other half: that ChatGroupsShell actually passes a callback that clears it.
 */

/**
 * The cargo is spent one MACROTASK after the submit, not synchronously: see the
 * helper's doc comment for why the verdict cannot simply be awaited. Two ticks,
 * so a refusal that needs a microtask drain of its own is never raced.
 */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0)).then(() => Promise.resolve());

/** Every collaborator, recorded, so ordering can be asserted and not just counts. */
function harness() {
  const calls: string[] = [];
  return {
    calls,
    submit: vi.fn((text: string) => {
      calls.push(`submit:${text}`);
    }),
    clearRouterState: vi.fn(() => {
      calls.push('clearRouterState');
    }),
    onConsumed: vi.fn(() => {
      calls.push('onConsumed');
    }),
    onRefused: vi.fn((message: string) => {
      calls.push(`onRefused:${message}`);
    }),
  };
}

describe('runInitialMessageAutoSubmit', () => {
  beforeEach(() => vi.clearAllMocks());

  it('submits the initial message and reports the cargo spent, in that order', async () => {
    const h = harness();

    const submitted = runInitialMessageAutoSubmit({
      hasSession: true,
      hasAutoSubmitted: false,
      initialMessage: 'WWMARKER name one animal',
      shouldStartAgent: false,
      submit: h.submit,
      clearRouterState: h.clearRouterState,
      onConsumed: h.onConsumed,
    });
    await settle();

    expect(submitted).toBe(true);
    expect(h.submit).toHaveBeenCalledTimes(1);
    expect(h.submit).toHaveBeenCalledWith('WWMARKER name one animal', undefined);
    expect(h.onConsumed).toHaveBeenCalledTimes(1);
    // The cargo is dropped AFTER it is spent, never before.
    expect(h.calls).toEqual(['submit:WWMARKER name one animal', 'clearRouterState', 'onConsumed']);
  });

  it('forwards attachments alongside the message', async () => {
    const h = harness();
    const attachments = [{ type: 'file', path: '/tmp/cohort.csv' }] as never;

    runInitialMessageAutoSubmit({
      hasSession: true,
      hasAutoSubmitted: false,
      initialMessage: 'summarize this',
      initialAttachments: attachments,
      shouldStartAgent: false,
      submit: h.submit,
      clearRouterState: h.clearRouterState,
      onConsumed: h.onConsumed,
    });
    await settle();

    expect(h.submit).toHaveBeenCalledWith('summarize this', attachments);
  });

  /**
   * THE DESIGN CONSTRAINT. A mount whose session has not resolved yet bails and
   * runs again once it has. If the owner cleared the cargo eagerly on render —
   * the obvious "simplification" — that first, legitimate message would never be
   * sent at all. So nothing may be consumed on this path.
   */
  it('does NOT consume the cargo when the session has not loaded yet', async () => {
    const h = harness();

    const submitted = runInitialMessageAutoSubmit({
      hasSession: false,
      hasAutoSubmitted: false,
      initialMessage: 'WWMARKER name one animal',
      shouldStartAgent: false,
      submit: h.submit,
      clearRouterState: h.clearRouterState,
      onConsumed: h.onConsumed,
    });
    await settle();

    expect(submitted).toBe(false);
    expect(h.submit).not.toHaveBeenCalled();
    expect(h.onConsumed).not.toHaveBeenCalled();
  });

  it('a bailed mount still submits once the session arrives', async () => {
    const h = harness();
    const args = {
      initialMessage: 'WWMARKER name one animal',
      shouldStartAgent: false,
      submit: h.submit,
      clearRouterState: h.clearRouterState,
      onConsumed: h.onConsumed,
    };

    // Effect run 1: session still resolving.
    let hasAutoSubmitted = runInitialMessageAutoSubmit({
      ...args,
      hasSession: false,
      hasAutoSubmitted: false,
    });
    // Effect run 2: session resolved.
    hasAutoSubmitted = runInitialMessageAutoSubmit({ ...args, hasSession: true, hasAutoSubmitted });
    await settle();

    expect(hasAutoSubmitted).toBe(true);
    expect(h.submit).toHaveBeenCalledTimes(1);
    expect(h.onConsumed).toHaveBeenCalledTimes(1);
  });

  it('never submits twice within one mount, however often the effect re-runs', async () => {
    const h = harness();
    const args = {
      hasSession: true,
      initialMessage: 'WWMARKER name one animal',
      shouldStartAgent: false,
      submit: h.submit,
      clearRouterState: h.clearRouterState,
      onConsumed: h.onConsumed,
    };

    let hasAutoSubmitted = runInitialMessageAutoSubmit({ ...args, hasAutoSubmitted: false });
    hasAutoSubmitted = runInitialMessageAutoSubmit({ ...args, hasAutoSubmitted });
    hasAutoSubmitted = runInitialMessageAutoSubmit({ ...args, hasAutoSubmitted });
    await settle();

    expect(hasAutoSubmitted).toBe(true);
    expect(h.submit).toHaveBeenCalledTimes(1);
    expect(h.onConsumed).toHaveBeenCalledTimes(1);
  });

  it('works without an owner callback (surfaces with no tab record)', async () => {
    const h = harness();

    expect(() =>
      runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: 'hello',
        shouldStartAgent: false,
        submit: h.submit,
        clearRouterState: h.clearRouterState,
        // no onConsumed — Dashboard / Hub mount BaseChat with no tab behind it
      })
    ).not.toThrow();
    await settle();
    expect(h.submit).toHaveBeenCalledTimes(1);
  });

  it('the shouldStartAgent path starts the agent but consumes no cargo', async () => {
    const h = harness();

    const submitted = runInitialMessageAutoSubmit({
      hasSession: true,
      hasAutoSubmitted: false,
      initialMessage: undefined,
      shouldStartAgent: true,
      submit: h.submit,
      clearRouterState: h.clearRouterState,
      onConsumed: h.onConsumed,
    });
    await settle();

    expect(submitted).toBe(true);
    expect(h.submit).toHaveBeenCalledWith('');
    // There was no message to spend, so there is nothing to tell the owner.
    expect(h.onConsumed).not.toHaveBeenCalled();
    expect(h.clearRouterState).not.toHaveBeenCalled();
  });

  it('an idle mount (no message, no agent flag) does nothing at all', async () => {
    const h = harness();

    const submitted = runInitialMessageAutoSubmit({
      hasSession: true,
      hasAutoSubmitted: false,
      shouldStartAgent: false,
      submit: h.submit,
      clearRouterState: h.clearRouterState,
      onConsumed: h.onConsumed,
    });
    await settle();

    expect(submitted).toBe(false);
    expect(h.calls).toEqual([]);
  });

  describe('a refused submit', () => {
    /**
     * THE MESSAGE-LOSS GATE. `false` from the store means the message was never
     * sent, nothing was shown, and the caller still owns the text. The composer
     * never held it (it came in as route cargo), so the only two places it can
     * survive are the composer, via `onRefused`, and the tab record. Both.
     */
    it('hands the message back and keeps the cargo', async () => {
      const h = harness();

      const submitted = runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: 'WWMARKER name one animal',
        shouldStartAgent: false,
        submit: vi.fn(async () => false),
        clearRouterState: h.clearRouterState,
        onConsumed: h.onConsumed,
        onRefused: h.onRefused,
      });
      await settle();

      expect(h.onRefused).toHaveBeenCalledTimes(1);
      expect(h.onRefused).toHaveBeenCalledWith('WWMARKER name one animal', undefined);
      // Neither half of "spent" may fire: the cargo is the durable copy.
      expect(h.onConsumed).not.toHaveBeenCalled();
      expect(h.clearRouterState).not.toHaveBeenCalled();
      // The mount has still spent its ONE attempt, so the effect must not loop.
      expect(submitted).toBe(true);
    });

    it('hands the attachments back with the message', async () => {
      const h = harness();
      const attachments = [{ type: 'file', path: '/tmp/cohort.csv' }] as never;

      runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: 'summarize this',
        initialAttachments: attachments,
        shouldStartAgent: false,
        submit: vi.fn(async () => false),
        clearRouterState: h.clearRouterState,
        onConsumed: h.onConsumed,
        onRefused: h.onRefused,
      });
      await settle();

      expect(h.onRefused).toHaveBeenCalledWith('summarize this', attachments);
    });

    /**
     * A refusal is decided before the submit does any network work, so it beats
     * the deadline in practice. If one ever does not, the cargo is already spent
     * and the composer becomes the only copy, so the give-back is unconditional
     * rather than an else-branch of the deadline.
     */
    it('still hands the message back when the refusal arrives after the deadline', async () => {
      const h = harness();

      runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: 'slow refusal',
        shouldStartAgent: false,
        submit: vi.fn(() => new Promise<boolean>((resolve) => setTimeout(() => resolve(false), 5))),
        clearRouterState: h.clearRouterState,
        onConsumed: h.onConsumed,
        onRefused: h.onRefused,
      });
      await new Promise((resolve) => setTimeout(resolve, 20));

      expect(h.onConsumed).toHaveBeenCalledTimes(1);
      expect(h.onRefused).toHaveBeenCalledWith('slow refusal', undefined);
    });

    /** A submit predating the contract resolves `undefined`. Not a refusal. */
    it('reads only an explicit false as a refusal', async () => {
      const h = harness();

      runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: 'legacy caller',
        shouldStartAgent: false,
        submit: vi.fn(async () => undefined),
        clearRouterState: h.clearRouterState,
        onConsumed: h.onConsumed,
        onRefused: h.onRefused,
      });
      await settle();

      expect(h.onRefused).not.toHaveBeenCalled();
      expect(h.onConsumed).toHaveBeenCalledTimes(1);
    });
  });

  describe('the duplicate-send guard', () => {
    /**
     * THE ONE THAT MATTERS MOST. `handleSubmit` does not resolve `true` until the
     * TURN ends, which is minutes on a real task. If the cargo waited for that,
     * every tab close during the turn would re-send the message, which is the
     * 2026-07-18 bug. An accepted submit must spend the cargo while the turn is still
     * running.
     */
    it('spends the cargo while an accepted turn is still running', async () => {
      const h = harness();

      runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: 'a long task',
        shouldStartAgent: false,
        // Resolves when the turn ends, which for this test is never.
        submit: vi.fn(() => new Promise<boolean>(() => {})),
        clearRouterState: h.clearRouterState,
        onConsumed: h.onConsumed,
        onRefused: h.onRefused,
      });
      await settle();

      expect(h.clearRouterState).toHaveBeenCalledTimes(1);
      expect(h.onConsumed).toHaveBeenCalledTimes(1);
      expect(h.onRefused).not.toHaveBeenCalled();
    });

    /**
     * The remount. The owner drops `pendingInitialMessage` when told the cargo is
     * spent, so the successor mount is handed nothing and sends nothing. Modelled
     * end to end rather than asserted on a counter, because the failure this
     * guards is a SECOND real agent turn.
     */
    it('a remount after acceptance does not re-send', async () => {
      const h = harness();
      // The owner's record, exactly as ChatGroupsShell holds it.
      let cargo: string | undefined = 'WWMARKER name one animal';
      const onConsumed = vi.fn(() => {
        cargo = undefined;
      });

      // Mount 1: accepted.
      runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: cargo,
        shouldStartAgent: false,
        submit: h.submit,
        clearRouterState: h.clearRouterState,
        onConsumed,
        onRefused: h.onRefused,
      });
      await settle();
      expect(cargo).toBeUndefined();

      // Mount 2: a fresh component, so `hasAutoSubmitted` starts false again.
      const remounted = runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: cargo,
        shouldStartAgent: false,
        submit: h.submit,
        clearRouterState: h.clearRouterState,
        onConsumed,
        onRefused: h.onRefused,
      });
      await settle();

      expect(remounted).toBe(false);
      expect(h.submit).toHaveBeenCalledTimes(1);
      expect(onConsumed).toHaveBeenCalledTimes(1);
    });

    /**
     * The mirror image: a refusal keeps the cargo, so the next mount is the
     * retry. That is the whole point of not consuming it, and it is exactly ONE
     * send, because the first attempt sent nothing.
     */
    it('a remount after a refusal retries the message', async () => {
      const h = harness();
      let cargo: string | undefined = 'WWMARKER name one animal';
      const onConsumed = vi.fn(() => {
        cargo = undefined;
      });

      runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: cargo,
        shouldStartAgent: false,
        submit: vi.fn(async () => false),
        clearRouterState: h.clearRouterState,
        onConsumed,
        onRefused: h.onRefused,
      });
      await settle();
      expect(cargo).toBe('WWMARKER name one animal');

      runInitialMessageAutoSubmit({
        hasSession: true,
        hasAutoSubmitted: false,
        initialMessage: cargo,
        shouldStartAgent: false,
        submit: h.submit,
        clearRouterState: h.clearRouterState,
        onConsumed,
        onRefused: h.onRefused,
      });
      await settle();

      expect(h.submit).toHaveBeenCalledTimes(1);
      expect(h.submit).toHaveBeenCalledWith('WWMARKER name one animal', undefined);
      expect(cargo).toBeUndefined();
    });
  });
});

describe('returnInitialMessageToComposer', () => {
  beforeEach(() => vi.clearAllMocks());

  it('restores the message into the composer that submitted it, and says so', () => {
    const dispatch = vi.spyOn(window, 'dispatchEvent');

    returnInitialMessageToComposer({
      sessionId: 'sess-1',
      message: 'WWMARKER name one animal',
      attachments: [],
    });

    const restore = dispatch.mock.calls
      .map((c) => c[0] as CustomEvent)
      .find((e) => e?.type === 'restore-chat-input');
    expect(restore).toBeTruthy();
    // Matched by sessionId in ChatInput's listener, so a broadcast can only
    // reach the composer that belongs to this chat.
    expect(restore!.detail).toMatchObject({
      sessionId: 'sess-1',
      value: 'WWMARKER name one animal',
    });

    // Text reappearing in an empty composer with no turn is only legible once
    // the user knows the send did not happen.
    expect(mockToastWarning).toHaveBeenCalledTimes(1);
    expect(mockToastWarning).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Message not sent' })
    );

    dispatch.mockRestore();
  });

  it('sends nothing for a message with no chat to address', () => {
    // A chat with no id cannot be addressed by a window-wide event without
    // addressing every such composer: a new tab's in every pane matched `''`.
    // `restoreComposerText` refuses it rather than broadcast it.
    const dispatch = vi.spyOn(window, 'dispatchEvent');

    returnInitialMessageToComposer({ message: 'keep me' });
    returnInitialMessageToComposer({ sessionId: '', message: 'keep me' });
    returnInitialMessageToComposer({ sessionId: null, message: 'keep me' });

    const restores = dispatch.mock.calls
      .map((c) => c[0] as CustomEvent)
      .filter((e) => e?.type === 'restore-chat-input');
    expect(restores).toHaveLength(0);

    dispatch.mockRestore();
  });
});

/**
 * The two halves above are each sound and prove nothing on their own: the helper
 * only gives the message back if the effect asks it to, and `BaseChatContent`
 * cannot be mounted here (react-router plus a dozen contexts, the reason the
 * decision was lifted out of the effect in the first place). So the wiring is
 * asserted AT THE SOURCE, the same way styles/composerFocus.test.ts pins a rule
 * jsdom cannot evaluate. Reading the file is the idiom BaseChat.privacy.test.tsx
 * already uses; vitest runs with `ui/desktop` as its root.
 */
describe('the effect wires the refusal to the composer', () => {
  it('passes onRefused, and it restores the message', () => {
    const source = readFileSync(path.join(process.cwd(), 'src/components/BaseChat.tsx'), 'utf8');
    const start = source.indexOf('runInitialMessageAutoSubmit({');
    expect(start).toBeGreaterThan(-1);
    const end = source.indexOf('\n  }, [', start);
    expect(end).toBeGreaterThan(start);

    const effectCall = source.slice(start, end);
    expect(effectCall).toMatch(/onRefused:[\s\S]*returnInitialMessageToComposer\(/);
    // Addressed to this chat's composer, so a split view cannot restore into a
    // sibling (ChatInput's listener matches on sessionId).
    expect(effectCall).toMatch(/returnInitialMessageToComposer\(\{\s*sessionId,/);
  });
});
