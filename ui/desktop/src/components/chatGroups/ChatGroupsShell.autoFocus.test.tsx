import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

/**
 * In a split, only the focused pane's composer takes the caret as it mounts.
 *
 * Every pane mounts at once when /pair is rebuilt, and a focus inside a pane
 * makes it the focused one (`ChatGroupPane`'s `onFocusCapture`). With every
 * composer focusing itself, the last pane won: measured in the dev app, New
 * chat resumed the draft showing in the left pane and the right pane's chat
 * ended up focused, with the caret. See `ChatInput.autoFocus.test.tsx` for the
 * composer's half.
 */

const chatProps = new Map<string, Record<string, unknown>>();

vi.mock('../BaseChat', () => ({
  default: (props: Record<string, unknown>) => {
    chatProps.set(String(props.terminalKey), props);
    return <div data-testid="basechat" />;
  },
}));

vi.mock('./ChatTabStrip', () => ({ ChatTabStrip: () => <div data-testid="strip" /> }));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => null,
  subscribeSessionList: () => () => {},
  preloadSessionList: () => {},
}));

const tab = (tabId: string, sessionId: string) => ({
  tabId,
  sessionId,
  title: tabId,
  userSetName: false,
});

vi.mock('../../contexts/ChatGroupsContext', () => ({
  useChatGroups: () => ({
    dispatch: vi.fn(),
    runningSessionIds: [],
    tabAnnotations: {},
    state: {
      activeGroupId: 'left',
      layout: {
        kind: 'branch',
        dir: 'row',
        sizes: [0.5, 0.5],
        children: [
          { kind: 'leaf', groupId: 'left' },
          { kind: 'leaf', groupId: 'right' },
        ],
      },
      groups: {
        left: { groupId: 'left', activeTabId: 'tab-draft', tabs: [tab('tab-draft', '')] },
        right: {
          groupId: 'right',
          activeTabId: 'tab-chat',
          tabs: [tab('tab-chat', 's-chat'), tab('tab-behind', '')],
        },
      },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

describe('ChatGroupsShell — which pane’s composer takes the caret on mount', () => {
  it('the focused pane’s, and no other', () => {
    chatProps.clear();
    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(chatProps.get('tab-draft')?.autoFocusComposer).toBe(true);
    expect(chatProps.get('tab-chat')?.autoFocusComposer).toBe(false);
  });
});
