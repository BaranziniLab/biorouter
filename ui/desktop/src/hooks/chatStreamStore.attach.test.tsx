/**
 * Attaching to a turn this client did not start — the renderer half of the
 * live turn stream contract (`.stream-contract.md`).
 *
 * These tests are aimed at where the bugs in this feature actually live, which
 * is NOT "did the frames arrive". A client that renders a re-attached turn
 * badly is worse than one that drops it, because the failure is silent and
 * looks like the model repeating itself. So the assertions here are mostly
 * about what the user SEES:
 *
 *   - the turn appears exactly once, in order (no duplicated paragraph);
 *   - the backlog lands as ONE render (no re-typing at the original speed);
 *   - the transcript is not remounted and the reader's scroll is not moved;
 *   - a failed attach is silent (no invented error card).
 */
import React from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render } from '@testing-library/react';
import { ChatState } from '../types/chatState';
import { ChatStreamRegistry, REPLAY_MAX_HOLD_MS } from './chatStreamStore';
import type { Message, MessageEvent, Session, TokenState } from '../api';
import { reply, resumeAgent } from '../api';

vi.mock('../api', async () => {
  return {
    cancelTurn: vi.fn(async () => ({ data: { cancelled: true } })),
    editMessage: vi.fn(),
    getSession: vi.fn(async () => ({ data: null })),
    interrupt: vi.fn(),
    listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
    observeSessionEvents: vi.fn(),
    reply: vi.fn(),
    resumeAgent: vi.fn(),
    updateFromSession: vi.fn(async () => ({ data: {} })),
    updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
  };
});

const TURN = 'turn-under-test';

/**
 * A fresh session id per test. The transcript LRU (`utils/sessionNameSync`) is
 * module-level and keyed by session id, so a reused id makes `loadSession` take
 * the CACHED path and inherit the previous test's transcript — which shows up
 * as a mysterious pile of unrelated messages rather than as a cache hit.
 */
let sessionSeq = 0;
let SID = 's0';

const tokenState: TokenState = {
  accumulatedInputTokens: 0,
  accumulatedOutputTokens: 0,
  accumulatedTotalTokens: 0,
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
};

function session(id: string, conversation: Message[] = []): Session {
  return {
    id,
    name: `Session ${id}`,
    working_dir: '/tmp',
    conversation,
    message_count: conversation.length,
    total_tokens: 0,
    created_at: '',
    updated_at: '',
    extension_data: {},
    user_set_name: false,
  } as Session;
}

/**
 * What `POST /agent/resume` answers with. `active_turn` is the authoritative —
 * and only — way a client learns that a turn is running for this session: the
 * server filters it on `finished_at.is_none()`, so it is present exactly when
 * there is something to rejoin.
 */
function resumeResponse(conversation: Message[] = [], activeTurnId?: string) {
  return {
    data: {
      session: session(SID, conversation),
      ...(activeTurnId ? { active_turn: { turn_id: activeTurnId } } : {}),
    },
  } as never;
}

