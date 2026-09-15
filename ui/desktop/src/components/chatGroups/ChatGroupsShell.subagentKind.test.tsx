import { Profiler } from 'react';
import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

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
 * DOES have is the tab's chat's session row, and that row says
 * `session_type: 'sub_agent'`. It arrives one of two ways:
 *
 *   - Usually a subagent's chat is NOT in the session list
 *     (`include_subagents=false`), so the shell reads it on its own with
 *     `GET /sessions/{id}?metadata_only=true`, as it already did for its name
 *     and tier.
 *   - But the list cache is module-global, and History's "Show subagent runs"
 *     refetches it WITH subagents, where it stays until something asks for the
 *     other flag. Then the subagent's tab IS listed and no read is ever made.
 *     The first version of this fix read the type only from the singular read,
 *     and an independent tester measured the result on 2026-09-14, desktop and
 *     `biorouter serve` alike: History with the box ticked → back, and both
 *     subagent tabs read `data-chat-kind="chat"` for 35 s.
 *
 * So this mounts the shell exactly as it is after a route change — no
 * annotations at all — with the REAL strip and the REAL glyph (only BaseChat is
 * stubbed, as the sibling shell suites do), and serves both.
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
type Tab = { tabId: string; sessionId: string; title: string; userSetName: boolean };
let tabs: Tab[] = [];
let activeTabId = 't-parent';
const listListeners = new Set<() => void>();

vi.mock('../BaseChat', () => ({
  default: (props: { renderSessionTitle?: () => React.ReactNode }) => (
    <div data-testid="basechat">{props.renderSessionTitle?.()}</div>
  ),
}));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => cachedList,
  subscribeSessionList: (listener: () => void) => {
    listListeners.add(listener);
    return () => listListeners.delete(listener);
  },
  preloadSessionList: () => {},
}));

/**
 * The live chat stream store, as the shell reads it: a tier and a session type
 * per chat whose store has loaded, published together from one row. Mocked on
 * `useSyncExternalStore`, the way the real hooks read the registry, so an
 * emission re-renders the shell exactly as the store's does.
 */
let liveTiers: Record<string, string> = {};
let liveTypes: Record<string, string> = {};
const liveListeners = new Set<() => void>();
vi.mock('../../hooks/chatStreamStore', async () => {
  const { useSyncExternalStore } = await import('react');
  const subscribe = (listener: () => void) => {
    liveListeners.add(listener);
    return () => {
      liveListeners.delete(listener);
    };
  };
  return {
    useLiveSessionTiers: () => useSyncExternalStore(subscribe, () => liveTiers),
    useLiveSessionTypes: () => useSyncExternalStore(subscribe, () => liveTypes),
  };
});

/**
 * One `GET /sessions/{id}` in flight. The read passes no `throwOnError`, so the
 * generated client RESOLVES for every outcome it has a response for, and the
 * three helpers below are the three shapes it resolves with.
 */
