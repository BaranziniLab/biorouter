import { readFileSync } from 'node:fs';
import path from 'node:path';
import { useState } from 'react';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { KnowledgeProvider, knowledgeScopeFor, useKnowledge } from './KnowledgeContext';
import { isFocusedChatPane, useMirrorFocusedChat } from '../BaseChat';
import type { ChatType } from '../../types/chat';

/**
 * rq2-107 (K-23). The window has ONE `KnowledgeProvider`, mounted in `App.tsx` and
 * scoped by the app-level chat, and every composer's Knowledge list writes through
 * it. That chat used to move only when a chat's NAME changed, so a focused draft
 * "New chat" tab (no session, no name) left the scope on the chat focused before
 * it: the draft's Hide all wrote that other chat's `hidden-kb-sessions` file, and
 * its agent then saw Soul only. Home had the same hole, since nothing moved the
 * scope when the person left a chat for Home.
 *
 * The rule now: the focused pane mirrors itself into the app-level chat whenever
 * it is the focused one, a draft as `''`, and a background pane never does; Home
 * is machine scope. This harness is that wiring with the real pieces: the real
 * provider, BaseChat's own mirror hook and focus predicate, App's own scope rule.
 * Only the daemon is faked.
 */

const mocks = vi.hoisted(() => ({
  listBases: vi.fn(),
  getActive: vi.fn(),
  setActive: vi.fn(),
}));

vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  listBases: mocks.listBases,
  getActive: mocks.getActive,
  setActive: mocks.setActive,
}));

vi.mock('../../toasts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../toasts')>()),
  toastError: vi.fn(),
}));

const NAMED = 'named-chat-20261116_1';

const base = (id: string) => ({
  id,
  name: id,
  color: '#000000',
  created_at: '',
  schema_version: 1,
});

interface Pane {
  tabId: string;
  sessionId: string;
  sessionName?: string;
}

/** One chat pane: what BaseChat does with the app-level chat, and nothing else. */
function ChatPane({
  pane,
  activeTabId,
  setChat,
}: {
  pane: Pane;
  activeTabId: string | undefined;
  setChat: (chat: ChatType) => void;
}) {
  useMirrorFocusedChat({
    focused: isFocusedChatPane(
      { activeTab: activeTabId ? { tabId: activeTabId } : undefined },
      pane.tabId,
      pane.sessionId
    ),
    sessionId: pane.sessionId,
    sessionName: pane.sessionName,
    messages: [],
    workflow: null,
    setChat,
  });
  return null;
}

/** What a composer's Knowledge list reads and the Hide all it presses. */
function KnowledgeList() {
  const { visibleBases, selectionSettled, hideAllKnowledgeBases } = useKnowledge();
  return (
    <div>
      <span data-testid="visible">{visibleBases.map((b) => b.id).join(',') || 'none'}</span>
      <span data-testid="settled">{selectionSettled ? 'settled' : 'waiting'}</span>
      <button type="button" onClick={hideAllKnowledgeBases}>
        Hide all
      </button>
    </div>
  );
}

/** App's shape: one chat in state, one provider scoped by it, the panes on the route. */
function Window({
  pathname,
  panes,
  activeTabId,
}: {
  pathname: string;
  panes: Pane[];
  activeTabId?: string;
}) {
  const [chat, setChat] = useState<ChatType>({
    sessionId: '',
    name: 'New Chat',
    messages: [],
    workflow: null,
  });
  return (
    <KnowledgeProvider sessionId={knowledgeScopeFor(pathname, chat.sessionId)}>
      {panes.map((pane) => (
        <ChatPane key={pane.tabId} pane={pane} activeTabId={activeTabId} setChat={setChat} />
      ))}
      <span data-testid="app-chat">{chat.sessionId || 'draft'}</span>
      <KnowledgeList />
    </KnowledgeProvider>
  );
}

const named: Pane = { tabId: 'tab-1', sessionId: NAMED, sessionName: 'Lab notes' };
const draft: Pane = { tabId: 'tab-2', sessionId: '' };