function userMessage(id: string, text: string): Message {
  return {
    id,
    role: 'user',
    created: 1,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

function assistantMessage(id: string, text: string): Message {
  return {
    id,
    role: 'assistant',
    created: 1,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

/** The shape the server puts on the wire: the MessageEvent plus the contract's
 * three envelope fields (`turn_stream.rs`, "Wire format"). */
type WireFrame = MessageEvent & { seq?: number; turn_id?: string; replay?: true };

function envelope(event: MessageEvent, seq: number, turnId: string, replay?: true): WireFrame {
  return { ...event, seq, turn_id: turnId, ...(replay ? { replay } : {}) } as unknown as WireFrame;
}

function replayed(seq: number, event: MessageEvent): WireFrame {
  return envelope(event, seq, TURN, true);
}

function live(seq: number, event: MessageEvent): WireFrame {
  return envelope(event, seq, TURN);
}

function messageFrame(id: string, text: string): MessageEvent {
  return {
    type: 'Message',
    message: assistantMessage(id, text),
    token_state: tokenState,
  } as MessageEvent;
}

const finishFrame = { type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent;

function persistedFrame(id: string): MessageEvent {
  return {
    type: 'MessagesPersisted',
    messages: [{ id, userVisible: true }],
  } as MessageEvent;
}

/** Serve these frames as the `{ stream }` a `/reply` POST resolves to. */
function servingFrames(...frames: WireFrame[]) {
  vi.mocked(reply).mockResolvedValue({
    stream: (async function* () {
      for (const frame of frames) yield frame as MessageEvent;
    })(),
  } as never);
}

/**
 * How far apart a slowly-drained backlog's frames arrive, in ms.
 *
 * It has to exceed the store's own notification batching window
 * (`NOTIFY_FALLBACK_MS`, ~two animation frames) or the test proves nothing: a
 * backlog whose frames all land inside one frame is coalesced by the #22
 * scheduler whether or not a replay hold exists. A real backlog is not that
 * kind: it is many socket reads and a lot of parsing and React work, so it
 * drains over many frames, which is exactly when a naive implementation re-types
 * the message in front of the user.
 */
const SLOW_DRAIN_MS = 50;

/** Serve the frames spread over several notification windows. */
function servingFramesSlowly(...frames: WireFrame[]) {
  vi.mocked(reply).mockResolvedValue({
    stream: (async function* () {
      for (const frame of frames) {
        await new Promise((resolve) => setTimeout(resolve, SLOW_DRAIN_MS));
        yield frame as MessageEvent;
      }
    })(),
  } as never);
}

/** All text rendered in the transcript, flattened, in order. */
function transcriptText(messages: Message[]): string[] {
  return messages.flatMap((m) => m.content.flatMap((c) => (c.type === 'text' ? [c.text] : [])));
}

function replyBody(): Record<string, unknown> {
  const calls = vi.mocked(reply).mock.calls;
  const call = calls[calls.length - 1][0] as { body: Record<string, unknown> };
  return call.body;
}

beforeEach(() => {
  SID = `attach-s${++sessionSeq}`;
  vi.mocked(resumeAgent).mockReset();
  vi.mocked(reply).mockReset();
  Object.assign(window, {
    electron: {
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('attaching to a turn this client did not start', () => {
  it('renders the whole turn, exactly once and in order', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session(SID, [userMessage('u1', 'run the analysis')]) },
    } as never);
    servingFrames(
      replayed(0, messageFrame('a1', 'First I will load the data.')),
      replayed(1, messageFrame('a2', 'Now the differential expression.')),
      live(2, messageFrame('a3', 'Done — 412 genes.')),
      live(3, finishFrame)
    );

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.attachToTurn(TURN);

    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'run the analysis',
      'First I will load the data.',
      'Now the differential expression.',
      'Done — 412 genes.',
    ]);
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
  });

  it('re-POSTs the same turn id rather than starting a new turn', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingFrames(live(0, finishFrame));

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.attachToTurn(TURN);

    expect(replyBody()).toMatchObject({ session_id: SID, turn_id: TURN, from_seq: 0 });
  });

  it('does not re-render frames it has already applied', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session(SID, [userMessage('u1', 'go')]) },
    } as never);
    // First attach: the client sees the first half of the turn and the stream
    // then dies without a terminal frame.
    servingFrames(
      replayed(0, messageFrame('a1', 'step one')),
      replayed(1, messageFrame('a2', 'step two'))
    );

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.attachToTurn(TURN);

    // Second attach: the server replays the turn from its start, as it is
    // entitled to. Frames 0 and 1 must be dropped on the floor.
    servingFrames(
      replayed(0, messageFrame('a1', 'step one')),
      replayed(1, messageFrame('a2', 'step two')),
      replayed(2, messageFrame('a3', 'step three')),
      live(3, finishFrame)
    );
    await controller.attachToTurn(TURN);

    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'go',
      'step one',
      'step two',
      'step three',
    ]);
  });

  it('asks only for what it is missing', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingFrames(replayed(0, messageFrame('a1', 'one')), replayed(1, messageFrame('a2', 'two')));

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.attachToTurn(TURN);

    servingFrames(live(2, finishFrame));
    await controller.attachToTurn(TURN);

    // Two frames applied (seq 0 and 1), so the next one wanted is 2.
    expect(replyBody()).toMatchObject({ turn_id: TURN, from_seq: 2 });
  });

  it('starts a new turn from zero rather than mistaking its frames for replays', async () => {
    // `seq` restarts at 0 every turn. Without the turn scoping on the gate,
    // every frame of the next turn would be at or below the previous turn's
    // high-water mark and the whole turn would vanish.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingFrames(
      replayed(0, messageFrame('a1', 'first turn')),
      replayed(1, messageFrame('a2', 'still first')),
      live(2, finishFrame)
    );

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.attachToTurn(TURN);

    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield envelope(messageFrame('b1', 'second turn'), 0, 'turn-two') as MessageEvent;
        yield envelope(finishFrame, 1, 'turn-two') as MessageEvent;
      })(),
    } as never);
    await controller.attachToTurn('turn-two');

    expect(transcriptText(controller.getSnapshot().messages)).toContain('second turn');
  });

  it('refuses to attach on top of a turn it is already driving', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    let release = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        await held;
        yield live(0, finishFrame) as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController(SID);
    await controller.loadSession();
    const running = controller.handleSubmit('hello');
    await vi.waitFor(() => expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming));

    const callsBefore = vi.mocked(reply).mock.calls.length;
    expect(callsBefore).toBe(1);
    await expect(controller.attachToTurn('some-other-turn')).resolves.toBe(false);
    expect(vi.mocked(reply).mock.calls.length).toBe(callsBefore);

    release();
    await running;
  });

  it('says nothing when the turn it tried to join is already over', async () => {
    // A pointer left behind by a window that died. The turn finished while the
    // window was gone; joining it fails, and that is not an error the user did
    // anything to cause — a red card here would be an invented failure.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session(SID, [userMessage('u1', 'done long ago')]) },
    } as never);
    vi.mocked(reply).mockRejectedValue(new Error('409 no such turn'));

    const controller = registry.getController(SID);
    await controller.loadSession();
    await expect(controller.attachToTurn(TURN)).resolves.toBe(false);

    expect(controller.getSnapshot().turnError).toBeUndefined();
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    expect(transcriptText(controller.getSnapshot().messages)).toEqual(['done long ago']);
  });

  it('does not mistake stored assistant text for proof that a rejoined turn succeeded', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: {
        session: session(SID, [
          userMessage('u1', 'delegate and wait'),
          assistantMessage('a-final', 'the collected child result'),
        ]),
      },
    } as never);
    servingFrames(
      live(0, {
        type: 'Error',
        error: 'The model turn ended unexpectedly. Please retry.',
        code: 'internal_error',
        scope: 'internal',
        retryable: true,
      } as MessageEvent)
    );

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.attachToTurn(TURN);

    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'delegate and wait',
      'the collected child result',
    ]);
    expect(controller.getSnapshot().turnError).toMatchObject({
      code: 'internal_error',
      retryable: true,
    });
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
  });

  it('rejoins a turn left running by a window that reloaded', async () => {
    // The reload case, end to end. The transcript comes back from the store
    // stopping at the user's message — a live turn has persisted nothing yet —
    // and `active_turn` on the same response is what fills in the rest.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue(resumeResponse([userMessage('u1', 'long job')], TURN));
    servingFrames(replayed(0, messageFrame('a1', 'still working')), live(1, finishFrame));

    const controller = registry.getController(SID);
    await controller.loadSession();
    // `loadSession` fires the attach without awaiting it: the transcript must
    // not wait behind a network round trip.
    await vi.waitFor(() =>
      expect(transcriptText(controller.getSnapshot().messages)).toContain('still working')
    );
    expect(replyBody()).toMatchObject({ turn_id: TURN, from_seq: 0 });
    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'long job',
      'still working',
    ]);
  });

  it('rejoins a turn it never started, under the name the server gave it', async () => {
    // The reloaded window has never seen the idempotency key the original
    // caller minted — that lived in a renderer heap which is gone. All it has
    // is the server's own `turn-N`, and posting THAT must attach rather than
    // 409. (Same path a turn started by the CLI or a scheduled run takes: this
    // client did not start it and could not have guessed its key.)
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue(resumeResponse([userMessage('u1', 'go')], 'turn-7'));
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield envelope(
          messageFrame('a1', 'output from elsewhere'),
          0,
          'turn-7',
          true
        ) as MessageEvent;
        yield envelope(finishFrame, 1, 'turn-7') as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController(SID);
    await controller.loadSession();
    await vi.waitFor(() =>
      expect(transcriptText(controller.getSnapshot().messages)).toContain('output from elsewhere')
    );
    expect(replyBody()).toMatchObject({ turn_id: 'turn-7' });
  });

  it('rejoins only once, however many times the session is loaded', async () => {
    // `loadSession` is a no-op on an already-painted session but is still
    // awaited by every submit, and `ensureAgentLoaded`'s resume reports
    // `active_turn` too. Neither may start a second attach to the same turn.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue(resumeResponse([], TURN));
    servingFrames(replayed(0, messageFrame('a1', 'working')), live(1, finishFrame));

    const controller = registry.getController(SID);
    await controller.loadSession();
    await vi.waitFor(() => expect(vi.mocked(reply)).toHaveBeenCalledTimes(1));
    await controller.loadSession();
    await controller.loadSession();
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(vi.mocked(reply)).toHaveBeenCalledTimes(1);
  });

  it('never eats a message typed while it is still trying to rejoin', async () => {
    // The attach is automatic and can fail; the message is not and must not.
    // If the attach claimed "a turn is in flight" before it knew one was, the
    // submit below would be refused SILENTLY — no error, no bubble, nothing.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue(resumeResponse([], TURN));

    let releaseAttachPost = () => {};
    const attachPostHeld = new Promise<void>((resolve) => {
      releaseAttachPost = resolve;
    });
    vi.mocked(reply)
      // The attach POST: parked, as a real one is for a round trip.
      .mockImplementationOnce(async () => {
        await attachPostHeld;
        throw new Error('409 that turn is over');
      })
      // The user's own turn.
      .mockResolvedValueOnce({
        stream: (async function* () {
          yield {
            type: 'Message',
            message: assistantMessage('a1', 'answering the new question'),
            token_state: tokenState,
          } as MessageEvent;
          yield { type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent;
        })(),
      } as never);

    const controller = registry.getController(SID);
    await controller.loadSession();
    await vi.waitFor(() => expect(vi.mocked(reply)).toHaveBeenCalledTimes(1));

    // Typed while the attach POST is still outstanding.
    await controller.handleSubmit('a brand new question');
    releaseAttachPost();
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'a brand new question',
      'answering the new question',
    ]);
    expect(controller.getSnapshot().turnError).toBeUndefined();
  });

  it('does not chase a turn when the server reports none', async () => {
    // No `active_turn` means no turn is running — the server filters it on
    // `finished_at.is_none()`. Nothing to rejoin, so nothing is POSTed; there
    // is no local guess left that could disagree.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue(resumeResponse());

    const controller = registry.getController(SID);
    await controller.loadSession();
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(vi.mocked(reply)).not.toHaveBeenCalled();
  });
});

