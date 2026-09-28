import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../ui/dialog';
import { useReturnFocusToOpener } from './useReturnFocusToOpener';

function Modal({ onClose, returnFocus }: { onClose: () => void; returnFocus: boolean }) {
  const returnToOpener = useReturnFocusToOpener();
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent {...(returnFocus ? { onCloseAutoFocus: returnToOpener } : {})}>
        <DialogTitle>Switch models</DialogTitle>
        <DialogDescription>Pick one.</DialogDescription>
        <input aria-label="Model" />
      </DialogContent>
    </Dialog>
  );
}

/** Opened from state, the way Settings opens its dialogs: no `Dialog.Trigger`. */
function Opener({ returnFocus }: { returnFocus: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Open
      </button>
      {open ? <Modal returnFocus={returnFocus} onClose={() => setOpen(false)} /> : null}
    </>
  );
}

afterEach(cleanup);

describe('useReturnFocusToOpener', () => {
  it('gives the focus back to the control that opened the dialog when Escape closes it', async () => {
    const user = userEvent.setup();
    render(<Opener returnFocus />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await screen.findByRole('dialog');

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(screen.getByRole('button', { name: 'Open' })).toHaveFocus());
  });

  it('is needed: without it the focus falls to the page', async () => {
    const user = userEvent.setup();
    render(<Opener returnFocus={false} />);
    await user.click(screen.getByRole('button', { name: 'Open' }));
    await screen.findByRole('dialog');

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.getByRole('button', { name: 'Open' })).not.toHaveFocus();
  });
});