type PendingRead = {
  sessionId: string;
  resolve: (row: Row) => void;
  /** The daemon answered, and the answer was no: 403 refused, 404 gone. */
  refuse: (status: number, body?: string) => void;
  /** No answer at all: the client caught fetch's throw and has no response. */
  unanswered: () => void;
  reject: (error: unknown) => void;
};
let reads: PendingRead[] = [];
vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  getSession: (options: { path: { session_id: string } }) =>
    new Promise((resolve, reject) => {
      reads.push({
        sessionId: options.path.session_id,
        resolve: (row) => resolve({ data: row, response: { status: 200 } }),
        refuse: (status, body = 'That chat is private, or there is no chat with that id.') =>
          resolve({ data: undefined, error: body, response: { status } }),
        unanswered: () =>
          resolve({
            data: undefined,
            error: new TypeError('Failed to fetch'),
            response: undefined,
          }),
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
          activeTabId,
          tabs,
        },
      },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';
import { clearSessionTypeMemory } from './sessionTypeMemory';

const defaultTabs = (): Tab[] => [
  { tabId: 't-parent', sessionId: 'parent', title: 'Delegation', userSetName: false },
  {
    tabId: 't-alpha',
    sessionId: 'sub-alpha',
    title: 'Subagent: Reply with ALPHA',
    userSetName: false,
  },
  {
    tabId: 't-beta',
    sessionId: 'sub-beta',
    title: 'Subagent: Reply with BETA',
    userSetName: false,
  },
  { tabId: 't-open', sessionId: 'open-chat', title: 'Public notes', userSetName: false },
];

/**
 * `kind|privacy` of alpha's glyph after EVERY commit, read from the real DOM.
 *
 * A `Profiler`, not an assertion after `render`: testing-library's `render` and
 * `act` flush effects AND the renders those effects schedule before returning,
 * so a state that is committed for one frame — painted, in the app — and
 * replaced by an effect is invisible to an assertion made afterwards. The
 * `onRender` callback runs inside each commit, after the DOM has been written.
 */
let commits: string[] = [];
function sampleAlpha() {
  const glyph = document.querySelector('[data-tab-id="t-alpha"] [data-testid="chat-kind-icon"]');
  if (glyph) {
    commits.push(`${glyph.getAttribute('data-chat-kind')}|${glyph.getAttribute('data-privacy')}`);
  }
}
const shell = () => (
  <Profiler id="shell" onRender={sampleAlpha}>
    <ChatGroupsShell onChatChange={() => {}} />
  </Profiler>
);

// Every describe below starts from a window whose stores have loaded nothing.
beforeEach(() => {
  liveTiers = {};
  liveTypes = {};
  liveListeners.clear();
  activeTabId = 't-parent';
  commits = [];
});

/** The store publishing what a loaded chat's row said, as the registry does. */
async function emitLive(next: { tiers?: Record<string, string>; types?: Record<string, string> }) {
  if (next.types) liveTypes = next.types;
  if (next.tiers) liveTiers = next.tiers;
  await act(async () => {
    for (const listener of [...liveListeners]) listener();
  });
}

/** The cache announcing a new list, as `refreshSessionList` does. */
async function emitList(rows: Row[]) {
  cachedList = rows;
  await act(async () => {
    for (const listener of [...listListeners]) listener();
  });
}

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
    tabs = defaultTabs();
    listListeners.clear();
    // The memory is module state and outlives a mount by design, so it would
    // outlive a test too.
    clearSessionTypeMemory();
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
    readOf('sub-alpha').refuse(403);
    await flush();
    expect(kindOf('t-alpha')).toBe('chat');
  });

  it('keeps the kind even when the tab was renamed while its row was being read', async () => {
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    // The name channel renamed the tab mid-read, so rule 4 drops the NAME in
    // the answer. The kind is a fact about the session, not about that title.
    tabs = tabs.map((tab) => (tab.tabId === 't-alpha' ? { ...tab, title: 'Echo check' } : tab));
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

describe('ChatGroupsShell — a subagent tab whose row is IN the cached list', () => {
  const parentRow: Row = {
    id: 'parent',
    name: 'Delegation',
    privacy_tier: 'private',
    session_type: 'user',
  };
  const openRow: Row = {
    id: 'open-chat',
    name: 'Public notes',
    privacy_tier: 'public',
    session_type: 'user',
  };

  beforeEach(() => {
    reads = [];
    dispatch.mockClear();
    tabAnnotations = {};
    tabs = defaultTabs();
    listListeners.clear();
    // The memory is module state and outlives a mount by design, so it would
    // outlive a test too.
    clearSessionTypeMemory();
    // What History's "Show subagent runs" leaves behind: the module-global
    // cache was refetched with `include_subagents=true`, so both subagents'
    // rows are in it — as measured, 20260914_2 and _3 as sub_agent/private.
    cachedList = [parentRow, alphaRow, betaRow, openRow];
  });

  it('marks both subagent tabs from the listed rows, with no read of their own', async () => {
    render(shell());
    // From the FIRST commit: the cached list's tier is drawn on the first render
    // (`useSessionListTiers`), so its type must be too, or a tab opened from
    // History's subagent list commits as a private plain chat before the
    // reconcile effect runs.
    expect(commits[0]).toBe('subagent|private');
    expect(commits).not.toContain('chat|private');
    await flush();
    // Nothing is left out of this list, so nothing is read on its own…
    expect(reads).toEqual([]);
    // …and the listed rows are the only thing that can say what these tabs are.
    expect(kindOf('t-alpha')).toBe('subagent');
    expect(kindOf('t-beta')).toBe('subagent');
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'private');
    expect(kindOf('t-parent')).toBe('chat');
    expect(kindOf('t-open')).toBe('chat');
  });

  it('marks a tab once a list carrying its row is published, even after its own read failed', async () => {
    cachedList = [parentRow, openRow];
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').unanswered();
    readOf('sub-beta').unanswered();
    await flush();
    expect(kindOf('t-alpha')).toBe('chat');

    // History ticks "Show subagent runs" in this window: the cache now holds them.
    await emitList([parentRow, alphaRow, betaRow, openRow]);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');
    expect(kindOf('t-beta')).toBe('subagent');
  });

  it('marks nothing from a listed row whose type is not sub_agent, even when it names a parent', async () => {
    cachedList = [parentRow, { ...alphaRow, session_type: 'user' }, betaRow, openRow];
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(kindOf('t-alpha')).toBe('chat');
    expect(kindOf('t-beta')).toBe('subagent');
  });

  it('keeps the kind when the list drops the row again and the read is refused', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');

    // History remounts with the box unticked and refetches without subagents.
    await emitList([parentRow, openRow]);
    await flush();
    readOf('sub-alpha').refuse(403);
    await flush();
    // The type was a fact about the session; a refused read changes nothing.
    expect(kindOf('t-alpha')).toBe('subagent');
  });

  it("does not hand a closed subagent tab's kind to a chat that reuses its id", async () => {
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');

    // The subagent's tab is closed and its chat deleted…
    tabs = tabs.filter((tab) => tab.tabId !== 't-alpha');
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    await emitList([parentRow, betaRow, openRow]);
    await flush();

    // …and the id is reissued to a new chat, opened in a new tab before it has
    // recorded a message. `create_session`'s high-water mark makes ids single
    // use, so only a store without it can do this: an older build sharing the
    // file, or a database restored from a backup. The forgetting also bounds
    // the map.
    tabs = [
      ...tabs,
      { tabId: 't-new', sessionId: 'sub-alpha', title: 'New chat', userSetName: false },
    ];
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    expect(kindOf('t-new')).toBe('chat');
    readOf('sub-alpha').resolve({ id: 'sub-alpha', name: 'New chat', session_type: 'user' });
    await flush();
    expect(kindOf('t-new')).toBe('chat');
  });
});

