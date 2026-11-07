import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatStreamRegistry } from './chatStreamStore';
import type { Message, MessageEvent, Session, TokenState } from '../api';
import { reply, resumeAgent } from '../api';
import {
  resetStreamedCrewStepsForTests,
  wasStreamedCrewStep,
} from '../components/crew/chat/streamedCrewSteps';

vi.mock('../api', async () => ({
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

/** A Crew step card as the daemon publishes one: user-only, and never stored. */
function stepCard(messageId: string, cardId: string): Message {
  return {
    id: messageId,
    role: 'assistant',
    created: 2,
    metadata: { userVisible: true, agentVisible: false },
    content: [
      {
        type: 'actionRequired',
        data: {
          actionType: 'crewStep',
          id: cardId,
          step: {
            kind: 'signIn',
            target: { kind: 'setup', jobId: 'job-1' },
            server: 'hpc.example.edu',
            login: 'you@hpc.example.edu',
          },
        },
      },
    ],
  } as Message;
}

function session(id: string, conversation: Message[]): Session {
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

beforeEach(() => {
  vi.useFakeTimers();
  resetStreamedCrewStepsForTests();
  vi.mocked(resumeAgent).mockReset();
  vi.mocked(reply).mockReset();
  Object.assign(window, {
    electron: {
      openExternal: vi.fn(async () => undefined),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

describe('the chat stream and Crew step cards', () => {
  it('notes a card it streamed, and never one a stored conversation carried', async () => {
    const sid = 'crew-step-streamed-vs-stored';
    // A card in the stored rows (an imported session, a forged file): the daemon never stores one.
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session(sid, [stepCard('stored-row', 'card-from-file')]) },
    } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield {
          type: 'Message',
          message: stepCard('live-row', 'card-from-daemon'),
          token_state: tokenState,
        } as MessageEvent;
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    const registry = new ChatStreamRegistry();
    await registry.getController(sid).handleSubmit('set up Crew on hpc');

    expect(wasStreamedCrewStep(sid, 'card-from-daemon')).toBe(true);
    expect(wasStreamedCrewStep(sid, 'card-from-file')).toBe(false);
    // Kept per chat: the same card ID in another chat is not this one.
    expect(wasStreamedCrewStep('another-chat', 'card-from-daemon')).toBe(false);
  });
});