describe('a dropped socket is not a dead turn', () => {
  /** A stream that yields these frames and then simply ENDS, with no `Finish`
   * and no error — what an SSE connection that gave up looks like from here. */
  function droppingAfter(...frames: WireFrame[]) {
    return {
      stream: (async function* () {
        for (const frame of frames) yield frame as MessageEvent;
      })(),
    };
  }

  it('picks the turn back up instead of showing a failure', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    vi.mocked(reply)
      .mockResolvedValueOnce(
        droppingAfter(
          live(0, messageFrame('a1', 'half an answer')),
          live(1, messageFrame('a2', 'and a bit more'))
        ) as never
      )
      .mockResolvedValueOnce({
        stream: (async function* () {
          yield replayed(2, messageFrame('a3', 'the rest of it')) as MessageEvent;
          yield live(3, finishFrame) as MessageEvent;
        })(),
      } as never);

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.handleSubmit('do the thing');

    // No error card: the turn was never lost, so there is nothing to report.
    expect(controller.getSnapshot().turnError).toBeUndefined();
    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'do the thing',
      'half an answer',
      'and a bit more',
      'the rest of it',
    ]);
    // The second POST rejoined the SAME turn the first one started, asking only
    // for the frames it had not seen.
    const first = vi.mocked(reply).mock.calls[0][0] as { body: Record<string, unknown> };
    const second = vi.mocked(reply).mock.calls[1][0] as { body: Record<string, unknown> };
    expect(second.body.turn_id).toBe(first.body.turn_id);
    expect(second.body.from_seq).toBe(2);
  });

  it('does not mistake a persisted assistant row for an authoritative terminal', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingFrames(
      live(0, messageFrame('a-final', 'the completed answer')),
      live(1, persistedFrame('a-final')),
      live(2, {
        type: 'Error',
        error: 'The model turn ended unexpectedly. Please retry.',
        code: 'internal_error',
        scope: 'internal',
        retryable: true,
      } as MessageEvent)
    );

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.handleSubmit('do the thing');

    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'do the thing',
      'the completed answer',
    ]);
    expect(controller.getSnapshot().turnError).toMatchObject({
      code: 'internal_error',
      retryable: true,
    });
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
  });

  it('still reports a failure once the turn really is unreachable', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    vi.mocked(reply).mockImplementation(
      async () =>
        ({
          stream: (async function* () {
            // A rendered and persisted fragment is still not proof of a
            // completed turn. Every stream ends without an authoritative
            // lifecycle terminal.
            yield live(0, messageFrame('a-partial', 'partial output')) as MessageEvent;
            yield live(1, persistedFrame('a-partial')) as MessageEvent;
          })(),
        }) as never
    );

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.handleSubmit('do the thing');

    expect(controller.getSnapshot().turnError).toMatchObject({
      code: 'stream_interrupted',
      retryable: true,
    });
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    // Bounded: it does not sit there re-POSTing forever.
    expect(vi.mocked(reply).mock.calls.length).toBeLessThanOrEqual(4);
  });
});

