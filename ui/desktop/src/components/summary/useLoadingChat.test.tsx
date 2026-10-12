import { act, render, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../../api';
import { ChatState } from '../../types/chatState';
import { summaryRailChat } from '../../hooks/useSummaryRail';
import { awaitingFirstRead, loadingChatLook, useLoadingChat } from './useLoadingChat';

/**
 * Vision V2-R5-6, round 7: a chat's first open after a reload. The listing of
 * every chat is still on its way and the row read waits behind the app's other
 * requests, so for 140 to 430 ms the chat has no row. It was drawn as a new chat
 * ("Ask Biorouter anything…", the new-chat model, no rail) and then jumped 140px
 * when the row landed. Now it paints nothing until it knows, and never the
 * new-chat look for a chat that has messages.
 */
const mocks = vi.hoisted(() => ({
  getSession: vi.fn(),
  rows: null as Session[] | null,
  listeners: new Set<() => void>(),
}));
vi.mock('../../api', () => ({ getSession: mocks.getSession }));
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'synthetic-proof' }),
}));
vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => mocks.rows,
  subscribeSessionList: (listener: () => void) => {
    mocks.listeners.add(listener);
    return () => mocks.listeners.delete(listener);
  },
}));

const gwas = {
  id: 'gwas',
  name: 'GWAS on Type 2 Diabetes',
  message_count: 41,
  provider_name: 'aws_bedrock',
  model_config: { model_name: 'claude-sonnet-4-6' },
  total_tokens: 90_000,
} as unknown as Session;
const emptyWorkflowChat = { ...gwas, id: 'flow', message_count: 0 } as unknown as Session;

beforeEach(() => {
  mocks.rows = null;
  mocks.listeners.clear();
  mocks.getSession.mockReset().mockReturnValue(new Promise(() => {}));
});

describe('loadingChatLook', () => {
  it('is the chat when its row has messages, new when empty, pending when there is no row', () => {
    expect(loadingChatLook({ row: gwas, startingWithCargo: false })).toBe('chat');
    expect(loadingChatLook({ row: gwas, startingWithCargo: true })).toBe('chat');
    expect(loadingChatLook({ row: emptyWorkflowChat, startingWithCargo: false })).toBe('new');
    expect(loadingChatLook({ row: undefined, startingWithCargo: true })).toBe('new');
    expect(loadingChatLook({ row: undefined, startingWithCargo: false })).toBe('pending');
  });
});

describe('awaitingFirstRead', () => {
  const base = {
    sessionId: 'gwas',
    hasSession: false,
    hasLoadError: false,
    chatState: ChatState.Idle,
    messageCount: 0,
    creatingChat: false,
  };
  it('is the commit before a named chat’s store starts its load', () => {
    expect(awaitingFirstRead(base)).toBe(true);
  });
  it('is nothing for a new chat, a loaded chat, a failed load or a chat being created', () => {
    expect(awaitingFirstRead({ ...base, sessionId: '' })).toBe(false);
    expect(awaitingFirstRead({ ...base, hasSession: true })).toBe(false);
    expect(awaitingFirstRead({ ...base, hasLoadError: true })).toBe(false);
    expect(awaitingFirstRead({ ...base, creatingChat: true })).toBe(false);
    expect(awaitingFirstRead({ ...base, messageCount: 2 })).toBe(false);
    // The load itself is `LoadingConversation`, which the hook counts separately.
    expect(awaitingFirstRead({ ...base, chatState: ChatState.LoadingConversation })).toBe(false);
  });
});

interface Store {
  chatState: ChatState;
  session?: Session;
  messageCount: number;
}

/**
 * BaseChat's wiring of these pieces, reduced to what the first open shows:
 * whether the column paints, the composer's placeholder, the chip's model and
 * the rail. BaseChat cannot be mounted in jsdom; `BaseChat.summaryRail.test.ts`
 * pins that it joins them this way.
 */
function OpenedChat({
  sessionId,
  store,
  initialMessage,
  log,
}: {
  sessionId: string;
  store: Store;
  initialMessage?: string;
  log: string[];
}) {
  const loadingChat = useLoadingChat({
    sessionId,
    session: store.session,
    hasLoadError: false,
    chatState: store.chatState,
    messageCount: store.messageCount,
    creatingChat: false,
    startingWithCargo: !!initialMessage,
  });
  const pending = loadingChat.look === 'pending';
  const rail = summaryRailChat({
    clean: false,
    messageCount: store.messageCount,
    loading: loadingChat.loading,
    running: false,
    creatingChat: false,
    knownToHaveMessages: loadingChat.knownMessages > 0,
  });
  const messagesLength = store.messageCount || loadingChat.knownMessages;
  const placeholder = messagesLength > 0 ? 'Ask a follow-up' : 'Ask Biorouter anything…';
  // usePinnedModel(session ?? loadingChatRow): the row's model, else the default.
  const binding = store.session ?? loadingChat.row;
  const chip = binding?.model_config?.model_name ?? 'gpt-5.5 (new-chat default)';
  const shown = pending
    ? 'painted nothing'
    : `${placeholder} | ${chip} | ${rail.active ? `rail ${rail.chatPhase}` : 'no rail'}`;
  log.push(shown);
  return <div data-testid="column">{shown}</div>;
}

