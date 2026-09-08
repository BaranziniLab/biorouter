/**
 * P-01 — every call this store makes to a **gated** route carries the
 * user-action proof.
 *
 * Issue #56 Task 58 put `POST /reply` and `GET /sessions/{id}/events` on
 * `routes/session_reach.rs`'s gated list: reaching a PRIVATE chat over either
 * takes a caller capability that covers the chat, or `X-User-Action`. The
 * renderer is the user's surface and holds the key, so the proof is its way in
 * — and it is one this store had only on the SUBMIT path.
 *
 * What that omission cost, measured against a real daemon: reload a window
 * mid-turn in a private chat and `attachToTurn` POSTs `/reply` with no proof.
 * The daemon answers 403; the generated SSE client (`api/core/
 * serverSentEvents.gen.ts`) does not honour `throwOnError` — it reports the
 * non-OK response to `onSseError` and, with `sseMaxRetryAttempts: 1`, ENDS the
 * generator — so the store sees a stream that opened and immediately closed.
 * `streamFromResponse` reads that as a dropped socket, spends all three
 * `reattachAfterDrop` attempts against the same 403 inside 300ms, and paints
 * "The connection closed before Biorouter received a completion status."
 * The daemon meanwhile finishes the turn and persists every message; the user
 * sees a permanent "Connection dropped" and no transcript until a full restart.
 *
 * ⚠ **Every chat on a LOCAL model is private-tier**, and this app ranks Local
 * Models first — so this was the ordinary case, not an edge one. A public chat
 * takes the identical code path and was never affected, which is exactly why
 * the failure looked intermittent.
 *
 * ⚠ This is written as a CENSUS, not as three per-call-site assertions, and it
 * is deliberate: the defect was one call site out of three on one route, so an
 * assertion per site that exists today is satisfied by a fourth site added
 * tomorrow. The invariant is "no gated call leaves this store unproven",
 * whatever produced it.
 *
 * What a jsdom test can and cannot pin, stated plainly: it pins what the store
 * puts on the wire. It cannot pin the daemon's answer to it — that the header
 * is what makes the difference between 403 and 200 on a private chat is a
 * server fact, verified against a real `biorouterd` rather than asserted here,
 * and it lives in `routes/session_reach.rs`'s own tests.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MessageEvent, Session, TokenState } from '../api';

const USER_ACTION_KEY = 'proof-of-user';

const mocks = vi.hoisted(() => ({
  reply: vi.fn(),
  observeSessionEvents: vi.fn(),
  resumeAgent: vi.fn(),
  cancelTurn: vi.fn(async () => ({ data: { cancelled: true } })),
  getSession: vi.fn(async () => ({ data: null })),
  interrupt: vi.fn(),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  updateFromSession: vi.fn(async (_options?: unknown) => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../api', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return { ...actual, ...mocks };
});

import { ChatStreamRegistry } from './chatStreamStore';

const tokenState: TokenState = {
  accumulatedInputTokens: 0,
  accumulatedOutputTokens: 0,
  accumulatedTotalTokens: 0,
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
};

const finishFrame = { type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent;

function session(id: string): Session {
  return {
    id,
    name: `Session ${id}`,
    working_dir: '/tmp',
    conversation: [],
    message_count: 0,
    total_tokens: 0,
    created_at: '',
    updated_at: '',
    extension_data: {},
    user_set_name: false,
  } as Session;
}

async function* streamOf(...frames: MessageEvent[]) {
  for (const frame of frames) yield frame;
}

/** Every `headers` object the store handed a gated route, in call order. */
function gatedCallHeaders(): Array<Record<string, string> | undefined> {
  return [...mocks.reply.mock.calls, ...mocks.observeSessionEvents.mock.calls].map(
    (call) => (call[0] as { headers?: Record<string, string> }).headers
  );
}

/**
 * A fresh session id per test. The transcript LRU in `utils/sessionNameSync` is
 * module-level and keyed by session id, so a reused id sends `loadSession` down
 * the CACHED path and it never reaches the resume that names the live turn.
 */
let sessionSeq = 0;

