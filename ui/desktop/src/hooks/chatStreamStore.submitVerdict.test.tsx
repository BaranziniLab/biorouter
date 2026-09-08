/**
 * The submit's VERDICT: the return value a caller holding the user's words
 * must obey.
 *
 * `handleSubmit` holds `submitInFlight` across the whole turn and releases it in
 * a `finally` that runs only once the turn's promise chain unwinds. The
 * `ChatState.Idle` flush that tells the composer its queue may drain is produced
 * INSIDE that chain (`finishCurrentStream`), so the drain's submit reliably
 * arrives while the latch is still set. That return used to be a bare `return`,
 * indistinguishable from a submit that worked, and the drain dequeued on it: the
 * user's queued message disappeared with no error.
 *
 * These tests pin the two answers a caller can act on, in the ordering that
 * produced the bug rather than in a synthetic one.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatStreamRegistry } from './chatStreamStore';
import { ChatState } from '../types/chatState';
import type { Message, MessageEvent, Session, TokenState } from '../api';
import { reply, resumeAgent } from '../api';

vi.mock('../api', () => ({
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
}));

const tokenState: TokenState = {
  accumulatedInputTokens: 0,
  accumulatedOutputTokens: 0,
  accumulatedTotalTokens: 0,
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
};

let sessionSeq = 0;
let SID = 'v0';

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

const finishFrame = { type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent;

/** One short turn: a single Finish frame, delivered on its own macrotask. */
function servingOneTurn() {
  vi.mocked(reply).mockResolvedValue({
    stream: (async function* () {
      await new Promise((resolve) => setTimeout(resolve, 0));
      yield finishFrame;
    })(),
  } as never);
}

beforeEach(() => {
  SID = `verdict-s${++sessionSeq}`;
  vi.mocked(reply).mockReset();
  vi.mocked(resumeAgent).mockReset();
  Object.assign(window, { electron: { showNotification: vi.fn(), logInfo: vi.fn() } });
});

describe('handleSubmit reports whether it took the message', () => {
  it('resolves true for a submit that launches a turn, and posts exactly one reply', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingOneTurn();

    const controller = registry.getController(SID);
    await controller.loadSession();

    await expect(controller.handleSubmit('the only message')).resolves.toBe(true);
    expect(reply).toHaveBeenCalledTimes(1);
  });

  it('resolves FALSE for the submit a queue drain makes as the turn ends', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    servingOneTurn();

    const controller = registry.getController(SID);
    await controller.loadSession();

    // Stand in for ChatInput's drain effect: it runs off the Idle notification,
    // which the store flushes SYNCHRONOUSLY from inside the first submit's own
    // promise chain. Nothing here fakes that ordering: subscribing is what the
    // composer does, and this is when it hears.
    let drained: Promise<boolean> | null = null;
    const unsubscribe = controller.subscribe(() => {
      if (drained || controller.getSnapshot().chatState !== ChatState.Idle) return;
      drained = controller.handleSubmit('typed while the turn was running');
    });

    await controller.handleSubmit('the first message');
    unsubscribe();

    expect(drained, 'the drain never ran, so this test is not measuring the bug').not.toBeNull();
    await expect(drained!).resolves.toBe(false);
    // The refused submit really did send nothing: a caller that dequeues on this
    // answer has thrown the user's message away.
    expect(reply).toHaveBeenCalledTimes(1);
  });

  it('resolves false when the controller has no session to submit into', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockRejectedValue(new Error('backend down'));

    const controller = registry.getController(SID);

    await expect(controller.handleSubmit('nowhere to go')).resolves.toBe(false);
    expect(reply).not.toHaveBeenCalled();
  });
});
