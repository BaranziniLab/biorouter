import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';
import { chatRowCopy } from '../chats/copy';

const mocks = vi.hoisted(() => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(navigator, { clipboard: { writeText: vi.fn(() => Promise.resolve()) } });
  Object.assign(window, { electron: { createChatWindow: vi.fn() } });
});

function tab(over: Partial<ChatTab> = {}): ChatTab {
  return {
    tabId: 'tab-1',
    sessionId: '20260823_2',
    title: 'Cohort query',
    userSetName: false,
    cwd: '/Users/x/project',
    ...over,
  };
}

function renderStrip(over: Partial<ChatTabStripProps> = {}) {
  const props: ChatTabStripProps = {
    tabs: [tab()],
    activeTabId: 'tab-1',
    runningSessionIds: [],
    onSelect: vi.fn(),
    onClose: vi.fn(),
    onReorder: vi.fn(),
    reserveTitlebar: false,
    isCompactSidebarOverlayOpen: false,
    ...over,
  };
  const utils = render(<ChatTabStrip {...props} />);
  return { ...utils, props };
}

/** #114: parity with History and Recents — the tab strip is where a user is
 *  most likely to be looking at the conversation whose id they want. */
describe('ChatTabStrip right-click menu', () => {
  it('offers Rename and the three shared actions on a right-click', async () => {
    renderStrip();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));

    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.textContent)).toEqual([
      `${chatRowCopy.menu.rename}${chatRowCopy.menu.renameShortcut}`,
      chatRowCopy.menu.openInNewTab,
      chatRowCopy.menu.openInNewWindow,
      chatRowCopy.menu.copyId,
    ]);
  });

  it('copies the tab’s session id, not its tab id', async () => {
    renderStrip();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));
    fireEvent.click(await screen.findByText(chatRowCopy.menu.copyId));

    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('20260823_2'));
    expect(navigator.clipboard.writeText).not.toHaveBeenCalledWith('tab-1');
  });

  /**
   * `onSelect` takes the TAB id, while every other action takes the session id —
   * the two are deliberately different (`chatGroupsTypes.ts`: a tab keeps its
   * identity across a session bind), so passing the wrong one is a real and easy
   * mistake that would activate nothing.
   */
  it('activates the tab by its tab id', async () => {
    const { props } = renderStrip();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));
    fireEvent.click(await screen.findByText('Open in new tab'));

    await waitFor(() => expect(props.onSelect).toHaveBeenCalledWith('tab-1'));
  });

  it('opens a window on the tab’s working directory when it has one', async () => {
    renderStrip();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));
    fireEvent.click(await screen.findByText('Open in new window'));

    await waitFor(() =>
      expect(window.electron.createChatWindow).toHaveBeenCalledWith(
        undefined,
        '/Users/x/project',
        undefined,
        '20260823_2',
        'pair'
      )
    );
  });

  /**
   * A right-click must not start a tab drag. `beginDrag` already returns early
   * on any button but 0 — that guard is the precondition for putting a menu here
   * at all, so it is pinned rather than assumed.
   */
  it('does not begin a drag on the secondary button', () => {
    const { container, props } = renderStrip({
      tabs: [tab(), tab({ tabId: 'tab-2', sessionId: '20260823_3', title: 'Second' })],
    });
    const label = screen.getByRole('tab', { name: /Cohort query/ });
    fireEvent.pointerDown(label, { button: 2, buttons: 2, clientX: 0, clientY: 0 });
    fireEvent.pointerMove(label, { button: 2, buttons: 2, clientX: 200, clientY: 0 });
    fireEvent.pointerUp(label, { button: 2, buttons: 2, clientX: 200, clientY: 0 });

    expect(props.onReorder).not.toHaveBeenCalled();
    expect(container.querySelector('.br-tab[data-dragging="true"]')).toBeNull();
  });

  /**
   * ⚠ The trigger is on the LABEL BUTTON, never on `.br-tab`. That wrapper owns
   * the drag gesture and the drop-target attributes, and `ChatTabStrip` is
   * explicit that nothing new may be declared on it — a merged handler there is
   * the app-region/drag race the strip's comments were written for.
   */
  it('keeps the trigger off the .br-tab wrapper', () => {
    const { container } = renderStrip();
    const wrapper = container.querySelector('[data-tab-id="tab-1"]') as HTMLElement;
    expect(wrapper.getAttribute('data-state')).toBeNull();
    expect(screen.getByRole('tab', { name: /Cohort query/ }).closest('.br-tab')).toBe(wrapper);
  });
});
