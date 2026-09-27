import { useState } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { Dialog, DialogContent, DialogDescription, DialogTitle } from './dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from './dropdown-menu';

/**
 * Q2-50 (live QA round 2). Radix swallows Tab inside a menu, so a keyboard user
 * who opened one could only leave with Escape (a11y P2-6, erin P2-10, carol
 * P2-28). The APG menu button pattern closes the menu on Tab and moves focus on
 * from the trigger, which is what these pin; and the menu's enter/exit ease is
 * the app's `--ease-out`, not the browser's `ease`.
 */
function Page({
  onOpenChange,
  onKeyDown,
  controlled = false,
  withSubmenu = false,
}: {
  onOpenChange?: (open: boolean) => void;
  onKeyDown?: (event: React.KeyboardEvent<HTMLDivElement>) => void;
  controlled?: boolean;
  withSubmenu?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const control = controlled
    ? {
        open,
        onOpenChange: (next: boolean) => {
          onOpenChange?.(next);
          setOpen(next);
        },
      }
    : { onOpenChange };
  return (
    <div>
      <button type="button">Before</button>
      <DropdownMenu {...control}>
        <DropdownMenuTrigger asChild>
          <button type="button">Options</button>
        </DropdownMenuTrigger>
        <DropdownMenuContent onKeyDown={onKeyDown}>
          <DropdownMenuItem>Rename</DropdownMenuItem>
          <DropdownMenuItem>Archive</DropdownMenuItem>
          {withSubmenu && (
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>Move to</DropdownMenuSubTrigger>
              <DropdownMenuPortal>
                <DropdownMenuSubContent>
                  <DropdownMenuItem>Inbox</DropdownMenuItem>
                </DropdownMenuSubContent>
              </DropdownMenuPortal>
            </DropdownMenuSub>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <button type="button">After</button>
    </div>
  );
}

async function openWithKeyboard(user: ReturnType<typeof userEvent.setup>) {
  screen.getByRole('button', { name: 'Options' }).focus();
  await user.keyboard('{Enter}');
  await screen.findByRole('menu');
  await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Rename' })).toHaveFocus());
}

describe('Tab leaves an open menu (APG menu button)', () => {
  it('closes the menu and moves on to the stop after the trigger', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await openWithKeyboard(user);

    await user.tab();

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
  });

  it('closes the menu and moves back to the stop before the trigger on Shift+Tab', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await openWithKeyboard(user);

    await user.tab({ shift: true });

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'Before' })).toHaveFocus();
  });

  // The close is the menu's own, so the exit animation's focus return — which would put focus
  // back on the trigger and undo the Tab — must stand down once the content unmounts.
  it('does not hand focus back to the trigger once the content unmounts', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await openWithKeyboard(user);
    await user.tab();
    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    // FocusScope's unmount auto-focus runs on a timer.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
  });

  it('tells a controlled caller it closed, once', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<Page controlled onOpenChange={onOpenChange} />);
    await openWithKeyboard(user);
    onOpenChange.mockClear();

    await user.tab();

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    expect(onOpenChange.mock.calls).toEqual([[false]]);
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
  });

  it('tells an uncontrolled caller it closed, too', async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(<Page onOpenChange={onOpenChange} />);
    await openWithKeyboard(user);
    expect(onOpenChange).toHaveBeenLastCalledWith(true);

    await user.tab();

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    expect(onOpenChange).toHaveBeenLastCalledWith(false);
  });

  it('closes the whole menu from inside a submenu', async () => {
    const user = userEvent.setup();
    render(<Page withSubmenu />);
    await openWithKeyboard(user);
    await user.keyboard('{ArrowDown}{ArrowDown}');
    expect(screen.getByRole('menuitem', { name: 'Move to' })).toHaveFocus();
    await user.keyboard('{ArrowRight}');
    await waitFor(() => expect(screen.getByRole('menuitem', { name: 'Inbox' })).toHaveFocus());

    await user.tab();

    await waitFor(() => expect(screen.queryAllByRole('menu')).toHaveLength(0));
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
  });

  it('leaves a Tab the caller handled itself alone', async () => {
    const user = userEvent.setup();
    const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.key === 'Tab') event.preventDefault();
    };
    render(<Page onKeyDown={onKeyDown} />);
    await openWithKeyboard(user);

    await user.tab();

    expect(screen.getByRole('menu')).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Rename' })).toHaveFocus();
  });

  it('still returns focus to the trigger on Escape', async () => {
    const user = userEvent.setup();
    render(<Page />);
    await openWithKeyboard(user);

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    await waitFor(() => expect(screen.getByRole('button', { name: 'Options' })).toHaveFocus());
  });

  // A modal menu traps focus while it is open, so the move lands once it has unmounted.
  it('moves on from a modal menu too, once its focus trap lets go', async () => {
    const user = userEvent.setup();
    render(
      <div>
        <button type="button">Before</button>
        <DropdownMenu modal>
          <DropdownMenuTrigger asChild>
            <button type="button">Options</button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuItem>Rename</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <button type="button">After</button>
      </div>
    );
    await openWithKeyboard(user);

    await user.tab();

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    await waitFor(() => expect(screen.getByRole('button', { name: 'After' })).toHaveFocus());
  });

  // A trigger inside a roving group carries tabindex="-1"; Tab continues from where it sits.
  it('continues from a trigger that is not itself a tab stop', async () => {
    const user = userEvent.setup();
    render(
      <div>
        <button type="button">Before</button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button type="button" tabIndex={-1}>
              Row options
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent>
            <DropdownMenuItem>Rename</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <button type="button">After</button>
      </div>
    );
    screen.getByRole('button', { name: 'Row options' }).focus();
    await user.keyboard('{Enter}');
    await screen.findByRole('menu');

    await user.tab();

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'After' })).toHaveFocus();
  });
});