/**
 * The glyph flip on a remount (follow-up to PR #314).
 *
 * The type map was React state in the shell, which mounts inside `/pair`, so
 * it started empty on every return to a chat. Until the row was read again a
 * subagent's tab drew the chat bubble, then switched to the Bot. Measured
 * 2026-09-14 on d7f02191 in the dev app: `chat/unknown` at 123 ms after
 * Settings → a sidebar chat and `subagent/private` at 751 ms; History → back,
 * 81 ms then 157 ms. The type now also lives in `sessionTypeMemory`, and the
 * shell's state is seeded from it on mount.
 */
describe('ChatGroupsShell — a subagent tab on a remount, before its row is read again', () => {
  beforeEach(() => {
    reads = [];
    dispatch.mockClear();
    tabAnnotations = {};
    tabs = defaultTabs();
    listListeners.clear();
    clearSessionTypeMemory();
    cachedList = [
      { id: 'parent', name: 'Delegation', privacy_tier: 'private', session_type: 'user' },
      { id: 'open-chat', name: 'Public notes', privacy_tier: 'public', session_type: 'user' },
    ];
  });

  it('draws the Bot glyph on the first render of a remount, with no flush', async () => {
    const first = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').resolve(alphaRow);
    readOf('sub-beta').resolve(betaRow);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');

    // Settings or History: leaving `/pair` unmounts the shell, and the
    // annotations go with the provider.
    first.unmount();
    reads = [];
    render(<ChatGroupsShell onChatChange={() => {}} />);

    // No flush, and no row has answered for this mount.
    expect(kindOf('t-alpha')).toBe('subagent');
    expect(kindOf('t-beta')).toBe('subagent');
    expect(kindOf('t-parent')).toBe('chat');
    expect(kindOf('t-open')).toBe('chat');
    // Only the kind is remembered. The tier stays not yet known until the row
    // answers again, which is #312's state and not this one's to change.
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'unknown');

    await flush();
    readOf('sub-alpha').resolve(alphaRow);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'private');
  });

  it('remembers a row that answers after the shell has gone', async () => {
    const first = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    // A reload, then away from the chat before its reads came back.
    first.unmount();
    readOf('sub-alpha').resolve(alphaRow);
    await flush();

    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(kindOf('t-alpha')).toBe('subagent');
    // Beta never answered, so nothing is remembered for it.
    expect(kindOf('t-beta')).toBe('chat');
  });

  it('forgets a closed tab, so a later mount does not seed a reissued id with its kind', async () => {
    const first = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').resolve(alphaRow);
    await flush();
    tabs = tabs.filter((tab) => tab.tabId !== 't-alpha');
    first.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    first.unmount();

    // A store without the high-water mark reissues the id, and the new chat is
    // opened in a tab while the shell is away.
    tabs = [
      ...tabs,
      { tabId: 't-new', sessionId: 'sub-alpha', title: 'New chat', userSetName: false },
    ];
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(kindOf('t-new')).toBe('chat');
  });

  it("forgets a closed tab's tier too, so a reissued id is not yet known rather than private", async () => {
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').resolve(alphaRow);
    await flush();
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'private');

    tabs = tabs.filter((tab) => tab.tabId !== 't-alpha');
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    reads = [];
    tabs = [
      ...tabs,
      { tabId: 't-new', sessionId: 'sub-alpha', title: 'New chat', userSetName: false },
    ];
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    // Its own row has not answered yet.
    expect(glyphOf('t-new')).toHaveAttribute('data-privacy', 'unknown');

    readOf('sub-alpha').resolve({
      id: 'sub-alpha',
      name: 'New chat',
      privacy_tier: 'public',
      session_type: 'user',
    });
    await flush();
    // Kept, the closed tab's private would have been raised over this for good.
    expect(glyphOf('t-new')).toHaveAttribute('data-privacy', 'public');
  });
});

