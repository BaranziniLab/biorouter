import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatStreamRegistry } from './chatStreamStore';
import { ChatState } from '../types/chatState';
import type { Message, MessageEvent, Session, TokenState } from '../api';
import { reply, resumeAgent } from '../api';
import {
  cardEndingOf,
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

/** A stream the test feeds one frame at a time. */
function controlledStream() {
  const events: MessageEvent[] = [];
  let wake: (() => void) | null = null;
  let closed = false;
  async function* stream() {
    while (!closed || events.length > 0) {
      if (events.length === 0) {
        await new Promise<void>((resolve) => {
          wake = resolve;
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
      wake?.();
      wake = null;
    },
    close() {
      closed = true;
      wake?.();
      wake = null;
    },
  };
}

async function settle() {
  await vi.advanceTimersByTimeAsync(0);
  await Promise.resolve();
  await Promise.resolve();
}

/** The daemon's WI-17 frame: a card's ID and how it ended, nothing else. */
function ended(id: string, ending: string): MessageEvent {
  return { type: 'ActionRequiredEnded', id, ending } as unknown as MessageEvent;
}

function changedKeyNotice(messageId: string, cardId: string): Message {
  const message = stepCard(messageId, cardId);
  (message.content[0] as { data: { step: unknown } }).data.step = {
    kind: 'hostKey',
    host: 'hpc.example.edu',
    port: 22,
    login: 'you',
    offered: null,
    known: null,
    changed: true,
  };
  return message;
}

describe('the life of a Crew step card on the chat stream (WI-17)', () => {
  it('waits for the person while a step waits, and stops the moment the daemon ends it', async () => {
    const sid = 'crew-step-waiting';
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid, []) } } as never);
    const feed = controlledStream();
    vi.mocked(reply).mockResolvedValue({ stream: feed.stream } as never);

    const registry = new ChatStreamRegistry();
    const controller = registry.getController(sid);
    const submitted = controller.handleSubmit('sign me in on hpc');
    await settle();

    feed.push({ type: 'Message', message: stepCard('row-1', 'card-1'), token_state: tokenState });
    await settle();
    // (f): a chat parked on a Crew step is waiting for the person, not streaming.
    expect(controller.getSnapshot().chatState).toBe(ChatState.WaitingForUserInput);
    expect(cardEndingOf(sid, 'card-1')).toBeUndefined();

    // Another chat's card ending, and an ID this chat never drew, leave it waiting.
    feed.push(ended('card-elsewhere', 'dismissed'));
    await settle();
    expect(controller.getSnapshot().chatState).toBe(ChatState.WaitingForUserInput);

    feed.push(ended('card-1', 'withdrawn'));
    await settle();
    expect(cardEndingOf(sid, 'card-1')).toBe('withdrawn');
    expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

    feed.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    feed.close();
    await submitted;
  });

  it('notes an end that came before its card, and an unknown ending as withdrawn', async () => {
    const sid = 'crew-step-early-end';
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid, []) } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield ended('card-early', 'a word from a newer daemon');
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);
    await new ChatStreamRegistry().getController(sid).handleSubmit('go');
    expect(cardEndingOf(sid, 'card-early')).toBe('withdrawn');
    expect(cardEndingOf('another-chat', 'card-early')).toBeUndefined();
  });

  it('does not wait on a changed host key, which is a notice', async () => {
    const sid = 'crew-step-changed-key';
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid, []) } } as never);
    const feed = controlledStream();
    vi.mocked(reply).mockResolvedValue({ stream: feed.stream } as never);
    const controller = new ChatStreamRegistry().getController(sid);
    const submitted = controller.handleSubmit('connect to hpc');
    await settle();
    feed.push({
      type: 'Message',
      message: changedKeyNotice('row-2', 'card-2'),
      token_state: tokenState,
    });
    await settle();
    expect(controller.getSnapshot().chatState).not.toBe(ChatState.WaitingForUserInput);
    feed.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    feed.close();
    await submitted;
  });

  it('draws a card handed over again once, and again after a resync replaced the view', async () => {
    const sid = 'crew-step-handed-again';
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid, []) } } as never);
    const text: Message = {
      id: 'prose',
      role: 'assistant',
      created: 3,
      metadata: { userVisible: true, agentVisible: true },
      content: [{ type: 'text', text: 'Waiting for you to sign in.' }],
    } as Message;
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Message', message: stepCard('row-a', 'card-x'), token_state: tokenState };
        yield { type: 'Message', message: text, token_state: tokenState };
        // The daemon hands the waiting card over again (after a lag resync of its own log).
        yield { type: 'Message', message: stepCard('row-b', 'card-x'), token_state: tokenState };
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);
    const controller = new ChatStreamRegistry().getController(sid);
    await controller.handleSubmit('go');
    const cards = controller
      .getSnapshot()
      .messages.filter((message) => message.content.some((c) => c.type === 'actionRequired'));
    expect(cards.map((message) => message.id)).toEqual(['row-a']);
  });

  it('asks the daemon to say when each card ends', async () => {
    const sid = 'crew-step-header';
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid, []) } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);
    await new ChatStreamRegistry().getController(sid).handleSubmit('go');
    const headers = vi.mocked(reply).mock.calls[0][0]?.headers as Record<string, string>;
    expect(headers['X-Card-Endings']).toBe('1');
    // The person's proof still rides with it.
    expect(headers['X-User-Action']).toBe('test-key');
  });
});
