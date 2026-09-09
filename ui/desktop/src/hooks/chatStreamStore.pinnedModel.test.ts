import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatStreamRegistry } from './chatStreamStore';
import type { MessageEvent, Session, TokenState } from '../api';
import { reply, resumeAgent } from '../api';

/**
 * Issue #56 Gate B / F2 — the store's half of "which model actually served this
 * turn".
 *
 * The daemon sends `PrivacyProviderPinned` when a turn had to fall back to the
 * provider the session row names. Before this frame existed, a private chat
 * answered from Versa while the composer's chip said `claude-opus-5` and its
 * gauge measured against Claude's 1M window.
 */
vi.mock('../api', () => ({
  cancelTurn: vi.fn(),
  editMessage: vi.fn(),
  getSession: vi.fn(async () => ({ data: null })),
  interrupt: vi.fn(),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  reply: vi.fn(),
  resumeAgent: vi.fn(),
  updateFromSession: vi.fn(async () => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../utils/continuationLease', () => ({
  abandonContinuationLease: vi.fn(async () => undefined),
  getContinuationOwnerId: vi.fn(() => 'test-window-owner'),
  recoverContinuationGroup: vi.fn(),
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

const PIN: MessageEvent = {
  type: 'PrivacyProviderPinned',
  provider: 'versa_azure',
  model: 'gpt-5.5-2026-04-24',
} as MessageEvent;

function streamOf(...events: MessageEvent[]) {
  return {
    stream: (async function* () {
      for (const event of events) yield event;
    })(),
  } as never;
}

beforeEach(() => {
  vi.mocked(reply).mockReset();
  vi.mocked(resumeAgent).mockReset();
  Object.assign(window, {
    electron: {
      openExternal: vi.fn(async () => undefined),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

describe('PrivacyProviderPinned', () => {
  async function runTurn(sid: string, ...events: MessageEvent[]) {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid) } } as never);
    vi.mocked(reply).mockImplementation(async () => streamOf(...events));
    const controller = registry.getController(sid);
    await controller.handleSubmit('hi');
    return controller;
  }

  it('records the binding the turn actually ran on', async () => {
    const controller = await runTurn('pin-records', PIN, {
      type: 'Finish',
      reason: 'done',
      token_state: tokenState,
    } as MessageEvent);

    expect(controller.getSnapshot().pinnedModel).toEqual({
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
    });
  });

  /**
   * ⚠ It must OUTLIVE the turn that reported it. The pin is a property of the
   * chat, not of one turn: clearing it on `Finish` would put the wrong model
   * back on the chip and the wrong window back on the gauge at the exact moment
   * the answer arrived — which is the defect, one second later.
   */
  it('survives the end of the turn that reported it', async () => {
    const controller = await runTurn('pin-survives', PIN, {
      type: 'Finish',
      reason: 'done',
      token_state: tokenState,
    } as MessageEvent);

    expect(controller.getSnapshot().chatState).not.toBe('streaming');
    expect(controller.getSnapshot().pinnedModel?.provider).toBe('versa_azure');
  });

  it('leaves an unpinned chat with nothing recorded', async () => {
    const controller = await runTurn('pin-absent', {
      type: 'Finish',
      reason: 'done',
      token_state: tokenState,
    } as MessageEvent);

    expect(controller.getSnapshot().pinnedModel).toBeUndefined();
  });

  /**
   * The frame arrives on every repaired turn. A fresh object each time would
   * re-render the composer — chip and gauge included — once per turn for no
   * change at all, so the snapshot identity has to be stable.
   */
  it('does not churn the snapshot when the same binding is reported again', async () => {
    const controller = await runTurn('pin-idempotent', PIN, PIN, PIN, {
      type: 'Finish',
      reason: 'done',
      token_state: tokenState,
    } as MessageEvent);

    const first = controller.getSnapshot().pinnedModel;
    await controller.handleSubmit('again');
    expect(controller.getSnapshot().pinnedModel).toBe(first);
  });
});