/**
 * Q2-50 round 2. The Tab-out search used to run over the whole document, so from a dialog's
 * first control Shift+Tab went to the page BEHIND the dialog: the open menu pauses the dialog's
 * focus trap, the page behind is only `aria-hidden` (still in the Tab order), and the trap does not
 * pull focus back when it resumes. PermissionModal is the live case — the first tool's
 * permission menu is the dialog's first stop, and its × comes after it.
 */
function MenuInDialog({
  modalMenu = false,
  menuLast = false,
}: {
  modalMenu?: boolean;
  /** Put the menu after every other stop, with no ×, so forward Tab is the one off the edge. */
  menuLast?: boolean;
}) {
  const menu = (
    <DropdownMenu modal={modalMenu}>
      <DropdownMenuTrigger asChild>
        <button type="button">Options</button>
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        <DropdownMenuItem>Rename</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
  return (
    <div>
      <button type="button">Background</button>
      <Dialog open>
        <DialogContent showCloseButton={!menuLast}>
          <DialogTitle>Permissions</DialogTitle>
          <DialogDescription>Choose what each tool may do.</DialogDescription>
          {!menuLast && menu}
          <button type="button">Save</button>
          {menuLast && menu}
        </DialogContent>
      </Dialog>
      <button type="button">Background after</button>
    </div>
  );
}

describe('Tab out of a menu inside a dialog stays in the dialog', () => {
  it('wraps Shift+Tab from the dialog’s first control to its last, not the page behind', async () => {
    const user = userEvent.setup();
    render(<MenuInDialog />);
    await openWithKeyboard(user);

    await user.tab({ shift: true });

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    // DialogContent renders its × after the children: it is the dialog's last stop.
    const close = screen.getByRole('button', { name: 'Close' });
    expect(close).toHaveFocus();
    expect(screen.getByRole('dialog')).toContainElement(close);
    // Past the unmount's focus return, too.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(close).toHaveFocus();
  });

  it('still moves forward to the next stop inside the dialog', async () => {
    const user = userEvent.setup();
    render(<MenuInDialog />);
    await openWithKeyboard(user);

    await user.tab();

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'Save' })).toHaveFocus();
  });

  it('wraps Tab from the dialog’s last control to its first, not the page after it', async () => {
    const user = userEvent.setup();
    render(<MenuInDialog menuLast />);
    await openWithKeyboard(user);

    await user.tab();

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'Save' })).toHaveFocus();
  });

  // A modal menu marks the whole page `aria-hidden`, the dialog it sits in included, so a search
  // that skipped hidden elements would find nothing at all.
  it('stays in the dialog from a modal menu, which hides the dialog itself', async () => {
    const user = userEvent.setup();
    render(<MenuInDialog modalMenu />);
    await openWithKeyboard(user);
    expect(
      screen.getByRole('dialog', { hidden: true }).closest('[aria-hidden="true"]')
    ).not.toBeNull();

    await user.tab({ shift: true });

    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument());
    await waitFor(() => expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus());
    expect(screen.getByRole('button', { name: 'Background', hidden: true })).not.toHaveFocus();
  });
});

describe('menu motion', () => {
  it('eases the content and a submenu with --ease-out, not the browser ease', async () => {
    const user = userEvent.setup();
    render(<Page withSubmenu />);
    await openWithKeyboard(user);
    expect(screen.getByRole('menu')).toHaveClass('ease-[var(--ease-out)]');

    await user.keyboard('{ArrowDown}{ArrowDown}{ArrowRight}');
    await waitFor(() => expect(screen.getAllByRole('menu')).toHaveLength(2));
    for (const menu of screen.getAllByRole('menu')) {
      expect(menu).toHaveClass('ease-[var(--ease-out)]');
    }
  });
});
