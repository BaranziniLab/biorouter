import { render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { TabStripMemory } from './tabStripMemory';

/**
 * Vision V1-H1: a new tab never grew in, because the strip that shows it is a
 * NEW strip. The pane keys its chat on the active tab and the strip renders in
 * that chat's header, so "+" remounts it. The strip compares a tab list against
 * a TabStripMemory, and this pins the shell's half: the record a group's strip
 * gets survives the remount, belongs to that group alone, and goes when the
 * group leaves the layout. The strip's own half is in ChatTabStrip.flip.test.
 */

type Tab = { tabId: string; sessionId: string; title: string; userSetName: boolean };
let groups: Record<string, { id: string; activeTabId: string | null; tabs: Tab[] }> = {};
let layout: unknown = null;

vi.mock('../BaseChat', () => ({
  default: (props: { renderSessionTitle?: () => React.ReactNode }) => (
    <div data-testid="basechat">{props.renderSessionTitle?.()}</div>
  ),
}));

const memories: Array<{ groupId: string; memory: TabStripMemory | undefined }> = [];
vi.mock('./ChatTabStrip', () => ({
  ChatTabStrip: (props: { groupId: string; memory?: TabStripMemory }) => {
    memories.push({ groupId: props.groupId, memory: props.memory });
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
    dispatch: vi.fn(),
    state: { activeGroupId: 'g1', layout, groups },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

const tab = (id: string): Tab => ({
  tabId: id,
  sessionId: `s-${id}`,
  title: id,
  userSetName: false,
});
const lastMemory = (groupId: string) =>
  [...memories].reverse().find((entry) => entry.groupId === groupId)?.memory;

describe('ChatGroupsShell: each strip keeps its memory across a remount', () => {
  beforeEach(() => {
    memories.length = 0;
    groups = { g1: { id: 'g1', activeTabId: 't1', tabs: [tab('t1')] } };
    layout = { kind: 'leaf', groupId: 'g1' };
  });

  it('hands the strip the same record before and after "+" remounts it', () => {
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    const before = lastMemory('g1');
    expect(before).toBeDefined();

    groups = { g1: { id: 'g1', activeTabId: 't2', tabs: [tab('t1'), tab('t2')] } };
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastMemory('g1')).toBe(before);
  });

  it('gives each group its own record, and forgets a group that leaves the layout', () => {
    groups = {
      g1: { id: 'g1', activeTabId: 't1', tabs: [tab('t1')] },
      g2: { id: 'g2', activeTabId: 't2', tabs: [tab('t2')] },
    };
    layout = {
      kind: 'branch',
      dir: 'row',
      sizes: [50, 50],
      children: [
        { kind: 'leaf', groupId: 'g1' },
        { kind: 'leaf', groupId: 'g2' },
      ],
    };
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    const g2First = lastMemory('g2');
    expect(lastMemory('g1')).toBeDefined();
    expect(g2First).toBeDefined();
    expect(lastMemory('g1')).not.toBe(g2First);

    // g2 closes, then a group with the same id comes back: it starts afresh.
    groups = { g1: groups.g1 };
    layout = { kind: 'leaf', groupId: 'g1' };
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    groups = { ...groups, g2: { id: 'g2', activeTabId: 't2', tabs: [tab('t2')] } };
    layout = {
      kind: 'branch',
      dir: 'row',
      sizes: [50, 50],
      children: [
        { kind: 'leaf', groupId: 'g1' },
        { kind: 'leaf', groupId: 'g2' },
      ],
    };
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(lastMemory('g2')).toBeDefined();
    expect(lastMemory('g2')).not.toBe(g2First);
  });
});
