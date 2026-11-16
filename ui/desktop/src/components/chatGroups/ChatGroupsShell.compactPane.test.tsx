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
import { compactPaneCopy } from './copy';
import { ChatState } from '../../types/chatState';
import { DICTATION_LABELS } from '../dictation/copy';
import { setDictationSnapshotForTests } from '../dictation/dictationSession';
import { existingChatComposerDraftKey } from '../../utils/composerDrafts';

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

  it('gives focus to the composer when the element that held it refuses it on return', () => {
    // In the app the terminal's input refused it: its dock transitions `visibility`, so in the
    // frame of the return it still computed hidden. Whatever the cause, focus must not fall to
    // the page.
    boxes.g2 = { width: 450, height: 600 };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const control = screen.getByRole('button', { name: 'Other control t2' });
    control.focus();
    resize({ g2: { width: 300, height: 600 } });
    expect(document.activeElement).toBe(action('g2'));
    control.focus = () => {};
    resize({ g2: { width: 400, height: 600 } });
    expect(document.activeElement).toBe(screen.getByLabelText('Message t2'));
  });

  it('carries focus to the band action when the pane gets too short for the card', () => {
    boxes.g2 = { width: 450, height: 600 };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    screen.getByLabelText('Message t2').focus();
    resize({ g2: { width: 300, height: 600 } });
    expect(document.activeElement?.getAttribute('data-testid')).toBe('compact-pane-action');
    // 300 by 100: band density, so the action moves from the body to the band.
    resize({ g2: { width: 300, height: 100 } });
    expect(pane('g2').querySelector('.br-compact-pane')!.getAttribute('data-density')).toBe('band');
    expect(document.activeElement?.getAttribute('data-testid')).toBe('compact-pane-band-action');
    // And back to the body button when there is room again.
    resize({ g2: { width: 300, height: 600 } });
    expect(document.activeElement?.getAttribute('data-testid')).toBe('compact-pane-action');
  });

  it('puts focus on the placeholder itself when its band has no room for an action', () => {
    // g1 is the first leaf, so its band keeps the 172px titlebar reserve (jsdom's 1024px window
    // reads as a compact sidebar on macOS). At 150 by 100 the pane is in band density and the band
    // has no room left for the action or the ▾, so the placeholder has nothing focusable inside it.
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const draft = screen.getByLabelText('Message t1');
    draft.focus();
    resize({ g1: { width: 150, height: 100 } });
    const placeholder = pane('g1').querySelector<HTMLElement>('.br-compact-pane')!;
    expect(placeholder.getAttribute('data-density')).toBe('band');
    expect(action('g1')).toBeNull();
    expect(document.activeElement, 'focus is not left inside the inert chat').toBe(placeholder);
    // Focused, it says what the missing action would: the state and how the chat comes back
    // (short on both axes in a row with no column to grow, so the help names the window and Merge).
    const described = placeholder.getAttribute('aria-describedby');
    expect(described).not.toBeNull();
    expect(document.getElementById(described!)!.textContent).toBe(
      `Chat, privacy not yet known. ${compactPaneCopy.shortfall.both}. ${compactPaneCopy.help('both', false)}`
    );
    resize({ g1: { width: 600, height: 600 } });
    expect(document.activeElement, 'and it comes back to the draft').toBe(draft);
  });

  it('describes the placeholder itself only while it has no action to describe', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const placeholder = pane('g2').querySelector<HTMLElement>('.br-compact-pane')!;
    expect(action('g2')).not.toBeNull();
    // The action carries the description; the group repeating it would read it twice.
    expect(placeholder.hasAttribute('aria-describedby')).toBe(false);
  });

  it('keeps focus in the placeholder when the band drops the ▾ that held it', () => {
    mocks.state = stateWith(TWO_PANES, 2);
    boxes.g2 = { width: 100, height: 100 };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const menu = () =>
      pane('g2').querySelector<HTMLElement>('[data-testid="chat-tab-overflow-trigger"]');
    const placeholder = pane('g2').querySelector<HTMLElement>('.br-compact-pane')!;
    // 100 by 100: band density with room for the action and the ▾.
    expect(placeholder.getAttribute('data-density')).toBe('band');
    menu()!.focus();
    expect(document.activeElement).toBe(menu());
    // 50 by 100: the band keeps the action but no longer the ▾, so focus goes to the action.
    resize({ g2: { width: 50, height: 100 } });
    expect(menu()).toBeNull();
    expect(document.activeElement?.getAttribute('data-testid')).toBe('compact-pane-band-action');
    // The ▾ coming back does not take focus from the action.
    resize({ g2: { width: 100, height: 100 } });
    expect(document.activeElement?.getAttribute('data-testid')).toBe('compact-pane-band-action');
    // 30 by 100: neither fits, so the placeholder itself holds it.
    menu()!.focus();
    resize({ g2: { width: 30, height: 100 } });
    expect(menu()).toBeNull();
    expect(action('g2')).toBeNull();
    expect(document.activeElement).toBe(placeholder);
  });

  it('carries focus between the action and the placeholder as the band loses and regains room', () => {
    boxes.g2 = { width: 450, height: 600 };
    render(<ChatGroupsShell onChatChange={() => {}} />);
    screen.getByLabelText('Message t2').focus();
    // 120 by 150: glyph density, the action in the body.
    resize({ g2: { width: 120, height: 150 } });
    expect(document.activeElement?.getAttribute('data-testid')).toBe('compact-pane-action');
    // 30 by 100: band density with under 28px of band, so no action anywhere.
    resize({ g2: { width: 30, height: 100 } });
    const placeholder = pane('g2').querySelector<HTMLElement>('.br-compact-pane')!;
    expect(action('g2')).toBeNull();
    expect(document.activeElement).toBe(placeholder);
    // Room again: focus goes to the action, not left on the placeholder.
    resize({ g2: { width: 120, height: 150 } });
    expect(document.activeElement?.getAttribute('data-testid')).toBe('compact-pane-action');
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

describe('what a compact pane says', () => {
  it('names the chat it holds', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const placeholder = pane('g2').querySelector('.br-compact-pane')!;
    expect(placeholder.getAttribute('aria-label')).toBe('Second chat');
    expect(placeholder.querySelector('.br-compact-pane__title')!.textContent).toBe('Second chat');
  });

  it('says Waiting for you for a chat waiting on the person, and announces it once', () => {
    mocks.running = [{ sessionId: 'sess-2', chatState: ChatState.Streaming }];
    const { rerender } = render(<ChatGroupsShell onChatChange={() => {}} />);
    const status = () => pane('g2').querySelector('[role="status"]')!;
    const line = () => pane('g2').querySelector('.br-compact-pane__state')!;
    expect(line().textContent).toBe(compactPaneCopy.running);
    // Silent on mount and while it runs.
    expect(status().textContent).toBe('');
    mocks.running = [{ sessionId: 'sess-2', chatState: ChatState.WaitingForUserInput }];
    rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(line().textContent).toBe(compactPaneCopy.waiting);
    expect(status().textContent).toBe(compactPaneCopy.announceWaiting('Second chat'));
    // A re-render while still waiting does not announce again: the text is unchanged.
    rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(status().textContent).toBe(compactPaneCopy.announceWaiting('Second chat'));
    // Leaving the waiting state clears it.
    mocks.running = [{ sessionId: 'sess-2', chatState: ChatState.Streaming }];
    rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(status().textContent).toBe('');
    expect(line().textContent).toBe(compactPaneCopy.running);
  });

  it('draws the running dot for any live entry, and nothing for a finished one', () => {
    mocks.running = [{ sessionId: 'sess-2', chatState: ChatState.Thinking }];
    const { rerender } = render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(pane('g2').querySelector('.br-compact-pane__dot')).not.toBeNull();
    mocks.running = [{ sessionId: 'sess-2', chatState: ChatState.Idle, completedAt: 5 }];
    rerender(<ChatGroupsShell onChatChange={() => {}} />);
    expect(pane('g2').querySelector('.br-compact-pane__dot')).toBeNull();
    expect(pane('g2').querySelector('.br-compact-pane__state')!.textContent).toBe(
      compactPaneCopy.shortfall.width
    );
  });

  it('says Listening… while the pane’s own composer is dictating', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    act(() => {
      setDictationSnapshotForTests({
        key: existingChatComposerDraftKey('t2', 'sess-2'),
        activity: 'recording',
      });
    });
    expect(pane('g2').querySelector('.br-compact-pane__state')!.textContent).toBe(
      DICTATION_LABELS.listening
    );
    act(() => {
      setDictationSnapshotForTests({ key: null, activity: 'idle' });
    });
    expect(pane('g2').querySelector('.br-compact-pane__state')!.textContent).toBe(
      compactPaneCopy.shortfall.width
    );
  });
});

