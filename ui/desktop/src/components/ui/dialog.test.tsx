import { useState } from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from './dialog';

afterEach(cleanup);

function ControlledDialog({ dismissible = true, onClose = () => {} }) {
  const [open, setOpen] = useState(false);

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (!nextOpen) onClose();
      }}
    >
      <DialogTrigger>Open dialog</DialogTrigger>
      <DialogContent dismissible={dismissible} aria-describedby={undefined}>
        <DialogTitle>Test dialog</DialogTitle>
        <button type="button">Inside action</button>
      </DialogContent>
    </Dialog>
  );
}

describe('DialogContent dismissal contract', () => {
  it('dismisses with Escape and restores focus to the trigger', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ControlledDialog onClose={onClose} />);

    const trigger = screen.getByRole('button', { name: 'Open dialog' });
    await user.click(trigger);
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(trigger).toHaveFocus();
  });

  it('dismisses when the backdrop is pressed', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ControlledDialog onClose={onClose} />);

    await user.click(screen.getByRole('button', { name: 'Open dialog' }));
    const overlay = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]');
    expect(overlay).not.toBeNull();
    fireEvent.pointerDown(overlay!);

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  /**
   * Defect D3b (2026-09-13). Toasts sit above every modal and are rendered
   * outside any dialog's tree, so a press on one reached Radix as a press
   * OUTSIDE the dialog: measured in the running app, one click on an error
   * toast's × closed "Make this chat public?" underneath while the toast stayed
   * on screen a minute later. The toast layer's own markup is reproduced here —
   * `section.Toastify` > `.Toastify__toast-container` > the card — because the
   * container is the hook the guard keys on.
   */
  it('does not treat a press on a toast as a press on the backdrop', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ControlledDialog onClose={onClose} />);
    await user.click(screen.getByRole('button', { name: 'Open dialog' }));

    const layer = document.createElement('section');
    layer.className = 'Toastify';
    layer.innerHTML =
      '<div class="Toastify__toast-container Toastify__toast-container--top-right">' +
      '<div role="alert"><button type="button" class="Toastify__close-button">×</button></div>' +
      '</div>';
    document.body.appendChild(layer);
    try {
      fireEvent.pointerDown(layer.querySelector('.Toastify__close-button')!);
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(screen.getByRole('dialog')).toBeInTheDocument();
      expect(onClose).not.toHaveBeenCalled();

      // …while the backdrop itself still dismisses, so the guard is not simply
      // "never dismiss on a press".
      fireEvent.pointerDown(document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]')!);
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
      expect(onClose).toHaveBeenCalledTimes(1);
    } finally {
      layer.remove();
    }
  });

  it('blocks Escape, backdrop, and the close button when not dismissible', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ControlledDialog dismissible={false} onClose={onClose} />);

    await user.click(screen.getByRole('button', { name: 'Open dialog' }));
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();

    await user.keyboard('{Escape}');
    const overlay = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]');
    expect(overlay).not.toBeNull();
    fireEvent.pointerDown(overlay!);

    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('closes only the topmost dialog on Escape', async () => {
    const user = userEvent.setup();

    function NestedDialogs() {
      const [parentOpen, setParentOpen] = useState(true);
      const [childOpen, setChildOpen] = useState(true);

      return (
        <Dialog open={parentOpen} onOpenChange={setParentOpen}>
          <DialogContent aria-describedby={undefined}>
            <DialogTitle>Parent dialog</DialogTitle>
            <Dialog open={childOpen} onOpenChange={setChildOpen}>
              <DialogContent aria-describedby={undefined}>
                <DialogTitle>Child dialog</DialogTitle>
              </DialogContent>
            </Dialog>
          </DialogContent>
        </Dialog>
      );
    }

    render(<NestedDialogs />);
    expect(screen.getByText('Parent dialog')).toBeInTheDocument();
    expect(screen.getByText('Child dialog')).toBeInTheDocument();

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.queryByText('Child dialog')).not.toBeInTheDocument());
    expect(screen.getByText('Parent dialog')).toBeInTheDocument();
  });
});
