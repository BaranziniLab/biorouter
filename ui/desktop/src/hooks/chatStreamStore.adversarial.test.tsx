/**
 * ADVERSARIAL probes against the live-turn-stream renderer half.
 *
 * Each test here is written to FAIL against the current implementation and to
 * describe, in its assertion, the user-visible symptom it proves.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatStreamRegistry } from './chatStreamStore';
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

type WireFrame = MessageEvent & { seq?: number; turn_id?: string; replay?: true };

function envelope(event: MessageEvent, seq: number, turnId: string, replay?: true): WireFrame {
  return { ...event, seq, turn_id: turnId, ...(replay ? { replay } : {}) } as unknown as WireFrame;
}
const replayed = (seq: number, e: MessageEvent) => envelope(e, seq, TURN, true);
const live = (seq: number, e: MessageEvent) => envelope(e, seq, TURN);

function messageFrame(id: string, text: string): MessageEvent {
  return {
    type: 'Message',
    message: assistantMessage(id, text),
    token_state: tokenState,
  } as MessageEvent;
}
const finishFrame = { type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent;

/** Yield each frame after its own delay, so frames land in separate macrotasks. */
function servingPaced(frames: { frame: WireFrame; delayMs: number }[]) {
  vi.mocked(reply).mockResolvedValue({
    stream: (async function* () {
      for (const { frame, delayMs } of frames) {
        await new Promise((resolve) => setTimeout(resolve, delayMs));
        yield frame as MessageEvent;
      }
    })(),
  } as never);
}

beforeEach(() => {
  SID = `adv-s${++sessionSeq}`;
  vi.mocked(resumeAgent).mockReset();
  vi.mocked(reply).mockReset();
  Object.assign(window, { electron: { showNotification: vi.fn(), logInfo: vi.fn() } });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('the live tail after a replay hold', () => {
  /**
   * DEFECT: `flushNotify()` returns EARLY when `notifySuspendDepth > 0` without
   * clearing `notifyScheduled`. A replay hold that outlives the rAF **and** the
   * `NOTIFY_FALLBACK_MS` timeout armed before it therefore consumes both arms of
   * the race and leaves `notifyScheduled === true` forever. From then on every
   * `scheduleNotify()` early-returns, so React is never told about anything
   * again: the snapshot keeps advancing, the screen does not.
   *
   * The user-visible symptom: re-attach to a running turn, watch the backlog
   * paint, then watch the transcript FREEZE for the rest of the turn. Everything
   * appears at once when `Finish` arrives (`finishCurrentStream` calls
   * `flushNotify` with the hold released, which unlatches it).
   */
  it('keeps notifying React after the backlog commits', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session(SID, [userMessage('u1', 'go')]) },
    } as never);

    // 60 ms is longer than one rAF (~16 ms) AND longer than NOTIFY_FALLBACK_MS
    // (32 ms), so the hold opened by the replay frame swallows both arms of the
    // notification race that `attachToTurn` armed just before it.
    servingPaced([
      { frame: replayed(0, messageFrame('a1', 'backlog')), delayMs: 0 },
      { frame: live(1, messageFrame('a2', ' one')), delayMs: 60 },
      { frame: live(2, messageFrame('a2', ' two')), delayMs: 60 },
      { frame: live(3, messageFrame('a2', ' three')), delayMs: 60 },
      { frame: live(4, finishFrame), delayMs: 60 },
    ]);

    const controller = registry.getController(SID);
    await controller.loadSession();

    let notifications = 0;
    const unsubscribe = controller.subscribe(() => {
      notifications += 1;
    });

    // Count only what happens AFTER the hold has released — i.e. while the live
    // tail is streaming. Sample midway through the turn, not at its end.
    let duringLiveTail = 0;
    const sample = setTimeout(() => {
      notifications = 0;
    }, 100);
    const measure = setTimeout(() => {
      duringLiveTail = notifications;
    }, 230);

    await controller.attachToTurn(TURN);
    clearTimeout(sample);
    clearTimeout(measure);
    unsubscribe();

    expect(
      duringLiveTail,
      'the live tail after a replay hold must still reach React; 0 means the transcript is frozen until the turn ends'
    ).toBeGreaterThan(0);
  });

  /**
   * CONTROL for the test above: the identical stream with NO replay frame — so
   * no hold is ever opened — does notify during the live tail. Proves the
   * freeze is caused by the replay hold and not by the pacing of the harness.
   */
  it('control: an all-live stream notifies during its tail', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session(SID, [userMessage('u1', 'go')]) },
    } as never);
    servingPaced([
      { frame: live(0, messageFrame('a1', 'backlog')), delayMs: 0 },
      { frame: live(1, messageFrame('a2', ' one')), delayMs: 60 },
      { frame: live(2, messageFrame('a2', ' two')), delayMs: 60 },
      { frame: live(3, messageFrame('a2', ' three')), delayMs: 60 },
      { frame: live(4, finishFrame), delayMs: 60 },
    ]);

    const controller = registry.getController(SID);
    await controller.loadSession();
    let notifications = 0;
    const unsubscribe = controller.subscribe(() => {
      notifications += 1;
    });
    let duringLiveTail = 0;
    const sample = setTimeout(() => {
      notifications = 0;
    }, 100);
    const measure = setTimeout(() => {
      duringLiveTail = notifications;
    }, 230);
    await controller.attachToTurn(TURN);
    clearTimeout(sample);
    clearTimeout(measure);
    unsubscribe();
    expect(duringLiveTail).toBeGreaterThan(0);
  });
});

