/**
 * The pending-card contract, exercised with a MIRRORED tool pair.
 *
 * `pendingToolCalls` is advisory display state held deliberately OUT of
 * `messages` (`chatStreamStore.tsx`, §6.1b). A skeleton is drawn the moment a
 * tool's name is known and must be dropped the instant the authoritative
 * `toolRequest` with the SAME id lands — otherwise a mirrored card and its
 * skeleton would both be on screen, which reads as the same tool call happening
 * twice.
 *
 * A coding-agent provider's mirrored pair carries the marker in
 * `metadata.biorouterProviderExecuted`, and the store must not care: the
 * replacement keys on the id alone.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatStreamRegistry } from './chatStreamStore';
import type { Message, MessageEvent, Session, TokenState } from '../api';
import { cancelTurn, getSession, reply, resumeAgent } from '../api';

vi.mock('../api', async () => ({
  cancelTurn: vi.fn(async () => ({ data: { cancelled: true } })),
  editMessage: vi.fn(),
  getSession: vi.fn(async () => ({ data: null })),
  interrupt: vi.fn(),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  reply: vi.fn(),
  resumeAgent: vi.fn(),
  updateFromSession: vi.fn(async () => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../utils/userAction')>()),
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

/** The assistant half of a mirrored pair, exactly as the provider emits it. */
function mirroredRequestMessage(callId: string): Message {
  return {
    id: 'mirror-request',
    role: 'assistant',
    created: 2,
    content: [
      {
        type: 'toolRequest',
        id: callId,
        toolCall: {
          status: 'success',
          value: { name: 'developer__shell', arguments: { command: 'ls' } },
        },
        metadata: { biorouterProviderExecuted: 'bridged' },
      },
    ],
    metadata: { userVisible: true, agentVisible: true },
  } as Message;
}

/**
 * A stream the test drives one event at a time, so the assertion that the
 * skeleton EXISTS before the request lands is a real observation rather than a
 * vacuous one. Driving it matters here: `finishCurrentStream` clears every
 * skeleton at the end of the turn, so a check taken only after the turn passes
 * whether or not the id-keyed replacement works.
 */
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

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
}

beforeEach(() => {
  vi.mocked(resumeAgent).mockReset();
  vi.mocked(reply).mockReset();
  vi.mocked(getSession).mockReset();
  vi.mocked(cancelTurn).mockReset();
  vi.mocked(getSession).mockResolvedValue({ data: null } as never);
  Object.assign(window, {
    electron: {
      openExternal: vi.fn(async () => undefined),
      readTempImageAsBase64: vi.fn(async () => ({ data: 'B64-image', mimeType: 'image/png' })),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

describe('a mirrored tool request retires its pending skeleton', () => {
  it('replaces the skeleton with the same id, leaving nothing stranded', async () => {
    const sid = 'mirror-pending-replaced';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid) } } as never);
    const controlled = createControlledStream();
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController(sid);
    await controller.loadSession();
    await flush();

    const inFlight = controller.handleSubmit('run ls');
    for (let i = 0; i < 12 && vi.mocked(reply).mock.calls.length === 0; i += 1) {
      await flush();
    }

    controlled.push({
      type: 'ToolCallPending',
      id: 'toolu_1',
      name: 'developer__shell',
      partial_args: '{"comm',
    } as MessageEvent);
    for (let i = 0; i < 12 && controller.getSnapshot().pendingToolCalls.length === 0; i += 1) {
      await flush();
    }
    // The skeleton really is on screen at this point — the precondition the
    // replacement assertion below is worth making.
    expect(controller.getSnapshot().pendingToolCalls).toEqual([
      { id: 'toolu_1', name: 'developer__shell', partialArgs: '{"comm' },
    ]);

    controlled.push({
      type: 'Message',
      message: mirroredRequestMessage('toolu_1'),
      token_state: tokenState,
    } as MessageEvent);
    for (let i = 0; i < 12 && controller.getSnapshot().pendingToolCalls.length > 0; i += 1) {
      await flush();
    }

    const snapshot = controller.getSnapshot();
    expect(snapshot.pendingToolCalls).toEqual([]);
    // The authoritative request is in `messages`, with its marker intact — the
    // skeleton never was and must never be.
    const request = snapshot.messages
      .flatMap((m) => m.content)
      .find((c) => c.type === 'toolRequest');
    expect(request).toMatchObject({
      id: 'toolu_1',
      metadata: { biorouterProviderExecuted: 'bridged' },
    });

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await inFlight;
  });

  it('clears a skeleton whose mirrored request never arrives when the turn ends', async () => {
    const sid = 'mirror-pending-abandoned';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid) } } as never);
    const controlled = createControlledStream();
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController(sid);
    await controller.loadSession();
    await flush();

    const inFlight = controller.handleSubmit('run ls');
    for (let i = 0; i < 12 && vi.mocked(reply).mock.calls.length === 0; i += 1) {
      await flush();
    }

    controlled.push({
      type: 'ToolCallPending',
      id: 'toolu_orphan',
      name: 'developer__shell',
    } as MessageEvent);
    for (let i = 0; i < 12 && controller.getSnapshot().pendingToolCalls.length === 0; i += 1) {
      await flush();
    }
    expect(controller.getSnapshot().pendingToolCalls).toHaveLength(1);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await inFlight;

    expect(controller.getSnapshot().pendingToolCalls).toEqual([]);
  });
});
