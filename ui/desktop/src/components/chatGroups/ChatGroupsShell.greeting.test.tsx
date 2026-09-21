import { Greeting, retainTabGreetings } from '../common/Greeting';
import type { GroupLayout } from './chatGroupsTypes';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi, beforeEach } from 'vitest';

/**
 * The placeholder pane must not draw a greeting it is about to throw away.
 *
 * A tabless `/pair` is never a resting state: `useEmptyPairRedirect` either
 * finds cargo in flight (a resume id, a parked launcher message, a workflow
 * deeplink, a session mid-create) or navigates to Home. So the pane rendered
 * while `activeTab` is undefined exists only until a real tab lands.
 *
 * `ChatGroupsShell` keys `BaseChat` on the tab id, so that landing unmounts the
 * placeholder and mounts a fresh `<Greeting>` — which draws a NEW random
 * sentence, by design. The unroll takes about a second and the awaited
 * `createSession` on the new-window paths takes about as long, so the
 * placeholder had time to finish before being discarded: the user saw a heading
 * arrive, vanish, and a different heading arrive after it. That is the reported
 * "flash then re-animate", reached by a second route. The first-frame fix in
 * `use-text-animator` cannot touch it, because this is a whole extra mount
 * rather than a flash inside one.
 *
 * ⚠ It suppresses the GREETING and not the empty state. `suppressEmptyState`
 * would take the composer with it, and the composer is the one thing that must
 * survive here in case the tab that was coming never arrives.
 */

let tabs: Array<{ tabId: string; sessionId: string; title: string; userSetName: boolean }> = [];
let activeTabId: string | null = null;
let layout: GroupLayout = { kind: 'leaf', groupId: 'g1' };
let tabGroupId = 'g1';
const animated = vi.fn();
vi.mock('../../hooks/use-text-animator', () => ({
  useTextAnimator: ({ enabled }: { enabled: boolean }) => {
    animated(enabled);
    return { current: null };
  },
}));
let lastChatProps: Record<string, unknown> = {};

vi.mock('../BaseChat', () => ({
  default: (props: Record<string, unknown>) => {
    lastChatProps = props;
    return (
      <div data-testid="basechat">
        {!props.suppressGreeting && <Greeting tabId={props.terminalKey as string} />}
      </div>
    );
  },
}));

vi.mock('./ChatTabStrip', () => ({ ChatTabStrip: () => <div data-testid="strip" /> }));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => null,
  subscribeSessionList: () => () => {},
  preloadSessionList: () => {},
}));

vi.mock('../../contexts/ChatGroupsContext', () => ({
  useChatGroups: () => ({
    dispatch: vi.fn(),
    state: {
      activeGroupId: tabGroupId,
      layout,
      groups: {
        g1: {
          id: 'g1',
          activeTabId: tabGroupId === 'g1' ? activeTabId : null,
          tabs: tabGroupId === 'g1' ? tabs : [],
        },
        g2: {
          id: 'g2',
          activeTabId: tabGroupId === 'g2' ? activeTabId : null,
          tabs: tabGroupId === 'g2' ? tabs : [],
        },
      },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

describe('ChatGroupsShell — the placeholder pane', () => {
  beforeEach(() => {
    lastChatProps = {};
    layout = { kind: 'leaf', groupId: 'g1' };
    tabGroupId = 'g1';
    retainTabGreetings([]);
    animated.mockClear();
  });

  it('suppresses the greeting while there is no tab, because that pane is replaced and not filled', () => {
    tabs = [];
    activeTabId = null;
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastChatProps.suppressGreeting).toBe(true);
    // ⚠ And ONLY the greeting. Taking the empty state would take the composer.
    expect(lastChatProps.suppressEmptyState).toBe(false);
  });

  it('draws the greeting for a real tab, which is the arrival it belongs to', () => {
    tabs = [{ tabId: 't1', sessionId: 'sess-1', title: 'Chat', userSetName: false }];
    activeTabId = 't1';
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastChatProps.suppressGreeting).toBe(false);
    expect(lastChatProps.suppressEmptyState).toBe(false);
  });
  it('keeps the greeting across a split, a tab move, and merging the panes', () => {
    tabs = [{ tabId: 't1', sessionId: '', title: 'New chat', userSetName: false }];
    activeTabId = 't1';
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    const message = screen.getByRole('heading').textContent;
    expect(animated).toHaveBeenLastCalledWith(true);
    animated.mockClear();
    layout = {
      kind: 'branch',
      dir: 'row',
      sizes: [0.5, 0.5],
      children: [
        { kind: 'leaf', groupId: 'g1' },
        { kind: 'leaf', groupId: 'g2' },
      ],
    };
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(screen.getByRole('heading').textContent).toBe(message);
    expect(animated).not.toHaveBeenCalledWith(true);
    tabGroupId = 'g2';
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(screen.getByRole('heading').textContent).toBe(message);
    expect(animated).not.toHaveBeenCalledWith(true);
    layout = { kind: 'leaf', groupId: 'g2' };
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(screen.getByRole('heading').textContent).toBe(message);
    expect(animated).not.toHaveBeenCalledWith(true);
  });
});