/**
 * A failed row read was never retried (follow-up to PR #314).
 *
 * Rule 3 marks a chat as read for a list when the read is ISSUED, so a read
 * that failed waited for the next list. Measured 2026-09-14 on d7f02191 in the
 * dev app: every `metadata_only` read failed with a connection error during
 * Settings → a sidebar chat, and 25 s after the daemon was answering again both
 * subagent tabs still read `chat/unknown`, with no read issued in that time.
 */
describe('ChatGroupsShell — a row read nobody answered is asked again', () => {
  beforeEach(() => {
    reads = [];
    dispatch.mockClear();
    tabAnnotations = {};
    tabs = defaultTabs();
    listListeners.clear();
    clearSessionTypeMemory();
    cachedList = [
      { id: 'parent', name: 'Delegation', privacy_tier: 'private', session_type: 'user' },
      { id: 'open-chat', name: 'Public notes', privacy_tier: 'public', session_type: 'user' },
    ];
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const readsOf = (sessionId: string) => reads.filter((read) => read.sessionId === sessionId);
  const lastReadOf = (sessionId: string) => {
    const all = readsOf(sessionId);
    if (all.length === 0) throw new Error(`no read issued for ${sessionId}`);
    return all[all.length - 1];
  };
  async function advance(ms: number) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
    await flush();
  }

  it.each<[string, (read: PendingRead) => void]>([
    ['no response at all', (read) => read.unanswered()],
    ['a 503', (read) => read.refuse(503, 'Service Unavailable')],
    ['a 429', (read) => read.refuse(429, 'Too Many Requests')],
    // No HTTP status at all. Measured: the Electron renderer reports a
    // CDP-fulfilled 503 as status 0, and the first cut of this retried only
    // `>= 500`, so that probe saw no retry.
    ['a status of 0', (read) => read.refuse(0, '')],
    ['a throw', (read) => read.reject(new TypeError('Failed to fetch'))],
  ])(
    'reads the row again after %s, and the tab gains kind and tier with no new list',
    async (_label, fail) => {
      render(<ChatGroupsShell onChatChange={() => {}} />);
      await flush();
      fail(readOf('sub-alpha'));
      fail(readOf('sub-beta'));
      await flush();
      expect(kindOf('t-alpha')).toBe('chat');
      expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'unknown');

      await advance(1_499);
      expect(readsOf('sub-alpha')).toHaveLength(1);
      await advance(1);
      expect(readsOf('sub-alpha')).toHaveLength(2);
      expect(readsOf('sub-beta')).toHaveLength(2);

      lastReadOf('sub-alpha').resolve(alphaRow);
      lastReadOf('sub-beta').resolve(betaRow);
      await flush();
      // No list was published in this test: `emitList` is never called.
      expect(kindOf('t-alpha')).toBe('subagent');
      expect(kindOf('t-beta')).toBe('subagent');
      expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'private');
      expect(glyphOf('t-beta')).toHaveAttribute('data-privacy', 'private');

      // Answered, so never asked about again for this list.
      await advance(60_000);
      expect(readsOf('sub-alpha')).toHaveLength(2);
    }
  );

  it.each([403, 404])('does not ask again after a %i: a refusal is an answer', async (status) => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').refuse(status);
    await flush();
    await advance(60_000);
    expect(readsOf('sub-alpha')).toHaveLength(1);
    expect(kindOf('t-alpha')).toBe('chat');
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'unknown');
  });

  it('backs off, and stops after five retries for one list', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    lastReadOf('sub-alpha').unanswered();
    await flush();
    for (const [index, delay] of [1_500, 3_000, 6_000, 10_000, 10_000].entries()) {
      await advance(delay - 1);
      expect(readsOf('sub-alpha')).toHaveLength(index + 1);
      await advance(1);
      expect(readsOf('sub-alpha')).toHaveLength(index + 2);
      lastReadOf('sub-alpha').unanswered();
      await flush();
    }
    await advance(120_000);
    expect(readsOf('sub-alpha')).toHaveLength(6);
  });

  it('asks again from the start once a new list arrives after the retries ran out', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    lastReadOf('sub-alpha').unanswered();
    await flush();
    for (let i = 0; i < 5; i++) {
      await advance(10_000);
      lastReadOf('sub-alpha').unanswered();
      await flush();
    }
    await advance(120_000);
    expect(readsOf('sub-alpha')).toHaveLength(6);

    // The daemon answered a list, so rule 3 reads the chat for it…
    await emitList([...(cachedList ?? [])]);
    await flush();
    expect(readsOf('sub-alpha')).toHaveLength(7);
    // …and a failure of that read is retried again, from the shortest wait.
    lastReadOf('sub-alpha').unanswered();
    await flush();
    await advance(1_500);
    expect(readsOf('sub-alpha')).toHaveLength(8);
  });

  it('keeps only the newest read landing: an overtaken read that fails asks nothing again', async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    const overtaken = readOf('sub-alpha');
    // A new list while the first read is out: rule 3 reads the chat again.
    await emitList([...(cachedList ?? [])]);
    await flush();
    expect(readsOf('sub-alpha')).toHaveLength(2);

    overtaken.unanswered();
    await flush();
    lastReadOf('sub-alpha').resolve(alphaRow);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');

    await advance(60_000);
    expect(readsOf('sub-alpha')).toHaveLength(2);
  });

  it("cannot let an overtaken read that fails later cancel the newest read's retry", async () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    const overtaken = readOf('sub-alpha');
    await emitList([...(cachedList ?? [])]);
    await flush();
    // The newest read fails first, and is due to be asked again…
    lastReadOf('sub-alpha').unanswered();
    await flush();
    // …then the overtaken one fails too, and must not touch that.
    overtaken.unanswered();
    await flush();

    await advance(1_500);
    expect(readsOf('sub-alpha')).toHaveLength(3);
    lastReadOf('sub-alpha').resolve(alphaRow);
    await flush();
    expect(kindOf('t-alpha')).toBe('subagent');
  });

  it('does not ask about a tab that was closed while it waited', async () => {
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').unanswered();
    await flush();
    tabs = tabs.filter((tab) => tab.tabId !== 't-alpha');
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    await advance(60_000);
    expect(readsOf('sub-alpha')).toHaveLength(1);
  });

  it('does not ask again after the shell has gone', async () => {
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    await flush();
    readOf('sub-alpha').unanswered();
    await flush();
    view.unmount();
    await advance(60_000);
    expect(readsOf('sub-alpha')).toHaveLength(1);
  });
});