describe('the replay must be invisible', () => {
  it('paints a backlog in one render, however many reads it arrives over', async () => {
    vi.useFakeTimers();
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingFramesSlowly(
      replayed(0, messageFrame('a1', 'one')),
      replayed(1, messageFrame('a2', 'two')),
      replayed(2, messageFrame('a3', 'three')),
      replayed(3, messageFrame('a4', 'four')),
      replayed(4, messageFrame('a5', 'five')),
      live(5, finishFrame)
    );

    const controller = registry.getController(SID);
    await controller.loadSession();
    const attaching = controller.attachToTurn(TURN);
    // Past `chatState: Streaming` and its notification, so what is counted
    // below is the BACKLOG's cost and nothing else.
    await vi.advanceTimersByTimeAsync(SLOW_DRAIN_MS - 1);

    let renders = 0;
    controller.subscribe(() => {
      renders += 1;
    });

    // Drain the whole backlog. Each frame lands in its own notification window
    // — five separate paints if nothing holds them together.
    await vi.advanceTimersByTimeAsync(SLOW_DRAIN_MS * 6 + 100);
    await attaching;

    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'one',
      'two',
      'three',
      'four',
      'five',
    ]);
    // One paint for the backlog, one for the turn ending. Not five.
    expect(renders).toBeLessThanOrEqual(2);
  });

  it('keeps the live tail frame by frame — only the backlog is batched', async () => {
    vi.useFakeTimers();
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingFramesSlowly(
      replayed(0, messageFrame('a1', 'backlog one')),
      replayed(1, messageFrame('a2', 'backlog two')),
      live(2, messageFrame('a3', 'live one')),
      live(3, messageFrame('a4', 'live two')),
      live(4, messageFrame('a5', 'live three'))
    );

    const controller = registry.getController(SID);
    await controller.loadSession();
    const attaching = controller.attachToTurn(TURN);
    await vi.advanceTimersByTimeAsync(SLOW_DRAIN_MS - 1);

    let renders = 0;
    controller.subscribe(() => {
      renders += 1;
    });
    await vi.advanceTimersByTimeAsync(SLOW_DRAIN_MS * 6 + 100);
    await attaching;

    expect(transcriptText(controller.getSnapshot().messages)).toEqual([
      'backlog one',
      'backlog two',
      'live one',
      'live two',
      'live three',
    ]);
    // The hold is released at the replay/live boundary, not at the end of the
    // stream: a live turn that streams for minutes must not be invisible for
    // minutes. So the three live frames each paint on their own — which is
    // strictly MORE renders than the two backlog frames cost together.
    expect(renders).toBeGreaterThanOrEqual(3);
  });

  it('releases the transcript even if the backlog never ends', async () => {
    // The safety valve. A producer that marks everything `replay` (a bug, or a
    // pathologically long backlog) must degrade to progressive rendering, never
    // to a frozen transcript.
    vi.useFakeTimers();
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    let seq = 0;
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        for (;;) {
          await new Promise((resolve) => setTimeout(resolve, 50));
          yield replayed(seq, messageFrame(`a${seq}`, `frame ${seq}`)) as MessageEvent;
          seq += 1;
        }
      })(),
    } as never);

    const controller = registry.getController(SID);
    await controller.loadSession();
    void controller.attachToTurn(TURN);

    let renders = 0;
    await vi.advanceTimersByTimeAsync(REPLAY_MAX_HOLD_MS + 100);
    controller.subscribe(() => {
      renders += 1;
    });
    await vi.advanceTimersByTimeAsync(500);

    expect(renders).toBeGreaterThan(0);
    controller.stopStreaming();
  });

  it('does not restart the elapsed-time origin when it rejoins a turn', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingFrames(replayed(0, messageFrame('a1', 'one')));

    const controller = registry.getController(SID);
    await controller.loadSession();
    await controller.attachToTurn(TURN);
    const first = controller.getSnapshot().turnStartedAt;

    servingFrames(replayed(1, messageFrame('a2', 'two')));
    await controller.attachToTurn(TURN);

    expect(controller.getSnapshot().turnStartedAt).toBe(first);
  });
});

