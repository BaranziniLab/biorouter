import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { GroupLayout } from './chatGroupsTypes';

/**
 * Compact panes (rung 5 of the yield ladder), wired into the shell.
 *
 * A pane in a split that is too small for its chat shows a placeholder and keeps its chat MOUNTED
 * behind it. jsdom lays nothing out, so the pane's box is stubbed on `[data-chat-group-id]` (and the
 * tree and the placeholder's own root), and the ResizeObservers are fakes the test calls. Every rect
 * jsdom reports is 0 otherwise, which `paneFit` reads as unmeasured, so every other shell suite
 * keeps seeing full panes.
 */

const mocks = vi.hoisted(() => ({
  mounts: new Map<string, number>(),
  dispatch: (() => {}) as (action: unknown) => void,
  running: [] as Array<{ sessionId: string; chatState: string; completedAt?: number }>,
  state: null as unknown,
}));

vi.mock('../BaseChat', async () => {
  const React = await import('react');
  return {
    default: function BaseChatStub(props: {
      terminalKey?: string;
      sessionId?: string;
      renderSessionTitle?: () => React.ReactNode;
    }) {
      const key = props.terminalKey ?? '';
      React.useEffect(() => {
        mocks.mounts.set(key, (mocks.mounts.get(key) ?? 0) + 1);
      }, [key]);
      return (
        <div data-testid="basechat" data-tab={key}>
          <textarea aria-label={`Message ${key}`} className="br-composer-input" />
          <button type="button">Other control {key}</button>
          {props.renderSessionTitle?.()}
        </div>
      );
    },
  };
});

// The strip itself is not under test; the ▾ menu the compact band borrows from it is real.
vi.mock('./ChatTabStrip', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./ChatTabStrip')>()),
  ChatTabStrip: () => <div data-testid="strip" />,
}));

vi.mock('../../hooks/chatStreamStore', () => ({
  useLiveSessionTiers: () => ({}),
  useLiveSessionTypes: () => ({}),
  useRunningChats: () => mocks.running,
}));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => null,
  subscribeSessionList: () => () => {},
  preloadSessionList: () => {},
}));

