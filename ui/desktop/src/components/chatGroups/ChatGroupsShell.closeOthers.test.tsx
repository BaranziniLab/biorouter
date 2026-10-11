import { render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * V2-R6-3: a tab's menu offers "Close tab" and "Close other tabs". The strip
 * only calls what it is given, so this pins the shell's half: each strip gets a
 * way to close every other tab in ITS group, and that way is the reducer's one
 * `closeOtherTabs` action (not a `closeTab` per tab, which would mount each
 * successor on the way). The reducer's half is in chatGroupsReducer.test.ts and
 * the menu's in ChatTabStrip.contextMenu.test.tsx.
 */

const dispatch = vi.hoisted(() => vi.fn());

type Tab = { tabId: string; sessionId: string; title: string; userSetName: boolean };
const strips: Array<{
  groupId: string;
  onClose: (tabId: string) => void;
  onCloseOthers?: (tabId: string) => void;
}> = [];

vi.mock('../BaseChat', () => ({
  default: (props: { renderSessionTitle?: () => React.ReactNode }) => (
    <div data-testid="basechat">{props.renderSessionTitle?.()}</div>
  ),
}));

vi.mock('./ChatTabStrip', () => ({
  ChatTabStrip: (props: {
    groupId: string;
    onClose: (tabId: string) => void;
    onCloseOthers?: (tabId: string) => void;
  }) => {
    strips.push({
      groupId: props.groupId,
      onClose: props.onClose,
      onCloseOthers: props.onCloseOthers,
    });
    return <div data-testid={`strip-${props.groupId}`} />;
  },
}));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => null,
  subscribeSessionList: () => () => {},
  preloadSessionList: () => {},
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
          tabs: ['t1', 't2', 't3'].map(
            (id): Tab => ({ tabId: id, sessionId: `s-${id}`, title: id, userSetName: false })
          ),
        },
      },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

describe('ChatGroupsShell: the strip’s close menu items reach the reducer', () => {
  beforeEach(() => {
    dispatch.mockClear();
    strips.length = 0;
  });

  it('hands the strip a Close other tabs that dispatches the one closeOtherTabs action', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const strip = strips[strips.length - 1];
    expect(strip.onCloseOthers).toBeTypeOf('function');

    strip.onCloseOthers!('t2');

    expect(dispatch).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenCalledWith({ type: 'closeOtherTabs', tabId: 't2' });
  });

  it('still closes one tab with closeTab', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    strips[strips.length - 1].onClose('t3');
    expect(dispatch).toHaveBeenCalledTimes(1);
    expect(dispatch).toHaveBeenCalledWith({ type: 'closeTab', tabId: 't3' });
  });
});
