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

import { ChatStreamRegistry, isRunningState } from './chatStreamStore';
import { ChatState } from '../types/chatState';

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

export { createControlledStream, session, userText, assistantText, tokenState, isRunningState, ChatState };
