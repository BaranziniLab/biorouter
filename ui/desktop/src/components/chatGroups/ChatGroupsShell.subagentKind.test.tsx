import { act, render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * A delegated subagent's tab must still read as a sub-agent after the window
 * has been somewhere else.
 *
 * Measured 2026-09-14 on main 1038a113, driving the dev app over CDP: a Versa
 * GPT-5.5 chat delegated two tasks, so the window held two tabs whose glyph
 * read `data-chat-kind="subagent"`. After Settings and back to the chat through
 * the sidebar (or History and back, or a reload) both read
 * `data-chat-kind="chat"` — a speech bubble or a padlocked bubble in place of
 * the Bot glyph — while their tier stayed correct.
 *
 * Why: the strip marked a sub-agent only from `tabAnnotations[id].badge`, and
 * that map is React state in `ChatGroupsProvider`, written only from live
 * daemon workspace frames. The provider mounts inside the `/pair` route, so
 * leaving `/pair` drops it, and a reload never had it. What a remounted shell
 * DOES have is the tab's own session row: a subagent's chat is never in the
 * session list (`include_subagents=false`), so the shell already reads it with
 * `GET /sessions/{id}?metadata_only=true` for its name and tier. That row says
 * `session_type: 'sub_agent'`.
 *
 * So this mounts the shell exactly as it is after a route change — no
 * annotations at all — with the REAL strip and the REAL glyph (only BaseChat is
 * stubbed, as the sibling shell suites do), and answers those reads.
 */

const dispatch = vi.fn();
type Row = {
  id: string;
  name: string;
  privacy_tier?: string;
  session_type?: string;
  parent_session_id?: string | null;
};
let cachedList: Row[] | null = null;
let tabAnnotations: Record<string, { badge?: string; parentSessionId?: string }> = {};
let alphaTitle = 'Subagent: Reply with ALPHA';

vi.mock('../BaseChat', () => ({
  default: (props: { renderSessionTitle?: () => React.ReactNode }) => (
    <div data-testid="basechat">{props.renderSessionTitle?.()}</div>
  ),
}));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => cachedList,
  subscribeSessionList: () => () => {},
  preloadSessionList: () => {},
}));

vi.mock('../../hooks/chatStreamStore', () => ({
  useLiveSessionTiers: () => ({}),
}));

type PendingRead = {
  sessionId: string;
  resolve: (row: Row) => void;
  reject: (error: unknown) => void;
};
let reads: PendingRead[] = [];
vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  getSession: (options: { path: { session_id: string } }) =>
    new Promise((resolve, reject) => {
      reads.push({
        sessionId: options.path.session_id,
        resolve: (row) => resolve({ data: row }),
        reject,
      });
    }),
}));
vi.mock('../../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../utils/userAction')>()),
  userActionHeaders: async () => ({ 'X-User-Action': 'proof-of-user' }),
}));

vi.mock('../../contexts/ChatGroupsContext', () => ({
  useChatGroups: () => ({
    dispatch,
    runningSessionIds: [],
    tabAnnotations,
    state: {
      activeGroupId: 'g1',
      layout: { kind: 'leaf', groupId: 'g1' },
      groups: {
        g1: {
          id: 'g1',
          activeTabId: 't-parent',
          tabs: [
            { tabId: 't-parent', sessionId: 'parent', title: 'Delegation', userSetName: false },
            { tabId: 't-alpha', sessionId: 'sub-alpha', title: alphaTitle, userSetName: false },
            {
              tabId: 't-beta',
              sessionId: 'sub-beta',
              title: 'Subagent: Reply with BETA',
              userSetName: false,
            },
            { tabId: 't-open', sessionId: 'open-chat', title: 'Public notes', userSetName: false },
          ],
        },
      },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

async function flush() {
  await act(async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  });
}

function glyphOf(tabId: string): HTMLElement {
  const tab = document.querySelector(`[data-tab-id="${tabId}"]`);
  const glyph = tab?.querySelector<HTMLElement>('[data-testid="chat-kind-icon"]');
  if (!glyph) throw new Error(`no glyph rendered for ${tabId}`);
  return glyph;
}

function kindOf(tabId: string): string | null {
  return glyphOf(tabId).getAttribute('data-chat-kind');
}

function readOf(sessionId: string): PendingRead {
  const read = reads.find((r) => r.sessionId === sessionId);
  if (!read) throw new Error(`no read issued for ${sessionId}`);
  return read;
}

const alphaRow: Row = {
  id: 'sub-alpha',
  name: 'Subagent: Reply with ALPHA',
  privacy_tier: 'private',
  session_type: 'sub_agent',
  parent_session_id: 'parent',
};
const betaRow: Row = {
  id: 'sub-beta',
  name: 'Subagent: Reply with BETA',
  privacy_tier: 'private',
  session_type: 'sub_agent',
  parent_session_id: 'parent',
};

describe('ChatGroupsShell — a subagent tab after the workspace annotations are gone', () => {
  beforeEach(() => {
    reads = [];
    dispatch.mockClear();
    tabAnnotations = {};
    alphaTitle = 'Subagent: Reply with ALPHA';
    // The list has loaded and, being `include_subagents=false`, carries neither
    // subagent.
    cachedList = [
      { id: 'parent', name: 'Delegation', privacy_tier: 'private', session_type: 'user' },
      { id: 'open-chat', name: 'Public notes', privacy_tier: 'public', session_type: 'user' },
    ];
  });

  it('marks both subagent tabs from their own rows once the rows answer', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(reads.map((r) => r.sessionId).sort()).toEqual(['sub-alpha', 'sub-beta']);

    readOf('sub-alpha').resolve(alphaRow);
    readOf('sub-beta').resolve(betaRow);
    await flush();

    expect(kindOf('t-alpha')).toBe('subagent');
    expect(kindOf('t-beta')).toBe('subagent');
    // The tier still arrives from the same row, untouched by this.
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'private');
    // And nothing else was re-kinded by it.
    expect(kindOf('t-parent')).toBe('chat');
    expect(kindOf('t-open')).toBe('chat');
  });

  it('marks nothing from a row whose type is not sub_agent, even when it names a parent', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    // The strip's rule, kept for the row: a parent link alone is not a kind.
    readOf('sub-alpha').resolve({ ...alphaRow, session_type: 'user' });
    await flush();
    expect(kindOf('t-alpha')).toBe('chat');
  });

  it('leaves the tab unmarked when the read is refused', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').reject('That chat is private, or there is no chat with that id.');
    await flush();
    expect(kindOf('t-alpha')).toBe('chat');
  });

  it('keeps the kind even when the tab was renamed while its row was being read', async () => {
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    // The name channel renamed the tab mid-read, so rule 4 drops the NAME in
    // the answer. The kind is a fact about the session, not about that title.
    alphaTitle = 'Echo check';
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').resolve(alphaRow);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');
    expect(dispatch).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: 'renameTab', sessionId: 'sub-alpha' })
    );
  });

  it('still marks a tab the live annotation names, before any row has answered', async () => {
    tabAnnotations = { 'sub-alpha': { badge: 'subagent', parentSessionId: 'parent' } };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');
    expect(kindOf('t-beta')).toBe('chat');
  });
});
