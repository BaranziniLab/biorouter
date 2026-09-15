/**
 * "A steer always lands, and the spinner is honest" — the renderer half.
 *
 * Every test here was written against `main` first and failed there (D-numbers
 * refer to the defect list in the PR). Rendezvous are handshakes on streams the
 * test drives; fake timers only ever advance a backoff or a watchdog the code
 * under test owns — never a guess about how long something takes.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message, MessageEvent, Session, TokenState } from '../api';

const mocks = vi.hoisted(() => ({
  observeSessionEvents: vi.fn(),
  reply: vi.fn(),
  resumeAgent: vi.fn(),
  interrupt: vi.fn(),
  cancelTurn: vi.fn(),
  getSession: vi.fn(async () => ({ data: null })),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  updateFromSession: vi.fn(async () => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../api', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return { ...actual, ...mocks };
});

import { ChatStreamRegistry, isRunningState, STREAM_SILENCE_MS } from './chatStreamStore';
import { ChatState } from '../types/chatState';

vi.mock('../utils/userAction', async (importOriginal) => ({
  ...((await importOriginal()) as Record<string, unknown>),
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
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
function freshSessionId(prefix: string): string {
  sessionSeq += 1;
  return `${prefix}-${sessionSeq}-${Date.now()}`;
}

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

function userText(id: string, text: string): Message {
  return {
    id,
    role: 'user',
    created: 1,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

function assistantText(id: string, text: string): Message {
  return {
    id,
    role: 'assistant',
    created: 1,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

/** A stream whose frames and end are driven by the test. */
function createControlledStream() {
  const events: MessageEvent[] = [];
  let resolveNext: (() => void) | null = null;
  let closed = false;

  async function* stream() {
    while (!closed || events.length > 0) {
      if (events.length === 0) {
        await new Promise<void>((resolve) => {
          resolveNext = resolve;
        });
      }
      const event = events.shift();
      if (event) yield event;
    }
  }

  return {
    stream: stream(),
    push(event: MessageEvent) {
      events.push(event);
      resolveNext?.();
      resolveNext = null;
    },
    close() {
      closed = true;
      resolveNext?.();
      resolveNext = null;
    },
  };
}

beforeEach(() => {
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.getSession.mockResolvedValue({ data: null });
  mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });
  mocks.updateFromSession.mockResolvedValue({ data: {} });
  mocks.updateSessionUserWorkflowValues.mockResolvedValue({ data: {} });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('D1 — an observer never leaks the connection a finished turn used', () => {
  it('aborts each iteration’s own socket once its stream has been drained', async () => {
    vi.useFakeTimers();
    const sid = freshSessionId('observer-leak');
    const registry = new ChatStreamRegistry();
    const controller = registry.getController(sid);
    const first = createControlledStream();
    const second = createControlledStream();
    mocks.observeSessionEvents
      .mockResolvedValueOnce({ stream: first.stream })
      .mockResolvedValueOnce({ stream: second.stream });

    try {
      const running = controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1);

      first.push({ type: 'TurnStarted', turn_id: 'observed-1' } as unknown as MessageEvent);
      first.push({
        type: 'Message',
        message: assistantText('a1', 'observed prose'),
        token_state: tokenState,
      } as MessageEvent);
      first.push({
        type: 'Finish',
        reason: 'stop',
        token_state: tokenState,
        turn_id: 'observed-1',
      } as unknown as MessageEvent);
      // The daemon keeps the observer stream open after a Finish: the session
      // outlives its turn. `first` is deliberately NOT closed.
      await vi.advanceTimersByTimeAsync(0);
      await vi.advanceTimersByTimeAsync(1100); // the reconnect backoff
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);

      const signals = mocks.observeSessionEvents.mock.calls.map(
        (call) => (call[0] as { signal: AbortSignal }).signal
      );
      expect(signals[0].aborted, 'the drained stream’s socket must be closed').toBe(true);
      expect(signals.filter((signal) => !signal.aborted)).toHaveLength(1);

      controller.stopObserving();
      second.close();
      await vi.advanceTimersByTimeAsync(20_000);
      await running;
    } finally {
      vi.useRealTimers();
    }
  });
});

