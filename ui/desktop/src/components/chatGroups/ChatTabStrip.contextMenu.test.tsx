import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';
import { consumeTabKeyboardFocus, isTabKeyboardFocusPending } from './chatTabKeyboardFocus';
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

/** Three chats, the first in front. */
function threeTabs(): ChatTab[] {
  return [
    tab(),
    tab({ tabId: 'tab-2', sessionId: '20260823_3', title: 'Second' }),
    tab({ tabId: 'tab-3', sessionId: '20260823_4', title: 'Third' }),
  ];
}

function renderStrip(over: Partial<ChatTabStripProps> = {}) {
  const props: ChatTabStripProps = {
    tabs: [tab()],
    activeTabId: 'tab-1',
    runningSessionIds: [],
    onSelect: vi.fn(),
    onClose: vi.fn(),
    onCloseOthers: vi.fn(),
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
  /**
   * V2-R6-3. A tab IS the open tab, so "Open in new tab" only selected the tab
   * the person was already pointing at and read as an action that did nothing
   * (20 tabs before, 20 after). The menu a tab opens closes it instead.
   */
  it('offers Rename, Open in new window, Copy chat ID and the two closers on a right-click', async () => {
    renderStrip({ tabs: threeTabs() });
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));

    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.textContent)).toEqual([
      `${chatRowCopy.menu.rename}${chatRowCopy.menu.renameShortcut}`,
      chatRowCopy.menu.openInNewWindow,
      chatRowCopy.menu.copyId,
      chatRowCopy.menu.closeTab,
      chatRowCopy.menu.closeOtherTabs,
    ]);
    expect(screen.queryByText(chatRowCopy.menu.openInNewTab)).toBeNull();
    // Rename · window · copy · the closers: a hairline between each group.
    expect(screen.getAllByRole('separator')).toHaveLength(3);
  });

  it('says the closers in the words the sidebar copy gives them', () => {
    expect(chatRowCopy.menu.closeTab).toBe('Close tab');
    expect(chatRowCopy.menu.closeOtherTabs).toBe('Close other tabs');
  });

  it('does not offer Close other tabs on a lone tab', async () => {
    renderStrip();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));

    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.getAttribute('data-chat-row-action'))).toEqual([
      'rename',
      'open-window',
      'copy-id',
      'close-tab',
    ]);
  });

  it('does not offer Close other tabs to a strip that was given no way to do it', async () => {
    renderStrip({ tabs: threeTabs(), onCloseOthers: undefined });
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));

    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.getAttribute('data-chat-row-action'))).not.toContain(
      'close-other-tabs'
    );
    expect(items.map((item) => item.getAttribute('data-chat-row-action'))).toContain('close-tab');
  });

  /**
   * The menu belongs to the tab that was right-clicked, not to the one in front:
   * closing the second tab from its own menu must close the second tab and leave
   * the first alone, and must not select anything on the way.
   */
  it('closes the tab that was right-clicked, by its tab id', async () => {
    const { props } = renderStrip({ tabs: threeTabs() });
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Second/ }));
    fireEvent.click(await screen.findByText(chatRowCopy.menu.closeTab));

    await waitFor(() => expect(props.onClose).toHaveBeenCalledWith('tab-2'));
    expect(props.onClose).toHaveBeenCalledTimes(1);
    expect(props.onCloseOthers).not.toHaveBeenCalled();
    expect(props.onSelect).not.toHaveBeenCalled();
  });

  it('keeps the tab that was right-clicked and closes the others', async () => {
    const { props } = renderStrip({ tabs: threeTabs() });
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Second/ }));
    fireEvent.click(await screen.findByText(chatRowCopy.menu.closeOtherTabs));

    await waitFor(() => expect(props.onCloseOthers).toHaveBeenCalledWith('tab-2'));
    expect(props.onCloseOthers).toHaveBeenCalledTimes(1);
    // The strip does not walk the others itself: one action closes them all.
    expect(props.onClose).not.toHaveBeenCalled();
  });

  /**
   * A keyboard close of the tab in front hands focus to the tab that takes over
   * (the closed tab's button is gone, and with it the menu's place to give focus
   * back to). A pointer close asks for nothing.
   */
  describe('where focus goes after a close', () => {
    afterEach(() => {
      for (const id of ['tab-1', 'tab-2', 'tab-3']) consumeTabKeyboardFocus(id);
    });

    it('asks for the successor after a keyboard Close tab on the tab in front', async () => {
      renderStrip({ tabs: threeTabs(), activeTabId: 'tab-2' });
      fireEvent.contextMenu(screen.getByRole('tab', { name: /Second/ }));
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
      fireEvent.click(await screen.findByText(chatRowCopy.menu.closeTab));

      // Closing the middle tab, the one that was after it takes its place.
      expect(isTabKeyboardFocusPending('tab-3')).toBe(true);
      expect(isTabKeyboardFocusPending('tab-1')).toBe(false);
    });

    it('asks for the one before when the last tab in front is closed', async () => {
      renderStrip({ tabs: threeTabs(), activeTabId: 'tab-3' });
      fireEvent.contextMenu(screen.getByRole('tab', { name: /Third/ }));
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
      fireEvent.click(await screen.findByText(chatRowCopy.menu.closeTab));

      expect(isTabKeyboardFocusPending('tab-2')).toBe(true);
    });

    it('keeps focus on the tab in front when another tab is closed', async () => {
      renderStrip({ tabs: threeTabs(), activeTabId: 'tab-1' });
      fireEvent.contextMenu(screen.getByRole('tab', { name: /Third/ }));
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
      fireEvent.click(await screen.findByText(chatRowCopy.menu.closeTab));

      expect(isTabKeyboardFocusPending('tab-1')).toBe(true);
    });

    it('asks for the kept tab after a keyboard Close other tabs', async () => {
      renderStrip({ tabs: threeTabs(), activeTabId: 'tab-1' });
      fireEvent.contextMenu(screen.getByRole('tab', { name: /Second/ }));
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
      fireEvent.click(await screen.findByText(chatRowCopy.menu.closeOtherTabs));

      expect(isTabKeyboardFocusPending('tab-2')).toBe(true);
    });

    it('asks for nothing after a pointer choice', async () => {
      renderStrip({ tabs: threeTabs(), activeTabId: 'tab-2' });
      fireEvent.contextMenu(screen.getByRole('tab', { name: /Second/ }));
      window.dispatchEvent(new Event('pointerdown'));
      fireEvent.click(await screen.findByText(chatRowCopy.menu.closeTab));

      for (const id of ['tab-1', 'tab-2', 'tab-3']) {
        expect(isTabKeyboardFocusPending(id)).toBe(false);
      }
    });

    /**
     * Radix hands focus back to the control that opened a menu. After a click
     * that is a script-set focus on a tab the person only clicked, and Chromium
     * draws the keyboard ring on it; a key choice keeps the return.
     */
    it('does not hand focus back to the tab after a pointer choice, and does after a key', async () => {
      renderStrip({ tabs: threeTabs(), activeTabId: 'tab-1' });
      const second = screen.getByRole('tab', { name: /Second/ });

      second.focus();
      fireEvent.contextMenu(second);
      window.dispatchEvent(new Event('pointerdown'));
      fireEvent.click(await screen.findByText(chatRowCopy.menu.closeOtherTabs));
      await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
      expect(document.activeElement).not.toBe(second);

      second.focus();
      fireEvent.contextMenu(second);
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
      fireEvent.click(await screen.findByText(chatRowCopy.menu.closeOtherTabs));
      await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
      expect(document.activeElement).toBe(second);
    });
  });

  it('copies the tab’s session id, not its tab id', async () => {
    renderStrip();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));
    fireEvent.click(await screen.findByText(chatRowCopy.menu.copyId));

    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('20260823_2'));
    expect(navigator.clipboard.writeText).not.toHaveBeenCalledWith('tab-1');
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
