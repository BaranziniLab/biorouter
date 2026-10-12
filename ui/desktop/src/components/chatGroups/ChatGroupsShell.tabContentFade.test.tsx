import { fireEvent, render, screen } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * Spec 3.16, "Tab content": after a tab opens or switches, the transcript fades
 * in once over `--dur-fast`. The pane carries `data-tab-content="switched"` in
 * the same commit as the new tab's chat, never on its first mount, and drops it
 * when the transcript's fade ends. jsdom runs no CSS, so the fade rule itself
 * is read at the source.
 */

let tabs: Array<{ tabId: string; sessionId: string; title: string; userSetName: boolean }> = [];
let activeTabId: string | null = null;

vi.mock('../BaseChat', () => ({
  default: (props: Record<string, unknown>) => (
    <div data-testid="basechat" data-tab={props.terminalKey as string}>
      <div data-testid="transcript" data-preview-transcript="" />
    </div>
  ),
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
      activeGroupId: 'g1',
      layout: { kind: 'leaf', groupId: 'g1' },
      groups: { g1: { id: 'g1', activeTabId, tabs } },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

const pane = () => screen.getByTestId('basechat').parentElement as HTMLElement;

describe('ChatGroupsShell: the tab content fade', () => {
  beforeEach(() => {
    tabs = [
      { tabId: 't1', sessionId: 's1', title: 'One', userSetName: false },
      { tabId: 't2', sessionId: 's2', title: 'Two', userSetName: false },
    ];
    activeTabId = 't1';
  });

  it('a pane that mounts with its tab draws at rest', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(pane()).not.toHaveAttribute('data-tab-content');
  });

  it('a tab switch marks the new chat in the same commit, and the fade ending clears it', () => {
    const view = render(<ChatGroupsShell onChatChange={() => {}} />);
    activeTabId = 't2';
    view.rerender(<ChatGroupsShell onChatChange={() => {}} />);

    expect(screen.getByTestId('basechat')).toHaveAttribute('data-tab', 't2');
    expect(pane()).toHaveAttribute('data-tab-content', 'switched');

    // Another element's animation ending does not clear it…
    fireEvent.animationEnd(screen.getByTestId('basechat'));
    expect(pane()).toHaveAttribute('data-tab-content', 'switched');
    // …the transcript's own fade does.
    fireEvent.animationEnd(screen.getByTestId('transcript'));
    expect(pane()).not.toHaveAttribute('data-tab-content');
  });

  it('fades the transcript in once, opacity only, on --dur-fast', () => {
    const css = readFileSync(join(__dirname, 'chat-tabs.css'), 'utf8');
    const rule = css.match(
      /\[data-tab-content='switched'\] \[data-preview-transcript\]\s*\{([^}]*)\}/
    );
    expect(rule, 'no tab content fade rule').not.toBeNull();
    expect(rule![1]).toMatch(/animation:\s*br-fade-in var\(--dur-fast\) var\(--ease-out\)/);
  });
});