/** A controller with a user-driven turn in flight on a stream the test drives. */
async function drivingTurn(prefix: string, conversation: Message[] = []) {
  const sid = freshSessionId(prefix);
  const registry = new ChatStreamRegistry();
  const driving = createControlledStream();
  mocks.resumeAgent.mockResolvedValue({ data: { session: session(sid, conversation) } });
  mocks.reply.mockResolvedValueOnce({ stream: driving.stream });
  const controller = registry.getController(sid);
  const submit = controller.handleSubmit('plot the data');
  await vi.waitFor(() => expect(mocks.reply).toHaveBeenCalledTimes(1));
  const turnId = (mocks.reply.mock.calls[0][0] as { body: { turn_id: string } }).body.turn_id;
  return { sid, registry, controller, driving, submit, turnId };
}

describe('D5 — the controller owns a steer until the daemon answers it', () => {
  it('retries a steer that hit a network error, with the same key, and keeps the chip', async () => {
    const { controller, driving, submit } = await drivingTurn('steer-network');
    mocks.interrupt
      .mockRejectedValueOnce(new TypeError('Failed to fetch'))
      .mockResolvedValueOnce({ data: { turn_id: 'agent-turn-1' }, response: { status: 202 } });

    vi.useFakeTimers();
    try {
      const steering = controller.steer('actually, use R');
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().pendingSteer?.text).toBe('actually, use R');
      await vi.advanceTimersByTimeAsync(1_000); // the retry backoff
      await expect(steering).resolves.toBe(true);
    } finally {
      vi.useRealTimers();
    }

    expect(mocks.interrupt).toHaveBeenCalledTimes(2);
    const keys = mocks.interrupt.mock.calls.map(
      (call) => (call[0] as { body: { turn_id: string } }).body.turn_id
    );
    expect(keys[0]).toBe(keys[1]);
    expect(controller.getSnapshot().pendingSteer?.text).toBe('actually, use R');

    driving.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
    driving.close();
    await submit;
  });

  it('retries a steer whose POST never answers', async () => {
    const { controller, driving, submit } = await drivingTurn('steer-hung');
    mocks.interrupt
      .mockImplementationOnce(() => new Promise(() => {}))
      .mockResolvedValueOnce({ data: { turn_id: 'agent-turn-1' }, response: { status: 202 } });

    vi.useFakeTimers();
    try {
      const steering = controller.steer('actually, use R');
      await vi.advanceTimersByTimeAsync(8_000 + 1_000);
      await expect(steering).resolves.toBe(true);
    } finally {
      vi.useRealTimers();
    }
    expect(mocks.interrupt).toHaveBeenCalledTimes(2);

    driving.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
    driving.close();
    await submit;
  });
});