describe('rejoining a MULTI-ROUND turn after a reload', () => {
  /**
   * DEFECT: the completed-turn path exists because "the client has already read
   * the turn back from the session store, so replaying it would re-render the
   * whole turn as duplicates" (`.stream-contract-wire.md`, outcome 3). The very
   * same overlap exists for a RUNNING turn and nothing handles it.
   *
   * `agent.rs` persists each agent-loop ITERATION's rows as it goes
   * (`add_message` per iteration, then `MessagesPersisted`). So a window that
   * reloads during round 2 of a tool-using turn reads a transcript that already
   * contains round 1 — and then attaches with `from_seq: 0` (its high-water
   * mark is -1, it has rendered nothing) and is replayed round 1's frames on
   * top of the copy it just loaded.
   *
   * `pushMessage` cannot save it: the replayed frames are DELTAS of a message
   * whose text is already complete in the transcript, so neither the
   * `startsWith` nor the `endsWith` guard matches and the delta is APPENDED.
   *
   * User-visible: the reloaded window shows the first half of the answer twice.
   */
  it('does not re-render the part of the turn the store already gave it', async () => {
    const registry = new ChatStreamRegistry();
    // What the store holds mid-turn: the prompt and the FIRST round's
    // assistant message, already persisted and already complete.
    vi.mocked(resumeAgent).mockResolvedValue({
      data: {
        session: session(SID, [
          userMessage('u1', 'go'),
          assistantMessage('a1', 'I loaded the data.'),
        ]),
        active_turn: { turn_id: TURN },
      },
    } as never);
    // What the server replays: the turn from seq 0, i.e. round 1's deltas again.
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield replayed(0, messageFrame('a1', 'I loaded ')) as MessageEvent;
        yield replayed(1, messageFrame('a1', 'the data.')) as MessageEvent;
        yield live(2, messageFrame('a2', 'Round two.')) as MessageEvent;
        yield live(3, finishFrame) as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController(SID);
    await controller.loadSession();
    await vi.waitFor(() => expect(vi.mocked(reply).mock.calls.length).toBeGreaterThan(0));
    await vi.waitFor(() =>
      expect(
        controller
          .getSnapshot()
          .messages.flatMap((m) => m.content.flatMap((c) => (c.type === 'text' ? [c.text] : [])))
          .join('|')
      ).toContain('Round two.')
    );

    const texts = controller
      .getSnapshot()
      .messages.flatMap((m) => m.content.flatMap((c) => (c.type === 'text' ? [c.text] : [])));
    expect(texts, `transcript was: ${JSON.stringify(texts)}`).toEqual([
      'go',
      'I loaded the data.',
      'Round two.',
    ]);
  });
});