describe('the reader must not be moved', () => {
  /** A transcript, deliberately minimal: one DOM node per message plus a
   * scrollable viewport, which is all that is needed to detect a remount or a
   * scroll reset. */
  function Transcript({
    controller,
  }: {
    controller: ReturnType<ChatStreamRegistry['getController']>;
  }) {
    const [snapshot, setSnapshot] = React.useState(controller.getSnapshot());
    React.useEffect(
      () => controller.subscribe(() => setSnapshot(controller.getSnapshot())),
      [controller]
    );
    return (
      <div data-testid="viewport" style={{ overflowY: 'auto' }}>
        {snapshot.messages.map((m, i) => (
          <p key={m.id ?? i} data-testid={`msg-${m.id}`}>
            {m.content.map((c) => (c.type === 'text' ? c.text : '')).join('')}
          </p>
        ))}
      </div>
    );
  }

  it('does not remount the transcript or reset scroll when a turn is rejoined', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: {
        session: session(SID, [
          userMessage('u1', 'earlier question'),
          assistantMessage('a0', 'earlier answer'),
        ]),
      },
    } as never);
    servingFrames(
      replayed(0, messageFrame('a1', 'new output')),
      live(1, messageFrame('a2', 'more output')),
      live(2, finishFrame)
    );

    const controller = registry.getController(SID);
    await controller.loadSession();

    const view = render(<Transcript controller={controller} />);
    const before = view.getByTestId('msg-u1');
    const viewport = view.getByTestId('viewport');
    // jsdom has no layout, so scrollTop is a plain property — which is exactly
    // what a remount would reset and an in-place update would not.
    viewport.scrollTop = 240;

    await act(async () => {
      await controller.attachToTurn(TURN);
    });

    expect(view.getByTestId('msg-u1')).toBe(before);
    expect(view.getByTestId('viewport')).toBe(viewport);
    expect(viewport.scrollTop).toBe(240);
    expect(view.getByTestId('msg-a1').textContent).toBe('new output');
  });

  it('never shows an empty transcript in the middle of rejoining', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session(SID, [userMessage('u1', 'earlier question')]) },
    } as never);
    servingFramesSlowly(
      replayed(0, messageFrame('a1', 'one')),
      replayed(1, messageFrame('a2', 'two')),
      live(2, finishFrame)
    );

    const controller = registry.getController(SID);
    await controller.loadSession();

    const seen: number[] = [];
    controller.subscribe(() => seen.push(controller.getSnapshot().messages.length));

    await controller.attachToTurn(TURN);

    // Every state the user could have been shown still had the conversation in
    // it. A flash of nothing is a bug even if the end state is right.
    expect(seen.every((n) => n >= 1)).toBe(true);
  });
});