describe('what a compact pane does', () => {
  it('Make wider makes the pane active, then resizes the branches the plan names', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const button = action('g2')!;
    expect(button.getAttribute('aria-label')).toBe('Make wider, Second chat');
    fireEvent.click(button);
    const calls = (mocks.dispatch as ReturnType<typeof vi.fn>).mock.calls.map((call) => call[0]);
    const relevant = calls.filter(
      (a: { type: string }) => a.type === 'setActiveGroup' || a.type === 'resizeBranch'
    );
    expect(relevant[relevant.length - 2]).toEqual({ type: 'setActiveGroup', groupId: 'g2' });
    const resize = relevant[relevant.length - 1] as {
      type: string;
      path: number[];
      sizes: number[];
    };
    expect(resize.type).toBe('resizeBranch');
    expect(resize.path).toEqual([]);
    // 901 wide, one splitter: g2 gets 496 of 900.
    expect(resize.sizes[1] * 900).toBeCloseTo(496, 6);
    expect(calls.some((a: { type: string }) => a.type === 'mergeAllGroups')).toBe(false);
  });

  it('offers Merge panes when the layout has no room, and merges into this pane', () => {
    // Two 200px panes in a 401px tree: even attempt 4 needs 376 + 64 + 1 = 441.
    boxes = {
      g1: { width: 200, height: 600 },
      g2: { width: 200, height: 600 },
      tree: { width: 401, height: 600 },
    };
    mocks.state = stateWith({ ...TWO_PANES, sizes: [0.5, 0.5] } as GroupLayout);
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const button = action('g2')!;
    expect(button.getAttribute('aria-label')).toBe('Merge panes, Second chat');
    (mocks.dispatch as ReturnType<typeof vi.fn>).mockClear();
    fireEvent.click(button);
    const calls = (mocks.dispatch as ReturnType<typeof vi.fn>).mock.calls.map((call) => call[0]);
    expect(calls).toEqual([{ type: 'setActiveGroup', groupId: 'g2' }, { type: 'mergeAllGroups' }]);
  });

  it('never merges on a press whose label said grow', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const button = action('g2')!;
    expect(button.getAttribute('aria-label')).toBe('Make wider, Second chat');
    // The tree shrinks before the press (to where even attempt 4, 376 + 64 + 1, cannot fit),
    // and no observer has seen it yet.
    boxes.tree = { width: 400, height: 600 };
    (mocks.dispatch as ReturnType<typeof vi.fn>).mockClear();
    fireEvent.click(button);
    const calls = (mocks.dispatch as ReturnType<typeof vi.fn>).mock.calls.map((call) => call[0]);
    expect(calls.some((a: { type: string }) => a.type === 'mergeAllGroups')).toBe(false);
    expect(calls.some((a: { type: string }) => a.type === 'resizeBranch')).toBe(false);
    // The label now says what a press would do.
    expect(action('g2')!.getAttribute('aria-label')).toBe('Merge panes, Second chat');
  });

  it('puts the tab menu in the band from two tabs, and choosing a tab activates it', () => {
    mocks.state = stateWith(TWO_PANES, 2);
    render(<ChatGroupsShell onChatChange={() => {}} />);
    const band = pane('g2').querySelector<HTMLElement>('.br-compact-pane__band')!;
    // The band moves the window by pointer gesture, never by a drag app-region.
    expect(band.getAttribute('style') ?? '').not.toMatch(/app-region:\s*drag/);
    const trigger = band.querySelector<HTMLElement>('[data-testid="chat-tab-overflow-trigger"]')!;
    expect(trigger).not.toBeNull();
    fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false });
    const item = document.querySelector<HTMLElement>('[data-testid="chat-tab-overflow-item-t3"]');
    expect(item).not.toBeNull();
    (mocks.dispatch as ReturnType<typeof vi.fn>).mockClear();
    fireEvent.click(item!);
    const calls = (mocks.dispatch as ReturnType<typeof vi.fn>).mock.calls.map((call) => call[0]);
    expect(calls).toContainEqual({ type: 'activateTab', tabId: 't3' });
  });

  it('shows no tab menu for a pane with one tab', () => {
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(pane('g2').querySelector('[data-testid="chat-tab-overflow-trigger"]')).toBeNull();
  });
});
