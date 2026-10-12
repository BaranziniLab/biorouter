import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';
import { chatRowCopy } from '../chats/copy';

/**
 * P3 (spec 3.16): Rename on a tab, in place, with the sidebar's editor
 * (`ChatRowRenameInput`) and the shared optimistic helper, so the tab, the
 * sidebar and History take the new name from one announcement.
 */

const mocks = vi.hoisted(() => ({
  rename: vi.fn((..._args: unknown[]) => Promise.resolve('renamed')),
}));

vi.mock('../../utils/sessionNameSync', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../utils/sessionNameSync')>();
  return { ...actual, renameSessionOptimistically: mocks.rename };
});

beforeEach(() => {
  mocks.rename.mockClear();
});

function tab(over: Partial<ChatTab> = {}): ChatTab {
  return {
    tabId: 'tab-1',
    sessionId: '20260823_2',
    title: 'Cohort query',
    userSetName: false,
    ...over,
  };
}

function renderStrip(over: Partial<ChatTabStripProps> = {}) {
  const props: ChatTabStripProps = {
    tabs: [tab(), tab({ tabId: 'tab-2', sessionId: '20260823_3', title: 'Second' })],
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

const editorFor = (title: string) =>
  screen.getByRole('textbox', { name: chatRowCopy.rename.inputLabel(title) });

describe('ChatTabStrip: rename a tab in place', () => {
  it('F2 on a tab opens its editor with the title selected, and Enter renames the chat', async () => {
    renderStrip();
    fireEvent.keyDown(screen.getByRole('tab', { name: /Cohort query/ }), { key: 'F2' });

    const input = editorFor('Cohort query') as HTMLInputElement;
    expect(input).toHaveFocus();
    expect(input.value).toBe('Cohort query');

    fireEvent.change(input, { target: { value: 'Cohort query, women over 50' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    expect(mocks.rename).toHaveBeenCalledWith('20260823_2', 'Cohort query, women over 50', {
      name: 'Cohort query',
      userSetName: false,
    });
    // The editor closes and focus returns to the tab.
    await waitFor(() => expect(screen.getByRole('tab', { name: /Cohort query/ })).toHaveFocus());
  });

  it('Escape cancels without renaming', () => {
    renderStrip();
    fireEvent.keyDown(screen.getByRole('tab', { name: /Cohort query/ }), { key: 'F2' });
    const input = editorFor('Cohort query');
    fireEvent.change(input, { target: { value: 'Something else' } });
    fireEvent.keyDown(input, { key: 'Escape' });

    expect(mocks.rename).not.toHaveBeenCalled();
    expect(screen.getByRole('tab', { name: /Cohort query/ })).toBeInTheDocument();
  });

  it('Rename in the right-click menu opens the same editor once the menu lets go', async () => {
    renderStrip();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Cohort query/ }));
    fireEvent.click(await screen.findByText(chatRowCopy.menu.rename));

    await waitFor(() => expect(editorFor('Cohort query')).toHaveFocus());
  });

  it('Shift+F10 opens the tab menu from the keyboard', async () => {
    renderStrip();
    fireEvent.keyDown(screen.getByRole('tab', { name: /Cohort query/ }), {
      key: 'F10',
      shiftKey: true,
    });
    expect(await screen.findByText(chatRowCopy.menu.rename)).toBeInTheDocument();
  });
});