describe('D8 — a /reply the daemon refused is not a dropped connection', () => {
  it('gives the words back and attaches to the turn that holds the chat', async () => {
    const sid = freshSessionId('reply-409');
    const registry = new ChatStreamRegistry();
    // Another surface starts turn-7 in this chat after the window loaded and
    // before the person presses Send: only resumes after the refusal name it.
    let foreignTurnStarted = false;
    mocks.resumeAgent.mockImplementation(async () => ({
      data: foreignTurnStarted
        ? { session: session(sid), active_turn: { turn_id: 'turn-7' } }
        : { session: session(sid) },
    }));
    const foreign = createControlledStream();
    mocks.reply
      .mockImplementationOnce(async (options: unknown) => ({
        stream: (async function* () {
          foreignTurnStarted = true;
          // What the generated SSE client does with a 409: it reports the
          // error and ends the generator without a frame.
          (options as { onSseError?: (e: unknown) => void }).onSseError?.(
            new Error('SSE failed: 409 Conflict')
          );
          yield* [] as MessageEvent[];
        })(),
      }))
      .mockResolvedValueOnce({ stream: foreign.stream });

    const controller = registry.getController(sid);
    await controller.loadSession();
    await expect(controller.handleSubmit('send this')).resolves.toBe(false);

    const texts = controller.getSnapshot().messages.map((m) => JSON.stringify(m.content));
    expect(texts.some((t) => t.includes('send this'))).toBe(false);
    await vi.waitFor(() => expect(mocks.reply).toHaveBeenCalledTimes(2));
    expect((mocks.reply.mock.calls[1][0] as { body: { turn_id: string } }).body.turn_id).toBe(
      'turn-7'
    );
    expect(controller.getSnapshot().turnError).toBeUndefined();
    await vi.waitFor(() => expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming));

    foreign.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
    foreign.close();
    await vi.waitFor(() => expect(controller.getSnapshot().chatState).toBe(ChatState.Idle));
  });
});