describe('a chat’s first open after a reload (V2-R5-6, round 7)', () => {
  it('cold: paints nothing until the row answers, then the chat at rest, never a new chat', async () => {
    let answer!: (row: Session) => void;
    mocks.getSession.mockReturnValue(
      new Promise((resolve) => (answer = (row) => resolve({ data: row })))
    );
    const log: string[] = [];
    // The store for a chat this window has not opened starts at Idle, empty.
    const view = render(
      <OpenedChat
        sessionId="gwas"
        store={{ chatState: ChatState.Idle, messageCount: 0 }}
        log={log}
      />
    );
    expect(log[0]).toBe('painted nothing');
    // Its load starts.
    view.rerender(
      <OpenedChat
        sessionId="gwas"
        store={{ chatState: ChatState.LoadingConversation, messageCount: 0 }}
        log={log}
      />
    );
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalled());
    expect(log[log.length - 1]).toBe('painted nothing');

    await act(async () => answer(gwas));
    expect(view.getByTestId('column')).toHaveTextContent(
      'Ask a follow-up | claude-sonnet-4-6 | rail loading'
    );
    // The transcript lands: the same chat, the rail still at rest.
    view.rerender(
      <OpenedChat
        sessionId="gwas"
        store={{ chatState: ChatState.Idle, session: gwas, messageCount: 41 }}
        log={log}
      />
    );
    expect(view.getByTestId('column')).toHaveTextContent(
      'Ask a follow-up | claude-sonnet-4-6 | rail idle'
    );
    const distinct = log.filter((entry, index) => entry !== log[index - 1]);
    expect(distinct).toEqual([
      'painted nothing',
      'Ask a follow-up | claude-sonnet-4-6 | rail loading',
      'Ask a follow-up | claude-sonnet-4-6 | rail idle',
    ]);
    expect(log.some((entry) => entry.includes('Ask Biorouter anything'))).toBe(false);
    expect(log.some((entry) => entry.includes('new-chat default'))).toBe(false);
  });

  it('warm listing: the first commit is already the chat, with its rail', () => {
    mocks.rows = [gwas];
    const log: string[] = [];
    render(
      <OpenedChat
        sessionId="gwas"
        store={{ chatState: ChatState.Idle, messageCount: 0 }}
        log={log}
      />
    );
    expect(log[0]).toBe('Ask a follow-up | claude-sonnet-4-6 | rail loading');
  });

  it('a chat created from Home with its message on the way keeps the new-chat composer', () => {
    const log: string[] = [];
    render(
      <OpenedChat
        sessionId="fresh"
        store={{ chatState: ChatState.LoadingConversation, messageCount: 0 }}
        initialMessage="Plan a GWAS"
        log={log}
      />
    );
    expect(log[0]).toBe('Ask Biorouter anything… | gpt-5.5 (new-chat default) | no rail');
  });

  it('a chat created from Workflows waits for its row and never draws the rail', async () => {
    mocks.getSession.mockResolvedValue({ data: emptyWorkflowChat });
    const log: string[] = [];
    const view = render(
      <OpenedChat
        sessionId="flow"
        store={{ chatState: ChatState.LoadingConversation, messageCount: 0 }}
        log={log}
      />
    );
    expect(log[0]).toBe('painted nothing');
    await waitFor(() => expect(view.getByTestId('column')).toHaveTextContent('no rail'));
    expect(log.some((entry) => entry.includes('rail loading'))).toBe(false);
  });

  it('nothing is pending once the chat has its session, or for a new chat with no id', async () => {
    const loaded = renderHook(() =>
      useLoadingChat({
        sessionId: 'gwas',
        session: gwas,
        hasLoadError: false,
        chatState: ChatState.Idle,
        messageCount: 41,
        creatingChat: false,
        startingWithCargo: false,
      })
    );
    expect(loaded.result.current).toMatchObject({ loading: false, look: null, knownMessages: 0 });

    const blank = renderHook(() =>
      useLoadingChat({
        sessionId: '',
        session: undefined,
        hasLoadError: false,
        chatState: ChatState.LoadingConversation,
        messageCount: 0,
        creatingChat: true,
        startingWithCargo: false,
      })
    );
    expect(blank.result.current.look).toBeNull();

    // A chat being created and then bound to its id is a new chat with its
    // first message on the way.
    const bound = renderHook(() =>
      useLoadingChat({
        sessionId: 'just-made',
        session: undefined,
        hasLoadError: false,
        chatState: ChatState.LoadingConversation,
        messageCount: 0,
        creatingChat: true,
        startingWithCargo: false,
      })
    );
    expect(bound.result.current.look).toBe('new');
    // Only the loading chat with an id asks for its row.
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(1));
    expect(mocks.getSession).toHaveBeenCalledWith(
      expect.objectContaining({ path: { session_id: 'just-made' } })
    );
  });

  it('a failed load is not loading: its error shows instead', () => {
    const failed = renderHook(() =>
      useLoadingChat({
        sessionId: 'gwas',
        session: undefined,
        hasLoadError: true,
        chatState: ChatState.Idle,
        messageCount: 0,
        creatingChat: false,
        startingWithCargo: false,
      })
    );
    expect(failed.result.current).toMatchObject({ loading: false, look: null });
  });
});
