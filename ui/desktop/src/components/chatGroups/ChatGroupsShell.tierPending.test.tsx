import { act, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * A private chat's subagent tabs must not flash PUBLIC while their tier is read.
 *
 * Measured 2026-09-14 on the 1.90.4 release candidate (main 1038a113), driving
 * the dev app over CDP: a Versa GPT-5.5 chat delegated two tasks, so the window
 * held the parent tab and two `sub_agent` tabs, all `privacy_tier=private`.
 * Settings → click the chat in the sidebar remounts this shell, and the two
 * subagent tabs drew `data-privacy="public"` until their own rows answered
 * (~0.5 s here, 3.2 s on the tester's machine); after a reload, all three tabs
 * drew public at 440 ms and the subagents stayed public until 1.9 s.
 *
 * Why the window exists at all: the session list is `include_subagents=false`,
 * no chat store exists for a tab this window has not opened, so the only source
 * for a subagent tab's tier is its own `metadata_only` read — and that is
 * per-hook state, empty on every mount. The map correctly leaves the tab out
 * while the read is pending. The glyph then drew "absent" as Public.
 *
 * This mounts the REAL strip and the REAL glyph (only BaseChat is stubbed, as
 * the sibling shell suites do), because every layer above the glyph was already
 * correct and a test of any one of them passes on the bug.
 */

const dispatch = vi.fn();
let cachedList: Array<{ id: string; name: string; privacy_tier?: string }> | null = null;

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
  resolve: (row: { id: string; name: string; privacy_tier?: string }) => void;
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
    tabAnnotations: {
      'sub-alpha': { badge: 'subagent', parentSessionId: 'parent' },
    },
    state: {
      activeGroupId: 'g1',
      layout: { kind: 'leaf', groupId: 'g1' },
      groups: {
        g1: {
          id: 'g1',
          activeTabId: 't-parent',
          tabs: [
            { tabId: 't-parent', sessionId: 'parent', title: 'Delegation', userSetName: false },
            {
              tabId: 't-alpha',
              sessionId: 'sub-alpha',
              title: 'Subagent: Reply with ALPHA',
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

describe('ChatGroupsShell — a subagent tab whose tier is still being read', () => {
  beforeEach(() => {
    reads = [];
    dispatch.mockClear();
    // The list has loaded: it carries the parent (private) and an ordinary
    // public chat, and — being `include_subagents=false` — not the subagent.
    cachedList = [
      { id: 'parent', name: 'Delegation', privacy_tier: 'private' },
      { id: 'open-chat', name: 'Public notes', privacy_tier: 'public' },
    ];
  });

  it('is drawn as not-yet-known, never as public, until its row answers', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();

    expect(screen.getAllByTestId('chat-kind-icon').length).toBeGreaterThanOrEqual(3);
    expect(reads.map((read) => read.sessionId)).toEqual(['sub-alpha']);

    // The read is out. This is the window the tester measured.
    const pending = glyphOf('t-alpha');
    expect(pending).toHaveAttribute('data-chat-kind', 'subagent');
    expect(pending).not.toHaveAttribute('data-privacy', 'public');
    expect(pending).toHaveAttribute('data-privacy', 'unknown');

    reads[0].resolve({
      id: 'sub-alpha',
      name: 'Subagent: Reply with ALPHA',
      privacy_tier: 'private',
    });
    await flush();
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'private');
  });

  it('shows a public chat the list has read as public at once, without waiting on any read', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    // No flush, no read answered: the list is the source and it is already here.
    expect(glyphOf('t-open')).toHaveAttribute('data-privacy', 'public');
    expect(glyphOf('t-parent')).toHaveAttribute('data-privacy', 'private');
    await flush();
    expect(reads.map((read) => read.sessionId)).not.toContain('open-chat');
  });

  it('stays not-yet-known when the read is refused, rather than settling on public', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    reads[0].reject('That chat is private, or there is no chat with that id.');
    await flush();
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'unknown');
  });

  it('draws every tab as not-yet-known before the list has loaded at all (a reload)', async () => {
    cachedList = null;
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    for (const tabId of ['t-parent', 't-alpha', 't-open']) {
      expect(glyphOf(tabId)).toHaveAttribute('data-privacy', 'unknown');
    }
  });
});