describe('D9 — a silent stream is reconciled with the daemon', () => {
  it('rejoins a turn the daemon still names after the stream goes silent', async () => {
    vi.useFakeTimers();
    try {
      const { controller, driving, turnId } = await drivingTurn('silent-same');
      driving.push({
        type: 'Message',
        message: assistantText('a1', 'partial'),
        token_state: tokenState,
        seq: 0,
        turn_id: turnId,
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      mocks.resumeAgent.mockResolvedValue({
        data: { session: session('x'), active_turn: { turn_id: turnId } },
      });
      const rejoined = createControlledStream();
      mocks.reply.mockResolvedValueOnce({ stream: rejoined.stream });

      await vi.advanceTimersByTimeAsync(STREAM_SILENCE_MS + 4_000);
      await vi.advanceTimersByTimeAsync(2_000); // the first reconcile backoff
      await vi.waitFor(() => expect(mocks.reply).toHaveBeenCalledTimes(2));
      const attach = mocks.reply.mock.calls[1][0] as {
        body: { turn_id: string; from_seq: number };
      };
      expect(attach.body.turn_id).toBe(turnId);
      expect(attach.body.from_seq).toBe(1);
      expect(isRunningState(controller.getSnapshot().chatState)).toBe(true);

      rejoined.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
      rejoined.close();
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    } finally {
      vi.useRealTimers();
    }
  });

  it('ends the spin without an error card when the daemon names no turn', async () => {
    vi.useFakeTimers();
    try {
      const { controller, driving } = await drivingTurn('silent-none');
      driving.push({
        type: 'Message',
        message: assistantText('a1', 'partial'),
        token_state: tokenState,
      } as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      mocks.resumeAgent.mockResolvedValue({ data: { session: session('x') } });

      await vi.advanceTimersByTimeAsync(STREAM_SILENCE_MS + 4_000);
      await vi.waitFor(() => expect(controller.getSnapshot().chatState).toBe(ChatState.Idle));
      expect(controller.getSnapshot().turnError).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('D10 — a steer chip is retired by the echo, whatever happened to the history', () => {
  const seven = Array.from({ length: 7 }, (_, i) =>
    i % 2 === 0 ? userText(`u${i}`, `prompt ${i}`) : assistantText(`a${i}`, `answer ${i}`)
  );

  it('retires the chip after a compaction shrank the transcript', async () => {
    const { controller, driving, submit } = await drivingTurn('steer-compaction', seven);
    mocks.interrupt.mockResolvedValue({ data: { turn_id: 't' }, response: { status: 202 } });
    await controller.steer('use the 2024 cohort');
    expect(controller.getSnapshot().pendingSteer).toBeDefined();

    driving.push({
      type: 'UpdateConversation',
      conversation: [userText('s1', 'summary'), assistantText('s2', 'ok')],
      token_state: tokenState,
    } as MessageEvent);
    driving.push({
      type: 'Message',
      message: userText('echo-1', 'use the 2024 cohort'),
      token_state: tokenState,
    } as MessageEvent);
    await vi.waitFor(() => expect(controller.getSnapshot().pendingSteer).toBeUndefined());

    driving.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
    driving.close();
    await submit;
  });

  it('retires the chip when the rewritten history already contains the echo', async () => {
    const { controller, driving, submit } = await drivingTurn('steer-resync', seven);
    mocks.interrupt.mockResolvedValue({ data: { turn_id: 't' }, response: { status: 202 } });
    await controller.steer('use the 2024 cohort');

    driving.push({
      type: 'UpdateConversation',
      conversation: [userText('s1', 'summary'), userText('echo-2', 'use the 2024 cohort')],
      token_state: tokenState,
    } as MessageEvent);
    await vi.waitFor(() => expect(controller.getSnapshot().pendingSteer).toBeUndefined());

    driving.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
    driving.close();
    await submit;
  });

  it('D4: an echo the daemon marked unanswered does not retire the chip as landed', async () => {
    const { controller, driving, submit } = await drivingTurn('steer-unanswered');
    mocks.interrupt.mockResolvedValue({ data: { turn_id: 't' }, response: { status: 202 } });
    await controller.steer('use the 2024 cohort');

    const unanswered = userText('carried', 'use the 2024 cohort');
    unanswered.metadata = { ...unanswered.metadata, steerOutcome: 'unanswered' };
    driving.push({ type: 'Message', message: unanswered, token_state: tokenState } as MessageEvent);
    driving.push({
      type: 'Message',
      message: assistantText('after', 'still going'),
      token_state: tokenState,
    } as MessageEvent);
    await vi.waitFor(() =>
      expect(JSON.stringify(controller.getSnapshot().messages)).toContain('still going')
    );
    expect(controller.getSnapshot().pendingSteer?.text).toBe('use the 2024 cohort');

    driving.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
    driving.close();
    await submit;
  });
});

describe('D11 — a Stop that learns of a successor turn attaches to it', () => {
  it('opens a socket on the successor and ends the spin at its Finish', async () => {
    const { controller, driving, submit, turnId } = await drivingTurn('stop-successor');
    mocks.cancelTurn.mockRejectedValueOnce({
      mismatch: true,
      expected_turn_id: turnId,
      active_turn_id: 'turn-9',
    });
    const successor = createControlledStream();
    mocks.reply.mockResolvedValueOnce({ stream: successor.stream });

    await expect(controller.stopStreaming()).resolves.toBe(false);
    await vi.waitFor(() => expect(mocks.reply).toHaveBeenCalledTimes(2));
    expect((mocks.reply.mock.calls[1][0] as { body: { turn_id: string } }).body.turn_id).toBe(
      'turn-9'
    );

    successor.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
    successor.close();
    await vi.waitFor(() => expect(controller.getSnapshot().chatState).toBe(ChatState.Idle));
    driving.close();
    await submit;
  });
});

describe('D16 — a retracted tool-call skeleton leaves the screen', () => {
  it('removes the skeletons a steer restart retracted', async () => {
    const { controller, driving, submit } = await drivingTurn('retract');
    driving.push({
      type: 'ToolCallPending',
      id: 'call-1',
      name: 'developer__shell',
    } as MessageEvent);
    await vi.waitFor(() => expect(controller.getSnapshot().pendingToolCalls).toHaveLength(1));
    driving.push({ type: 'ToolCallsRetracted', ids: ['call-1'] } as MessageEvent);
    await vi.waitFor(() => expect(controller.getSnapshot().pendingToolCalls).toHaveLength(0));

    driving.push({ type: 'Finish', reason: 'stop', token_state: tokenState } as MessageEvent);
    driving.close();
    await submit;
  });
});