beforeEach(() => {
  mocks.reply.mockReset();
  mocks.observeSessionEvents.mockReset();
  mocks.resumeAgent.mockReset();
  mocks.updateFromSession.mockClear();
  Object.assign(window, {
    electron: {
      // What `utils/userAction.ts` reads. The real bridge is the Electron
      // preload; the key itself never enters the daemon's environment, which is
      // the whole reason the proof means anything (AR-11).
      getUserActionKey: vi.fn(async () => USER_ACTION_KEY),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

describe('gated routes carry the user-action proof', () => {
  it('on both resume phases and the session-to-agent update', async () => {
    const sid = `ua-resume-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: session(sid) } });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await vi.waitFor(() => expect(mocks.resumeAgent).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(mocks.updateFromSession).toHaveBeenCalledTimes(1));

    expect(
      mocks.resumeAgent.mock.calls.map(
        (call) => (call[0] as { headers?: Record<string, string> }).headers
      )
    ).toEqual([{ 'X-User-Action': USER_ACTION_KEY }, { 'X-User-Action': USER_ACTION_KEY }]);
    expect(
      (mocks.updateFromSession.mock.calls[0][0] as { headers?: Record<string, string> }).headers
    ).toEqual({ 'X-User-Action': USER_ACTION_KEY });
  });

  it('on the attach that rejoins a live turn after a reload', async () => {
    const sid = `ua-attach-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: session(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.attachToTurn('turn-9');

    expect(mocks.reply).toHaveBeenCalledTimes(1);
    expect(gatedCallHeaders()).toEqual([{ 'X-User-Action': USER_ACTION_KEY }]);
  });

  it('on the reattach that follows a dropped stream', async () => {
    const sid = `ua-reattach-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: session(sid) } });
    // First stream ends with NO `Finish` — a dropped socket, which is what
    // `reattachAfterDrop` exists for. The replacement completes the turn.
    mocks.reply
      .mockResolvedValueOnce({ stream: streamOf() })
      .mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.attachToTurn('turn-9');

    // The reattach happened at all — without it this test would pass vacuously
    // on the attach's own header.
    expect(mocks.reply).toHaveBeenCalledTimes(2);
    expect(gatedCallHeaders()).toEqual([
      { 'X-User-Action': USER_ACTION_KEY },
      { 'X-User-Action': USER_ACTION_KEY },
    ]);
  });

  it('on the observer feed a daemon-opened tab subscribes to', async () => {
    const sid = `ua-observe-${++sessionSeq}`;
    mocks.observeSessionEvents.mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    const loop = controller.observeSession();
    // One connect + drain. `observeSession` is deliberately non-terminating, so
    // it is stopped rather than awaited to completion.
    await vi.waitFor(() => expect(mocks.observeSessionEvents).toHaveBeenCalled());
    controller.stopObserving();
    await loop;

    expect(gatedCallHeaders()[0]).toEqual({ 'X-User-Action': USER_ACTION_KEY });
  });

  it('on a user submit, which is where the proof was already correct', async () => {
    const sid = `ua-submit-${++sessionSeq}`;
    mocks.resumeAgent.mockResolvedValue({ data: { session: session(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.handleSubmit('hello');

    expect(gatedCallHeaders()).toEqual([{ 'X-User-Action': USER_ACTION_KEY }]);
  });

  it('sends no forged proof when there is no bridge to read the key from', async () => {
    // A surface with no preload (the browser harness, a future headless view).
    // `userActionHeaders` fails CLOSED — it sends nothing, so the daemon refuses
    // and explains, rather than the renderer inventing a value that would be
    // compared against the installed digest and could only ever be wrong.
    const sid = `ua-nobridge-${++sessionSeq}`;
    Object.assign(window, { electron: { showNotification: vi.fn(), logInfo: vi.fn() } });
    mocks.resumeAgent.mockResolvedValue({ data: { session: session(sid) } });
    mocks.reply.mockResolvedValue({ stream: streamOf(finishFrame) });

    const controller = new ChatStreamRegistry().getController(sid);
    await controller.loadSession();
    await controller.attachToTurn('turn-9');

    expect(gatedCallHeaders()).toEqual([{}]);
  });
});