/** The `session_id` every selection write named, `null` for the machine-wide default. */
const writtenScopes = () =>
  mocks.setActive.mock.calls.map(
    ([options]) => (options as { body: { session_id?: string } }).body.session_id ?? null
  );

/** The selection reads, by the scope they asked for. */
const readScopes = () =>
  mocks.getActive.mock.calls.map(
    ([options]) => (options as { query?: { session_id?: string } }).query?.session_id ?? null
  );

async function settledOn(visible: string) {
  await waitFor(() => expect(screen.getByTestId('settled')).toHaveTextContent('settled'));
  await waitFor(() => expect(screen.getByTestId('visible')).toHaveTextContent(visible));
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  mocks.listBases.mockResolvedValue({ data: [base('alpha'), base('beta')] });
  mocks.getActive.mockResolvedValue({
    data: { kb_ids: ['alpha', 'beta'], primary_kb: 'alpha', active_kb: 'alpha', hidden_kbs: [] },
  });
  mocks.setActive.mockImplementation(async ({ body }: { body: { hidden_kbs?: string[] } }) => ({
    data: { kb_ids: [], primary_kb: null, active_kb: null, hidden_kbs: body.hidden_kbs ?? [] },
  }));
});

describe('the window’s one knowledge scope follows the focused tab', () => {
  it('a_draft_tab_beside_a_named_chat_writes_the_default_not_that_chat', async () => {
    const user = userEvent.setup();
    // A split: the named chat and a draft side by side, the named chat focused.
    const { rerender } = render(
      <Window pathname="/pair" panes={[named, draft]} activeTabId={named.tabId} />
    );
    await waitFor(() => expect(screen.getByTestId('app-chat')).toHaveTextContent(NAMED));
    await settledOn('alpha,beta');

    // The person clicks into the draft and presses Hide all in its `+` popover.
    rerender(<Window pathname="/pair" panes={[named, draft]} activeTabId={draft.tabId} />);
    await waitFor(() => expect(screen.getByTestId('app-chat')).toHaveTextContent('draft'));
    await settledOn('alpha,beta');
    mocks.setActive.mockClear();
    await user.click(screen.getByRole('button', { name: 'Hide all' }));

    await waitFor(() => expect(mocks.setActive).toHaveBeenCalledTimes(1));
    expect(writtenScopes()).toEqual([null]);
    expect(mocks.setActive.mock.calls[0][0].body.hidden_kbs).toEqual(['alpha', 'beta']);
    // The named chat's selection is neither written nor its local copy touched.
    expect(writtenScopes()).not.toContain(NAMED);
    expect(localStorage.getItem(`knowledge_hidden_kbs:${NAMED}`)).toBe('[]');
  });

  it('a draft opened as a new tab in the same pane leaves the named chat alone', async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <Window pathname="/pair" panes={[named]} activeTabId={named.tabId} />
    );
    await waitFor(() => expect(screen.getByTestId('app-chat')).toHaveTextContent(NAMED));
    await settledOn('alpha,beta');

    // Cmd+T: one pane, its chat replaced by the new tab's draft.
    rerender(<Window pathname="/pair" panes={[draft]} activeTabId={draft.tabId} />);
    await waitFor(() => expect(screen.getByTestId('app-chat')).toHaveTextContent('draft'));
    await settledOn('alpha,beta');
    mocks.setActive.mockClear();
    await user.click(screen.getByRole('button', { name: 'Hide all' }));

    await waitFor(() => expect(mocks.setActive).toHaveBeenCalledTimes(1));
    expect(writtenScopes()).toEqual([null]);
  });

  it('moves back to the named chat when it is focused again', async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <Window pathname="/pair" panes={[named, draft]} activeTabId={draft.tabId} />
    );
    await settledOn('alpha,beta');
    rerender(<Window pathname="/pair" panes={[named, draft]} activeTabId={named.tabId} />);
    await waitFor(() => expect(screen.getByTestId('app-chat')).toHaveTextContent(NAMED));
    await settledOn('alpha,beta');
    mocks.setActive.mockClear();
    await user.click(screen.getByRole('button', { name: 'Hide all' }));

    await waitFor(() => expect(mocks.setActive).toHaveBeenCalledTimes(1));
    expect(writtenScopes()).toEqual([NAMED]);
  });

  it('a background chat whose name arrives late does not take the scope', async () => {
    const { rerender } = render(
      <Window
        pathname="/pair"
        panes={[{ ...named, sessionName: undefined }, draft]}
        activeTabId={draft.tabId}
      />
    );
    await settledOn('alpha,beta');
    // The named chat loads in the background pane and its name arrives.
    rerender(<Window pathname="/pair" panes={[named, draft]} activeTabId={draft.tabId} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByTestId('app-chat')).toHaveTextContent('draft');
    expect(readScopes()).not.toContain(NAMED);
  });

  it('Home’s list is the default for chats, not the chat that was open last', async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <Window pathname="/pair" panes={[named]} activeTabId={named.tabId} />
    );
    await waitFor(() => expect(screen.getByTestId('app-chat')).toHaveTextContent(NAMED));
    await settledOn('alpha,beta');

    // The person goes Home: /pair unmounts its panes; the app-level chat stays.
    rerender(<Window pathname="/" panes={[]} />);
    await settledOn('alpha,beta');
    mocks.setActive.mockClear();
    await user.click(screen.getByRole('button', { name: 'Hide all' }));

    await waitFor(() => expect(mocks.setActive).toHaveBeenCalledTimes(1));
    expect(writtenScopes()).toEqual([null]);
    expect(screen.getByTestId('app-chat')).toHaveTextContent(NAMED);
  });

  it('keeps the last chat’s scope on the Knowledge view, as it always has', () => {
    expect(knowledgeScopeFor('/knowledge', NAMED)).toBe(NAMED);
    expect(knowledgeScopeFor('/pair', NAMED)).toBe(NAMED);
    expect(knowledgeScopeFor('/pair', '')).toBeNull();
    expect(knowledgeScopeFor('/', NAMED)).toBeNull();
  });

  it('isFocusedChatPane: the active tab on /pair, a placeholder only without a session', () => {
    const groups = (tabId?: string) => ({ activeTab: tabId ? { tabId } : undefined });
    expect(isFocusedChatPane(groups('tab-2'), 'tab-2', '')).toBe(true);
    expect(isFocusedChatPane(groups('tab-2'), 'tab-1', NAMED)).toBe(false);
    expect(isFocusedChatPane(groups(), 'grp-1-empty', '')).toBe(true);
    expect(isFocusedChatPane(groups(), 'tab-1', NAMED)).toBe(false);
    // Off /pair there is no tab strip, and the one chat on screen is the focused one.
    expect(isFocusedChatPane(null, undefined, NAMED)).toBe(true);
  });

  /**
   * The harness above is the wiring with the real pieces; what it cannot see is
   * that BaseChat and App hand those pieces their real inputs. BaseChat cannot be
   * mounted in jsdom (`BaseChat.privacy.test.tsx`), so the two call sites are read.
   */
  it('BaseChat mirrors itself only through the focus rule, and App scopes by it', () => {
    // vitest runs with `ui/desktop` as its root.
    const read = (...parts: string[]) => readFileSync(path.join(process.cwd(), ...parts), 'utf8');
    const baseChat = read('src', 'components', 'BaseChat.tsx');
    expect(baseChat).toMatch(
      /useMirrorFocusedChat\(\{\s*focused: isFocusedChatPane\(chatGroups, terminalKey, sessionId\),\s*sessionId,\s*sessionName: session\?\.name,/
    );
    // The hook's own write is the only one: no other path moves the app-level chat.
    expect(baseChat.match(/\bsetChat\(/g)).toHaveLength(1);
    expect(read('src', 'App.tsx')).toMatch(
      /<KnowledgeProvider sessionId=\{knowledgeScopeFor\(pathname, chat\.sessionId\)\}>/
    );
  });
});
