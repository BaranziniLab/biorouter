import { render } from '@testing-library/react';
import { describe, expect, it, vi, beforeEach } from 'vitest';

/**
 * The tab strip's privacy dots must not depend on some OTHER screen having
 * fetched the session list first (issue #56, R10).
 *
 * `useSessionPrivacyTiers` READS the shared session-list cache. The question
 * this file pins is who FILLS it. The hook originally documented that
 * "AppSidebar already calls preloadSessionList() at module scope, so in the
 * running app the cache is warm before any strip renders" — which is not what
 * that file does. `preloadSessionList` lives inside `preloadHome()`, wired to
 * `onFocus`/`onPointerEnter` on the Home nav entry, so it only runs if the user
 * points at Home. What actually warmed the cache on a normal launch was the Hub
 * index route mounting SessionInsights, which calls `refreshSessionList()` — an
 * incidental side effect of an unrelated screen.
 *
 * A window that opens straight onto a chat, never touching Home, therefore drew
 * every tab unmarked — including private ones. That fails silent rather than
 * asserting Public, but "silent" is still the wrong answer for the surface the
 * user is working in.
 *
 * So the strip warms the cache itself. `preloadSessionList()` is idempotent
 * (it returns early when the cache is non-null) and swallows its own errors,
 * so calling it here costs one fetch on a cold start and nothing thereafter.
 */

const dispatch = vi.fn();
const preloadSessionList = vi.fn();
let cachedList: Array<{ id: string; privacy_tier?: string }> | null = null;
let liveTiers: Record<string, string> = {};

// The strip is not a child of the shell — it reaches the DOM through BaseChat's
// `renderSessionTitle` render prop, so a BaseChat stub that ignores its props
// renders no strip at all and every assertion below would read `undefined`
// against a component that is in fact wired correctly.
vi.mock('../BaseChat', () => ({
  default: (props: { renderSessionTitle?: () => React.ReactNode }) => (
    <div data-testid="basechat">{props.renderSessionTitle?.()}</div>
  ),
}));

// Capture what the shell hands the strip without rendering the real one.
let lastStripProps: Record<string, unknown> = {};
vi.mock('./ChatTabStrip', () => ({
  ChatTabStrip: (props: Record<string, unknown>) => {
    lastStripProps = props;
    return <div data-testid="strip" />;
  },
}));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => cachedList,
  subscribeSessionList: () => () => {},
  preloadSessionList: () => preloadSessionList(),
}));

// The LIVE half of the tier map: what the chat stores in this window hold right
// now. Stubbed here so this file stays about the MERGE — the registry's own
// side, a controller's row becoming an entry in that map, is pinned in
// `hooks/chatStreamStore.binding.test.tsx`.
vi.mock('../../hooks/chatStreamStore', () => ({
  useLiveSessionTiers: () => liveTiers,
}));

vi.mock('../../contexts/ChatGroupsContext', () => ({
  useChatGroups: () => ({
    dispatch,
    state: {
      activeGroupId: 'g1',
      layout: { kind: 'leaf', groupId: 'g1' },
      groups: {
        g1: {
          id: 'g1',
          activeTabId: 't1',
          tabs: [{ tabId: 't1', sessionId: 'sess-1', title: 'Chat', userSetName: false }],
        },
      },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

describe('ChatGroupsShell — the strip warms the list it reads', () => {
  beforeEach(() => {
    preloadSessionList.mockClear();
    cachedList = null;
    liveTiers = {};
    lastStripProps = {};
  });

  it('asks the cache to populate itself on mount, rather than assuming another screen did', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(preloadSessionList).toHaveBeenCalled();
  });

  it('passes the tiers it finds in the cache down to the strip', () => {
    cachedList = [
      { id: 'sess-1', privacy_tier: 'private' },
      { id: 'sess-2', privacy_tier: 'public' },
      { id: 'sess-3' },
    ];
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastStripProps.privacyTiers).toEqual({ 'sess-1': 'private', 'sess-2': 'public' });
  });

  it('marks nothing when the cache is empty, rather than defaulting to public', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastStripProps.privacyTiers).toEqual({});
  });
});

/**
 * Finding M8, measured on merged main 2026-09-10: new chat tab, one turn on
 * `versa_azure`, and 52.9 s later sqlite read `privacy_tier=private`, the model
 * chip said "Private chat", the sidebar row said `data-privacy="private"` — and
 * the ACTIVE TAB's own glyph still said `data-privacy="public"`.
 *
 * The cause is structural rather than a race: a chat CREATED in this window is
 * not in the session list at all. `GET /sessions` INNER JOINs `messages`, so a
 * row that has recorded none is not listable, and `refreshSessionBinding`
 * patches only entries the cache already holds. Waiting longer never fixed it.
 *
 * The store, meanwhile, had the answer from the reply stream's first frames —
 * which is why the chip and the header pill were right. These tests pin that
 * the strip now reads the same source, and that the two sources combine in the
 * safe direction.
 */
describe('ChatGroupsShell — the tab dot follows the live store, not just the list', () => {
  it('marks a chat the store says is private but the list has never carried', () => {
    cachedList = [{ id: 'someone-else', privacy_tier: 'public' }];
    liveTiers = { 'sess-1': 'private' };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastStripProps.privacyTiers).toEqual({
      'someone-else': 'public',
      'sess-1': 'private',
    });
  });

  it('marks it even when the list has not been fetched at all', () => {
    cachedList = null;
    liveTiers = { 'sess-1': 'private' };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastStripProps.privacyTiers).toEqual({ 'sess-1': 'private' });
  });

  it('lets the live store raise a chat the cache still calls public', () => {
    // The ratchet fired during this turn; the cached list was fetched before it.
    cachedList = [{ id: 'sess-1', privacy_tier: 'public' }];
    liveTiers = { 'sess-1': 'private' };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastStripProps.privacyTiers).toEqual({ 'sess-1': 'private' });
  });

  it('never lets a store that has not caught up LOWER a private the list knows', () => {
    // The safe-direction invariant, stated as its own gate. The tier is a
    // permanent ratchet server-side, so `private` from either source is a fact
    // that still holds — and a surface may only render private-or-unmarked,
    // never public over a source that has seen private.
    cachedList = [{ id: 'sess-1', privacy_tier: 'private' }];
    liveTiers = { 'sess-1': 'public' };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastStripProps.privacyTiers).toEqual({ 'sess-1': 'private' });
  });

  it('still marks nothing when neither source has an opinion', () => {
    cachedList = [];
    liveTiers = {};
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastStripProps.privacyTiers).toEqual({});
  });
});