/**
 * An ACTIVE subagent tab after a reload (follow-up 3 to PR #314).
 *
 * A reload empties both the workspace annotations and `sessionTypeMemory`, so
 * the kind had one source left: the row read, which cannot start until the
 * session list lands. The ACTIVE tab's tier does not wait for that — its
 * BaseChat loads the chat into the live store, and the store publishes the
 * row's tier. Measured 2026-09-14 on b6fab4a1 by an independent tester: the
 * active subagent tab read `data-chat-kind="chat" data-privacy="private"` from
 * 649 to 1634 ms and from 1000 to 2295 ms on the desktop, and from 385 to
 * 1371 ms on `biorouter serve` — an undimmed plain chat, marked private — and
 * only then the Bot. The store held `session_type: 'sub_agent'` on that same
 * row the whole time; it now publishes it beside the tier.
 */
describe('ChatGroupsShell — an active subagent tab whose chat the live store has loaded', () => {
  beforeEach(() => {
    reads = [];
    dispatch.mockClear();
    tabAnnotations = {};
    tabs = defaultTabs();
    listListeners.clear();
    // A reload: nothing remembered, and the session list has not landed.
    clearSessionTypeMemory();
    cachedList = null;
    activeTabId = 't-alpha';
  });

  it('marks the tab from the store, before any list or row read, and never as a private plain chat', async () => {
    render(shell());
    await flush();
    // Nothing has said anything yet: no list, so no row read either.
    expect(reads).toEqual([]);
    expect(kindOf('t-alpha')).toBe('chat');
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'unknown');

    // BaseChat's load lands in the store, which publishes the row it read.
    await emitLive({ types: { 'sub-alpha': 'sub_agent' }, tiers: { 'sub-alpha': 'private' } });
    await flush();

    expect(kindOf('t-alpha')).toBe('subagent');
    expect(glyphOf('t-alpha')).toHaveAttribute('data-privacy', 'private');
    // Still no list and no read: the store was the only source.
    expect(reads).toEqual([]);
    // And at no commit in between was it a plain chat marked private.
    expect(commits.length).toBeGreaterThan(1);
    expect(commits).not.toContain('chat|private');
    // The other tabs were not re-kinded by it.
    expect(kindOf('t-beta')).toBe('chat');
    expect(kindOf('t-parent')).toBe('chat');
  });

  it('keeps it through the list landing and its own row read answering', async () => {
    render(shell());
    await flush();
    await emitLive({ types: { 'sub-alpha': 'sub_agent' }, tiers: { 'sub-alpha': 'private' } });
    await flush();

    await emitList([
      { id: 'parent', name: 'Delegation', privacy_tier: 'private', session_type: 'user' },
      { id: 'open-chat', name: 'Public notes', privacy_tier: 'public', session_type: 'user' },
    ]);
    await flush();
    readOf('sub-alpha').resolve(alphaRow);
    readOf('sub-beta').resolve(betaRow);
    await flush();

    expect(kindOf('t-alpha')).toBe('subagent');
    expect(kindOf('t-beta')).toBe('subagent');
    expect(commits).not.toContain('chat|private');
  });

  it('remembers what the store said, so a remount paints the Bot before anything answers', async () => {
    const first = render(shell());
    await flush();
    await emitLive({ types: { 'sub-alpha': 'sub_agent' }, tiers: { 'sub-alpha': 'private' } });
    await flush();
    first.unmount();

    // Only the memory can say it now.
    liveTypes = {};
    liveTiers = {};
    commits = [];
    render(shell());
    expect(commits[0]).toBe('subagent|unknown');
    expect(kindOf('t-alpha')).toBe('subagent');
  });

  it('lets a row that says otherwise win over the store, for a closed tab reissued its id', async () => {
    render(shell());
    await flush();
    // A store still holding a row from before the id was reissued.
    await emitLive({ types: { 'sub-alpha': 'sub_agent' } });
    await flush();
    await emitList([
      { id: 'parent', name: 'Delegation', privacy_tier: 'private', session_type: 'user' },
      { id: 'sub-alpha', name: 'New chat', privacy_tier: 'public', session_type: 'user' },
    ]);
    await flush();
    expect(kindOf('t-alpha')).toBe('chat');
  });
});
