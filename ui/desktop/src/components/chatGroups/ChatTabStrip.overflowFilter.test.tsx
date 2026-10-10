import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { ChatTabStrip, ChatTabStripProps, tabMatchesFilter } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';
import { chatTabStripCopy, TAB_MENU_FILTER_MIN } from './copy';

/**
 * "Searching tabs" (owner message 1, spec 3.16): the ▾ overflow menu grows a
 * filter field once it lists eight or more tabs. Results replace at once, the
 * empty line fades in, Enter opens the first match and ArrowDown walks into the
 * list. jsdom computes no overflow, so the strip's measurement is stubbed the
 * way `ChatTabStrip.test.tsx` stubs it.
 */

function tab(i: number, title = `Chat ${i}`): ChatTab {
  return { tabId: `tab-${i}`, sessionId: `s${i}`, title, userSetName: false };
}

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

function renderStrip(tabs: ChatTab[], over: Partial<ChatTabStripProps> = {}) {
  const props: ChatTabStripProps = {
    tabs,
    activeTabId: tabs[0]?.tabId ?? null,
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

const titles = [
  'Cohort query',
  'Variant calling',
  'Spoke graph',
  'Single cell atlas',
  'Grant draft',
  'Journal club',
  'Cohort follow-up',
  'Figures',
];
const eight = titles.map((title, i) => tab(i + 1, title));

function openMenu() {
  fireEvent.pointerDown(screen.getByTestId('chat-tab-overflow-trigger'));
}

describe('the tab overflow menu filter', () => {
  it('matches a title by case-insensitive substring and treats blank as no filter', () => {
    expect(tabMatchesFilter('Cohort query', 'cohort')).toBe(true);
    expect(tabMatchesFilter('Cohort query', 'QUERY')).toBe(true);
    expect(tabMatchesFilter('Cohort query', 'spoke')).toBe(false);
    expect(tabMatchesFilter('Cohort query', '   ')).toBe(true);
  });

  it('has no filter below eight tabs', () => {
    const restore = stubOverflowing();
    try {
      renderStrip(eight.slice(0, TAB_MENU_FILTER_MIN - 1));
      openMenu();
      expect(screen.queryByTestId('chat-tab-overflow-filter')).toBeNull();
      expect(screen.getAllByRole('menuitem')).toHaveLength(TAB_MENU_FILTER_MIN - 1);
    } finally {
      restore();
    }
  });

  it('opens with the filter focused at eight tabs, and typing replaces the list at once', () => {
    const restore = stubOverflowing();
    try {
      renderStrip(eight);
      openMenu();
      const field = screen.getByRole('searchbox', { name: chatTabStripCopy.filterLabel });
      expect(field).toHaveFocus();
      expect(screen.getAllByRole('menuitem')).toHaveLength(8);

      fireEvent.change(field, { target: { value: 'cohort' } });
      const items = screen.getAllByRole('menuitem');
      expect(items.map((item) => item.textContent)).toEqual([
        expect.stringContaining('Cohort query'),
        expect.stringContaining('Cohort follow-up'),
      ]);
    } finally {
      restore();
    }
  });

  it('says "No tabs match" when nothing matches', () => {
    const restore = stubOverflowing();
    try {
      renderStrip(eight);
      openMenu();
      fireEvent.change(screen.getByTestId('chat-tab-overflow-filter'), {
        target: { value: 'zzz' },
      });
      expect(screen.queryAllByRole('menuitem')).toHaveLength(0);
      expect(screen.getByRole('status')).toHaveTextContent(chatTabStripCopy.noMatch);
    } finally {
      restore();
    }
  });

  it('Enter in the field opens the first match', () => {
    const restore = stubOverflowing();
    try {
      const { props } = renderStrip(eight);
      openMenu();
      const field = screen.getByTestId('chat-tab-overflow-filter');
      fireEvent.change(field, { target: { value: 'journal' } });
      fireEvent.keyDown(field, { key: 'Enter' });
      expect(props.onSelect).toHaveBeenCalledWith('tab-6');
    } finally {
      restore();
    }
  });

  it('ArrowDown moves into the list; a letter typed on a row goes back into the field', () => {
    const restore = stubOverflowing();
    try {
      renderStrip(eight);
      openMenu();
      const field = screen.getByTestId('chat-tab-overflow-filter') as HTMLInputElement;
      fireEvent.keyDown(field, { key: 'ArrowDown' });
      const first = screen.getAllByRole('menuitem')[0];
      expect(first).toHaveFocus();

      // A letter on a row is the filter's, not the menu's first-letter jump.
      fireEvent.keyDown(first, { key: 'j' });
      expect(field).toHaveFocus();
      expect(field.value).toBe('j');
      expect(screen.getAllByRole('menuitem').map((item) => item.textContent)).toEqual([
        expect.stringContaining('Journal club'),
      ]);
    } finally {
      restore();
    }
  });

  it('marks the tab you are on with the trailing check, not with weight', () => {
    const restore = stubOverflowing();
    try {
      renderStrip(eight, { activeTabId: 'tab-3' });
      openMenu();
      const current = screen.getByTestId('chat-tab-overflow-item-tab-3');
      expect(current).toHaveAttribute('aria-current', 'true');
      expect(current.className).not.toMatch(/font-medium/);
      expect(screen.getByLabelText(chatTabStripCopy.current)).toBeInTheDocument();
      expect(screen.getByTestId('chat-tab-overflow-item-tab-1')).not.toHaveAttribute(
        'aria-current'
      );
    } finally {
      restore();
    }
  });

  it('the empty line fades in on the motion tokens (asserted at the source)', () => {
    const css = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
    const rule = css.match(/\.br-tabstrip__menu-empty\s*\{([^}]*)\}/);
    expect(rule, 'no .br-tabstrip__menu-empty rule').not.toBeNull();
    expect(rule![1]).toMatch(/animation:\s*br-fade-in var\(--dur-fast-max\) var\(--ease-out\)/);
  });
});
