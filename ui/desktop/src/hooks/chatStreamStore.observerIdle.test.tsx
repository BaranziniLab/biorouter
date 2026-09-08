/**
 * BR-71 §3c — **a message appended to an IDLE conversation must not make its
 * tab claim a running turn.**
 *
 * Cross-chat injection publishes the stored row onto the target session's bus,
 * and the target's tab renders it live through the observer feed. For
 * `workspace_send_prompt mode:"note"` that row arrives with **no turn in
 * flight**: a note is explicitly "leave context, start nothing".
 *
 * `applyMessageEvent` derives `ChatState.Streaming` from any message, because
 * on the driver path a message is by definition a turn producing output. On the
 * observer path with no active turn that is false, and it fails in the worst
 * direction: nothing is running, so nothing will ever publish the terminal that
 * would retire the state. Measured in the running app before the fix — the
 * daemon's `/active_work` empty, the target tab showing "Thinking · 29s" with a
 * live stop button, indefinitely.
 *
 * ⚠ This is the half a jsdom test *can* hold: what the store does with a frame.
 * That the frame arrives at all — publish → SSE → observer — is a daemon fact,
 * held by `workspace_extension`'s bus tests and by the GUI drive-through.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message, MessageEvent, TokenState } from '../api';

const mocks = vi.hoisted(() => ({
  reply: vi.fn(),
  observeSessionEvents: vi.fn(),
  resumeAgent: vi.fn(async () => ({ data: null })),
  cancelTurn: vi.fn(async () => ({ data: { cancelled: true } })),
  getSession: vi.fn(async () => ({ data: null })),
  interrupt: vi.fn(),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  updateFromSession: vi.fn(async () => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../api', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return { ...actual, ...mocks };
});

const { ChatStreamRegistry } = await import('./chatStreamStore');
const { ChatState } = await import('../types/chatState');

const tokenState = {
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
  accumulatedInputTokens: 0,
  accumulatedOutputTokens: 0,
  accumulatedTotalTokens: 0,
} as unknown as TokenState;

function userMessage(id: string, text: string): Message {
  return {
    id,
    role: 'user',
    created: Math.floor(Date.now() / 1000),
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  } as unknown as Message;
}

function messageFrame(id: string, text: string): MessageEvent {
  return {
    type: 'Message',
    message: userMessage(id, text),
    token_state: tokenState,
  } as MessageEvent;
}

const turnStateIdle = { type: 'TurnState', active_turn_id: null } as unknown as MessageEvent;
const turnStateRunning = {
  type: 'TurnState',
  active_turn_id: 'turn-live',
} as unknown as MessageEvent;

async function* streamOf(...frames: MessageEvent[]) {
  for (const frame of frames) yield frame;
}

let sessionSeq = 0;

beforeEach(() => {
  mocks.observeSessionEvents.mockReset();
  Object.assign(window, {
    electron: {
      getUserActionKey: vi.fn(async () => 'proof-of-user'),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

/** Drive one observer connection to completion and stop the loop. */
async function observeOnce(sid: string, frames: MessageEvent[]) {
  mocks.observeSessionEvents.mockResolvedValue({ stream: streamOf(...frames) });
  const controller = new ChatStreamRegistry().getController(sid);
  const loop = controller.observeSession();
  await vi.waitFor(() => expect(mocks.observeSessionEvents).toHaveBeenCalled());
  await new Promise((r) => setTimeout(r, 20));
  const state = controller.getSnapshot().chatState;
  const messages = controller.getSnapshot().messages;
  controller.stopObserving();
  await loop;
  return { state, messages };
}

describe('an injected message on an observer feed', () => {
  it('leaves an idle conversation idle', async () => {
    const { state, messages } = await observeOnce(`obs-idle-${++sessionSeq}`, [
      turnStateIdle,
      messageFrame('m-note', 'INJECTED-NOTE-MARKER'),
    ]);

    // The row is rendered — that is the whole point of publishing it.
    expect(messages.some((m) => JSON.stringify(m).includes('INJECTED-NOTE-MARKER'))).toBe(true);
    // …and the tab does not claim a turn that does not exist. Nothing would
    // ever retire it if it did.
    expect(state).toBe(ChatState.Idle);
  });

  it('still reflects a real running turn as running', async () => {
    // The control, and the reason the guard keys on `activeTurnId` rather than
    // on "this is an observer". A genuinely running turn announces itself
    // first — the SSE handler sends `TurnState` right after its snapshot — so
    // its messages must still raise the running state, or an observed subagent
    // would render as idle for its whole run.
    const { state, messages } = await observeOnce(`obs-live-${++sessionSeq}`, [
      turnStateRunning,
      messageFrame('m-live', 'STREAMED-OUTPUT-MARKER'),
    ]);

    expect(messages.some((m) => JSON.stringify(m).includes('STREAMED-OUTPUT-MARKER'))).toBe(true);
    expect(state).not.toBe(ChatState.Idle);
  });
});