vi.mock('../../contexts/ChatGroupsContext', () => ({
  useChatGroups: () => ({
    dispatch: (action: unknown) => mocks.dispatch(action),
    runningSessionIds: mocks.running.map((entry) => entry.sessionId),
    tabAnnotations: {},
    state: mocks.state,
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

const TWO_PANES: GroupLayout = {
  kind: 'branch',
  dir: 'row',
  children: [
    { kind: 'leaf', groupId: 'g1' },
    { kind: 'leaf', groupId: 'g2' },
  ],
  sizes: [2 / 3, 1 / 3],
};

function stateWith(layout: GroupLayout, g2Tabs = 1) {
  return {
    version: 1,
    seq: 9,
    activeGroupId: 'g1',
    layout,
    groups: {
      g1: {
        groupId: 'g1',
        activeTabId: 't1',
        tabs: [{ tabId: 't1', sessionId: 'sess-1', title: 'First chat', userSetName: false }],
      },
      g2: {
        groupId: 'g2',
        activeTabId: 't2',
        tabs: [
          { tabId: 't2', sessionId: 'sess-2', title: 'Second chat', userSetName: false },
          ...(g2Tabs > 1
            ? [{ tabId: 't3', sessionId: 'sess-3', title: 'Third chat', userSetName: false }]
            : []),
        ],
      },
    },
  };
}

/** The boxes jsdom does not compute: each pane, the tree, and the placeholder (its pane's box). */
let boxes: Record<string, { width: number; height: number }> = {};
const originalRect = HTMLElement.prototype.getBoundingClientRect;
let resizeCallbacks: Array<() => void> = [];
const originalResizeObserver = globalThis.ResizeObserver;

function rect(width: number, height: number): DOMRect {
  return {
    x: 0,
    y: 0,
    left: 0,
    top: 0,
    width,
    height,
    right: width,
    bottom: height,
    toJSON: () => ({}),
  } as DOMRect;
}

beforeEach(() => {
  mocks.mounts.clear();
  mocks.running = [];
  mocks.dispatch = vi.fn();
  mocks.state = stateWith(TWO_PANES);
  boxes = {
    g1: { width: 600, height: 600 },
    g2: { width: 300, height: 600 },
    tree: { width: 901, height: 600 },
  };
  HTMLElement.prototype.getBoundingClientRect = function (this: HTMLElement) {
    const group = this.getAttribute('data-chat-group-id');
    if (group && boxes[group]) return rect(boxes[group].width, boxes[group].height);
    if (this.hasAttribute('data-chat-groups-tree'))
      return rect(boxes.tree.width, boxes.tree.height);
    if (this.classList.contains('br-compact-pane')) {
      const pane = this.closest('[data-chat-group-id]')?.getAttribute('data-chat-group-id');
      if (pane && boxes[pane]) return rect(boxes[pane].width, boxes[pane].height);
    }
    return originalRect.call(this);
  };
  resizeCallbacks = [];
  class FakeResizeObserver {
    constructor(callback: () => void) {
      resizeCallbacks.push(callback);
    }
    observe() {}
    disconnect() {}
    unobserve() {}
  }
  globalThis.ResizeObserver = FakeResizeObserver as unknown as typeof ResizeObserver;
});

afterEach(() => {
  HTMLElement.prototype.getBoundingClientRect = originalRect;
  globalThis.ResizeObserver = originalResizeObserver;
});

function resize(next: Record<string, { width: number; height: number }>) {
  boxes = { ...boxes, ...next };
  act(() => {
    for (const callback of [...resizeCallbacks]) callback();
  });
}

const pane = (groupId: string) =>
  document.querySelector<HTMLElement>(`[data-chat-group-id="${groupId}"]`)!;
const content = (groupId: string) =>
  pane(groupId).querySelector<HTMLElement>('[data-testid="chat-pane-content"]')!;
const action = (groupId: string) =>
  pane(groupId).querySelector<HTMLElement>('[data-compact-pane-action]');

describe('a compact pane in the shell', () => {
  it('replaces only the pane under the floor, and makes its chat inert', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(pane('g1').getAttribute('data-pane-fit')).toBe('full');
    expect(pane('g2').getAttribute('data-pane-fit')).toBe('compact');
    expect(pane('g2').getAttribute('data-pane-shortfall')).toBe('width');
    expect(document.querySelectorAll('.br-compact-pane')).toHaveLength(1);
    expect(pane('g2').querySelector('.br-compact-pane')).not.toBeNull();
    expect(content('g2').hasAttribute('inert')).toBe(true);
    expect(content('g1').hasAttribute('inert')).toBe(false);
    // The chat is still there, behind the placeholder.
    expect(content('g2').querySelector('[data-testid="basechat"]')).not.toBeNull();
  });

  it('mounts each pane’s chat once across full, compact and full', () => {
    boxes.g2 = { width: 450, height: 600 };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(pane('g2').getAttribute('data-pane-fit')).toBe('full');
    resize({ g2: { width: 300, height: 600 } });
    expect(pane('g2').getAttribute('data-pane-fit')).toBe('compact');
    resize({ g2: { width: 371, height: 600 } });
    expect(pane('g2').getAttribute('data-pane-fit')).toBe('compact');
    resize({ g2: { width: 372, height: 600 } });
    expect(pane('g2').getAttribute('data-pane-fit')).toBe('full');
    expect(mocks.mounts.get('t1')).toBe(1);
    expect(mocks.mounts.get('t2')).toBe(1);
    // The content wrapper is the pane's first child in both states.
    expect(pane('g2').firstElementChild).toBe(content('g2'));
  });

  it('moves focus from the chat to the action on compaction, and back on restore', () => {
    boxes.g2 = { width: 450, height: 600 };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const draft = screen.getByLabelText('Message t2');
    draft.focus();
    expect(document.activeElement).toBe(draft);
    resize({ g2: { width: 300, height: 600 } });
    expect(document.activeElement).toBe(action('g2'));
    resize({ g2: { width: 400, height: 600 } });
    expect(document.activeElement).toBe(draft);
  });

  it('never takes focus from a pane that does not hold it', () => {
    boxes.g2 = { width: 450, height: 600 };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const draft = screen.getByLabelText('Message t1');
    draft.focus();
    resize({ g2: { width: 300, height: 600 } });
    expect(document.activeElement).toBe(draft);
    resize({ g2: { width: 400, height: 600 } });
    expect(document.activeElement).toBe(draft);
  });

  it('never replaces a lone pane, however small', () => {
    mocks.state = stateWith({ kind: 'leaf', groupId: 'g1' });
    boxes.g1 = { width: 200, height: 200 };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    resize({ g1: { width: 150, height: 150 } });
    expect(pane('g1').getAttribute('data-pane-fit')).toBe('full');
    expect(document.querySelector('.br-compact-pane')).toBeNull();
    expect(content('g1').hasAttribute('inert')).toBe(false);
  });

  it('resets a scroll the hidden chat pushed onto the pane', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const root = pane('g2');
    root.scrollLeft = 40;
    root.scrollTop = 20;
    fireEvent.scroll(root);
    expect(root.scrollLeft).toBe(0);
    expect(root.scrollTop).toBe(0);
  });
});
