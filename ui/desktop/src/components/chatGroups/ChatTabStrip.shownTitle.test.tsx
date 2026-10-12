import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';

import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';
import { chatTabStripCopy, TAB_MENU_FILTER_MIN } from './copy';
import { chatRowCopy } from '../chats/copy';
import type { TabAnnotation } from './workspaceCommandPlanner';

/**
 * A tab says its kind once (vision V1-R2-10 and V1-R3-6, inbox WS-SIDEBAR-4): the
 * glyph says "sub-agent" or "app", so the title drops what the glyph already says,
 * from the resolver History and the sidebar use (`historyRowTitle`). Display only:
 * the stored title is what the rename starts from and rolls back to, and the
 * overflow filter matches both spellings.
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

const STORED = 'Subagent: Reply with ALPHA';
const SHOWN = 'Reply with ALPHA';
const subagent: TabAnnotation = { badge: 'subagent', parentSessionId: 'parent-1' };

function tab(over: Partial<ChatTab> = {}): ChatTab {
  return { tabId: 'tab-1', sessionId: 's1', title: STORED, userSetName: false, ...over };
}

function renderStrip(over: Partial<ChatTabStripProps> = {}) {
  const props: ChatTabStripProps = {
    tabs: [tab()],
    activeTabId: 'tab-1',
    runningSessionIds: [],
    tabAnnotations: { s1: subagent },
    onSelect: vi.fn(),
    onClose: vi.fn(),
    onReorder: vi.fn(),
    reserveTitlebar: false,
    isCompactSidebarOverlayOpen: false,
    ...over,
  };
  return render(<ChatTabStrip {...props} />);
}

const tabNode = (container: HTMLElement, tabId = 'tab-1') =>
  container.querySelector(`[data-tab-id="${tabId}"]`) as HTMLElement;

describe('ChatTabStrip: the title a tab shows', () => {
  it('a sub-agent tab shows its label beside the robot glyph, not "Subagent:"', () => {
    const { container } = renderStrip();
    const node = tabNode(container);
    expect(node.querySelector('[data-chat-kind]')?.getAttribute('data-chat-kind')).toBe('subagent');
    expect(node.querySelector('.br-tab__label')?.textContent).toBe(SHOWN);
    const button = within(node).getByRole('tab');
    expect(button.getAttribute('title')).toBe(SHOWN);
    expect(within(node).getByRole('button', { name: chatTabStripCopy.close(SHOWN) })).toBeTruthy();
    expect(node.textContent).not.toContain('Subagent:');
  });

  it('takes the kind from the row too, after the annotation is gone', () => {
    const { container } = renderStrip({ tabAnnotations: {}, sessionTypes: { s1: 'sub_agent' } });
    expect(tabNode(container).querySelector('.br-tab__label')?.textContent).toBe(SHOWN);
  });

  it('a running sub-agent tab names its close control by the shown title as well', () => {
    const { container } = renderStrip({ runningSessionIds: ['s1'] });
    expect(
      within(tabNode(container)).getByRole('button', { name: chatTabStripCopy.close(SHOWN) })
    ).toBeTruthy();
  });

  it('keyed on the kind: a plain chat a person titled "Subagent: x" keeps its title', () => {
    const { container } = renderStrip({ tabAnnotations: {}, sessionTypes: { s1: 'user' } });
    expect(tabNode(container).querySelector('.br-tab__label')?.textContent).toBe(STORED);
  });

  it('an app chat reads as the app, not as app:<id>', () => {
    const { container } = renderStrip({
      tabs: [tab({ title: 'app:living-news-router' })],
      tabAnnotations: {},
    });
    const node = tabNode(container);
    expect(node.querySelector('[data-chat-kind]')?.getAttribute('data-chat-kind')).toBe('app');
    expect(node.querySelector('.br-tab__label')?.textContent).toBe('Living news router');
  });

  it('a scheduled run keeps "Scheduled job:", because its tab draws no clock to say it', () => {
    const { container } = renderStrip({
      tabs: [tab({ title: 'Scheduled job: daily-meditation' })],
      tabAnnotations: {},
      sessionTypes: { s1: 'scheduled' },
    });
    const node = tabNode(container);
    expect(node.querySelector('[data-chat-kind]')?.getAttribute('data-chat-kind')).toBe('chat');
    expect(node.querySelector('.br-tab__label')?.textContent).toBe(
      'Scheduled job: daily-meditation'
    );
  });

  it('rename starts from the stored name, under a label that names the shown title', () => {
    renderStrip();
    fireEvent.keyDown(screen.getByRole('tab'), { key: 'F2' });
    const input = screen.getByRole('textbox', {
      name: chatRowCopy.rename.inputLabel(SHOWN),
    }) as HTMLInputElement;
    expect(input.value).toBe(STORED);

    fireEvent.change(input, { target: { value: 'Audit pass' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(mocks.rename).toHaveBeenCalledWith('s1', 'Audit pass', {
      name: STORED,
      userSetName: false,
    });
  });
});

describe('ChatTabStrip: the overflow menu shows and finds the same title', () => {
  function stubOverflowing() {
    const scroll = vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get');
    const client = vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get');
    scroll.mockImplementation(function (this: HTMLElement) {
      return this.dataset.testid === 'chat-tab-strip' ? 2000 : 0;
    });
    client.mockImplementation(function (this: HTMLElement) {
      return this.dataset.testid === 'chat-tab-strip' ? 300 : 0;
    });
    return () => {
      scroll.mockRestore();
      client.mockRestore();
    };
  }

  const many: ChatTab[] = Array.from({ length: TAB_MENU_FILTER_MIN }, (_, i) =>
    i === 0 ? tab() : tab({ tabId: `tab-${i + 1}`, sessionId: `s${i + 1}`, title: `Chat ${i}` })
  );

  const openMenu = () => fireEvent.pointerDown(screen.getByTestId('chat-tab-overflow-trigger'));
  const rowTexts = () => screen.queryAllByRole('menuitem').map((item) => item.textContent ?? '');

  it('lists the sub-agent tab by its shown title', () => {
    const restore = stubOverflowing();
    try {
      renderStrip({ tabs: many });
      openMenu();
      const row = screen.getByTestId('chat-tab-overflow-item-tab-1');
      expect(row.textContent).toContain(SHOWN);
      expect(row.textContent).not.toContain('Subagent:');
    } finally {
      restore();
    }
  });

  it('matches the shown title and the stored name', () => {
    const restore = stubOverflowing();
    try {
      renderStrip({ tabs: many });
      openMenu();
      const field = screen.getByTestId('chat-tab-overflow-filter');

      fireEvent.change(field, { target: { value: 'alpha' } });
      expect(rowTexts()).toEqual([expect.stringContaining(SHOWN)]);

      fireEvent.change(field, { target: { value: 'subagent' } });
      expect(rowTexts()).toEqual([expect.stringContaining(SHOWN)]);
    } finally {
      restore();
    }
  });
});
