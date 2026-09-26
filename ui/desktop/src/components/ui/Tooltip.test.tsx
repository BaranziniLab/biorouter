import { useState } from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from './dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from './Tooltip';

/**
 * Q2-56 (live QA round 2). Radix opens a trigger's tooltip on any focus that did
 * not start with a pointer press, and the app hands focus back to an opener
 * every time a menu, dialog or pane closes — so "More actions", "Attach" and
 * "Analysis Lab options" popped their tooltips over the New label after every
 * close. The shared trigger now opens on focus only when the Tab key moved it.
 */
function Page({
  onOpenChange,
  onTriggerFocus,
}: {
  onOpenChange?: (open: boolean) => void;
  onTriggerFocus?: (event: React.FocusEvent<HTMLButtonElement>) => void;
}) {
  return (
    <div>
      <button type="button">Before</button>
      <Tooltip delayDuration={0} onOpenChange={onOpenChange}>
        <TooltipTrigger asChild>
          <button type="button" onFocus={onTriggerFocus}>
            Attach
          </button>
        </TooltipTrigger>
        <TooltipContent>Attach a file</TooltipContent>
      </Tooltip>
    </div>
  );
}

const trigger = () => screen.getByRole('button', { name: 'Attach' });

describe('a tooltip opens on focus only when Tab moved it', () => {
  it('opens when the person tabs onto the trigger', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());

    await user.tab();

    expect(trigger()).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Attach a file');
  });

  it('opens on Shift+Tab too', async () => {
    const user = userEvent.setup();
    render(
      <div>
        <Tooltip delayDuration={0}>
          <TooltipTrigger asChild>
            <button type="button">Attach</button>
          </TooltipTrigger>
          <TooltipContent>Attach a file</TooltipContent>
        </Tooltip>
        <button type="button">After</button>
      </div>
    );
    act(() => screen.getByRole('button', { name: 'After' }).focus());

    await user.tab({ shift: true });

    expect(trigger()).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toBeInTheDocument();
  });

  it('stays shut when a program puts focus on the trigger', async () => {
    render(<Page />);

    act(() => trigger().focus());

    expect(trigger()).toHaveFocus();
    // Radix opens on focus synchronously; give it a turn to prove it did not.
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
  });

  it('stays shut when a menu hands focus back to its trigger', async () => {
    const user = userEvent.setup();
    render(
      <DropdownMenu>
        <Tooltip delayDuration={0}>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <button type="button">More actions</button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent>More actions</TooltipContent>
        </Tooltip>
        <DropdownMenuContent>
          <DropdownMenuItem>Copy text</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    );
    const opener = screen.getByRole('button', { name: 'More actions' });
    act(() => opener.focus());
    await user.keyboard('{Enter}');
    await screen.findByRole('menu');

    await user.keyboard('{Escape}');

    await waitFor(() => expect(opener).toHaveFocus());
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
  });

  it('stays shut when focus comes back with the window', async () => {
    render(<Page />);
    // A Tab that never finished (the window lost focus mid-press) must not mark a later focus.
    fireEvent.keyDown(document.body, { key: 'Tab' });
    fireEvent.blur(window);

    act(() => trigger().focus());

    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
  });

  it('still opens on hover', async () => {
    const user = userEvent.setup();
    render(<Page />);

    await user.hover(trigger());

    expect(await screen.findByRole('tooltip')).toHaveTextContent('Attach a file');
  });

  it('opens on hover after a refused focus, and closes on Escape', async () => {
    const user = userEvent.setup();
    render(<Page />);
    act(() => trigger().focus());

    await user.hover(trigger());
    expect(await screen.findByRole('tooltip')).toBeInTheDocument();

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('tooltip')).not.toBeInTheDocument());
  });

  it('tells a caller only about the opens it allowed', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<Page onOpenChange={onOpenChange} />);

    act(() => trigger().focus());
    expect(onOpenChange).not.toHaveBeenCalled();

    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(onOpenChange).toHaveBeenLastCalledWith(true);
  });

  it('works for a controlled tooltip', async () => {
    const user = userEvent.setup();
    function Controlled() {
      const [open, setOpen] = useState(false);
      return (
        <div>
          <button type="button">Before</button>
          <Tooltip open={open} onOpenChange={setOpen}>
            <TooltipTrigger asChild>
              <button type="button">Attach</button>
            </TooltipTrigger>
            <TooltipContent>Attach a file</TooltipContent>
          </Tooltip>
          <span data-testid="state">{open ? 'open' : 'closed'}</span>
        </div>
      );
    }
    render(<Controlled />);

    act(() => trigger().focus());
    await act(() => new Promise((resolve) => setTimeout(resolve, 20)));
    expect(screen.getByTestId('state')).toHaveTextContent('closed');

    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(screen.getByTestId('state')).toHaveTextContent('open');
  });

  /**
   * The open is declined at the root, NOT by cancelling the focus event: a cancelled focus
   * event skips every handler Radix composes after the caller's, and a tooltip wraps triggers
   * that act on focus themselves (a tab activates, a roving item records its stop).
   */
  it('leaves the focus event itself alone for the trigger’s own handlers', () => {
    const seen: boolean[] = [];
    render(<Page onTriggerFocus={(event) => seen.push(event.defaultPrevented)} />);

    act(() => trigger().focus());

    expect(seen).toEqual([false]);
  });
});
