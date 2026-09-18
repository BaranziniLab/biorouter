import { act, fireEvent, render, screen } from '@testing-library/react';
import { BottomMenuReasoningEffort } from '../components/bottom_menu/BottomMenuReasoningEffort';
import {
  resetReasoningEffortForTests,
  sessionReasoningScope,
  setReasoningEffort,
} from '../store/reasoningEffort';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatStreamRegistry } from './chatStreamStore';
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
  SID = `reasoning-s${++sessionSeq}`;
  localStorage.clear();
  sessionStorage.clear();
  resetReasoningEffortForTests();
  vi.mocked(reply).mockReset();
  vi.mocked(resumeAgent).mockReset();
  Object.assign(window, { electron: { showNotification: vi.fn(), logInfo: vi.fn() } });
});

describe('conversation-scoped reasoning in actual reply requests', () => {
  async function ready(id: string, registry: ChatStreamRegistry) {
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(id) } } as never);
    const controller = registry.getController(id);
    await controller.loadSession();
    return controller;
  }

  function select(current: string, next: string) {
    fireEvent.click(screen.getByLabelText(`Reasoning effort: ${current}`));
    fireEvent.click(screen.getByRole('menuitemradio', { name: new RegExp(next) }));
  }

  async function submit(controller: ReturnType<ChatStreamRegistry['getController']>) {
    servingOneTurn();
    await act(async () => {
      await controller.handleSubmit('Reply ready');
    });
    const calls = vi.mocked(reply).mock.calls;
    return calls[calls.length - 1][0]!.body;
  }

  it('keeps A Quick after B Deep, including the next reply after switching back and reloading', async () => {
    const registry = new ChatStreamRegistry();
    const aId = `${SID}-a`;
    const bId = `${SID}-b`;
    const a = await ready(aId, registry);
    const b = await ready(bId, registry);
    const view = render(<BottomMenuReasoningEffort scope={sessionReasoningScope(aId)} />);
    select('Normal', 'Quick');
    expect(await submit(a)).toMatchObject({ session_id: aId, reasoning_effort: 'quick' });

    view.rerender(<BottomMenuReasoningEffort scope={sessionReasoningScope(bId)} />);
    select('Normal', 'Deep');
    expect(await submit(b)).toMatchObject({ session_id: bId, reasoning_effort: 'deep' });
    view.rerender(<BottomMenuReasoningEffort scope={sessionReasoningScope(aId)} />);
    expect(screen.getByLabelText('Reasoning effort: Quick')).toBeInTheDocument();
    expect(await submit(a)).toMatchObject({ session_id: aId, reasoning_effort: 'quick' });

    view.unmount();
    resetReasoningEffortForTests();
    render(<BottomMenuReasoningEffort scope={sessionReasoningScope(aId)} />);
    expect(screen.getByLabelText('Reasoning effort: Quick')).toBeInTheDocument();
    const resumed = await ready(aId, new ChatStreamRegistry());
    expect(await submit(resumed)).toMatchObject({ session_id: aId, reasoning_effort: 'quick' });
    select('Quick', 'Normal');
    expect((await submit(resumed))?.reasoning_effort).toBeUndefined();
    expect(await submit(b)).toMatchObject({ session_id: bId, reasoning_effort: 'deep' });
  });
  it('keeps the submitted choice while the transcript and agent are still loading', async () => {
    const controller = new ChatStreamRegistry().getController(SID);
    let finishResume!: (value: unknown) => void;
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(SID) } } as never);
    vi.mocked(resumeAgent).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishResume = resolve;
        }) as never
    );
    setReasoningEffort(sessionReasoningScope(SID), 'quick');
    servingOneTurn();
    const sending = controller.handleSubmit('Submitted before loading completed');
    setReasoningEffort(sessionReasoningScope(SID), 'deep');
    await act(async () => {
      await Promise.resolve();
      finishResume({ data: { session: session(SID) } });
      await sending;
    });
    expect(vi.mocked(reply).mock.calls[0][0]?.body).toMatchObject({ reasoning_effort: 'quick' });
  });
});
