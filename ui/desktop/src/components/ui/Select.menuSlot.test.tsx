import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Select, SELECT_MENU_SLOT } from './Select';

/**
 * The shared Select's open menu names itself as the Radix menus do (WS-MOTION-2, V1-R7-1): the
 * toast layer yields to any element with a menu `data-slot` and `data-state="open"`, so a select
 * listbox the person opened stands above a standing toast once its slot is in that rule.
 * jsdom cannot draw a portalled menu (react-select waits for a measured control), so that path is
 * held in Chromium by `selectMenuSlot.browser.test.tsx`.
 */
const OPTIONS = [
  { value: 'daily', label: 'Every day at 9:00' },
  { value: 'weekdays', label: 'Every weekday' },
];

const OPEN_MENU = `[data-slot='${SELECT_MENU_SLOT}'][data-state='open']`;

describe('the shared Select’s menu slot', () => {
  it('uses the slot name a Radix Select would carry', () => {
    expect(SELECT_MENU_SLOT).toBe('select-content');
  });

  it('marks the open menu surface, and nothing while the menu is closed', () => {
    const { container, rerender } = render(
      <Select aria-label="Schedule" options={OPTIONS} classNamePrefix="s" menuIsOpen />
    );
    const menu = container.querySelector('.s__menu');
    expect(menu).not.toBeNull();
    expect(menu).toHaveAttribute('data-slot', SELECT_MENU_SLOT);
    expect(menu).toHaveAttribute('data-state', 'open');
    // The surface itself, not a wrapper: the listbox sits inside it.
    expect(menu!.querySelector('[role="listbox"]')).not.toBeNull();
    expect(container.querySelectorAll(OPEN_MENU)).toHaveLength(1);

    rerender(
      <Select aria-label="Schedule" options={OPTIONS} classNamePrefix="s" menuIsOpen={false} />
    );
    expect(document.querySelector(OPEN_MENU)).toBeNull();
  });

  it('keeps the library’s own menu handlers (a press in the menu does not blur the field)', () => {
    const { container } = render(
      <Select aria-label="Schedule" options={OPTIONS} classNamePrefix="s" menuIsOpen />
    );
    const menu = container.querySelector('.s__menu')!;
    // react-select's onMenuMouseDown cancels a primary press so the input keeps focus; it is
    // passed in `innerProps`, which the slot is merged into rather than replacing.
    const notCancelled = fireEvent.mouseDown(menu, { button: 0 });
    expect(notCancelled).toBe(false);
  });

  it('still picks an option and lets a caller’s own Menu win', () => {
    const onChange = vi.fn();
    const { unmount } = render(
      <Select aria-label="Schedule" options={OPTIONS} menuIsOpen onChange={onChange} />
    );
    fireEvent.click(screen.getByRole('option', { name: 'Every weekday' }));
    expect(onChange).toHaveBeenCalledWith(OPTIONS[1], expect.anything());
    unmount();

    const CallerMenu = () => <div data-testid="caller-menu" />;
    render(
      <Select
        aria-label="Schedule"
        options={OPTIONS}
        menuIsOpen
        components={{ Menu: CallerMenu }}
      />
    );
    expect(screen.getByTestId('caller-menu')).toBeInTheDocument();
    expect(document.querySelector(OPEN_MENU)).toBeNull();
  });
});
