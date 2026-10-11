import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DangerousConfirmDialog } from './DangerousConfirmDialog';

afterEach(cleanup);

// The dialog reads the input that opened it (`ui/inputTrail`): a key open puts focus on Cancel, any
// other open rests on the dialog box (V3-R6-2). Each focus case says which it models.
const keyOpen = () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
const pointerOpen = () => window.dispatchEvent(new Event('pointerdown'));

describe('DangerousConfirmDialog', () => {
  it('Cancel holds initial focus after a key open and Enter does not fire the destructive action', async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();

    keyOpen();
    render(
      <DangerousConfirmDialog
        open
        title="Make this chat public?"
        phrase="def456"
        fieldLabel="Type the last 6 characters of this chat's ID"
        confirmLabel="Make public"
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />
    );

    // Radix's own default focuses the FIRST tabbable node in the content (here
    // the phrase field), chosen by layout. This dialog overrides it.
    expect(document.activeElement).toBe(screen.getByRole('button', { name: /Cancel/ }));

    await user.type(screen.getByRole('textbox'), 'def4{Enter}');
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('does not fire the destructive action on Enter even once the phrase is complete', async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();

    render(
      <DangerousConfirmDialog
        open
        title="Make this chat public?"
        phrase="def456"
        fieldLabel="Type the last 6 characters of this chat's ID"
        confirmLabel="Make public"
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />
    );

    // The interesting case: the field is satisfied, so the confirm is enabled,
    // and a stray Return in the text field is exactly the accident a typed
    // confirmation exists to prevent.
    await user.type(screen.getByRole('textbox'), 'def456{Enter}');
    expect(screen.getByRole('button', { name: /Make public/ })).toBeEnabled();
    expect(onConfirm).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: /Make public/ }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it('gates the confirm on the exact phrase, ignoring case and surrounding space', async () => {
    const user = userEvent.setup();
    render(
      <DangerousConfirmDialog
        open
        title="Disable protection?"
        phrase="DISABLE PROTECTION"
        fieldLabel="Type DISABLE PROTECTION to continue"
        confirmLabel="Disable"
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    const confirm = screen.getByRole('button', { name: /Disable/ });
    const field = screen.getByRole('textbox');
    expect(confirm).toBeDisabled();

    await user.type(field, 'DISABLE');
    expect(confirm).toBeDisabled();

    await user.clear(field);
    await user.type(field, ' disable protection ');
    expect(confirm).toBeEnabled();
  });

  it('needs no phrase when none is required, and still never confirms on Enter', async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    keyOpen();
    render(
      <DangerousConfirmDialog
        open
        title="Make this chat public?"
        confirmLabel="Make public"
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />
    );

    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.getByRole('button', { name: /Make public/ })).toBeEnabled();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: /Cancel/ }));

    await user.keyboard('{Enter}');
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it.each([
    ['a pointer press', pointerOpen],
    // A dialog the app raises on a page nobody has touched reads as the pointer.
    ['no input at all', () => undefined],
  ])(
    'rests on the dialog box after %s, never on Cancel or the destructive button',
    async (_label, before) => {
      const user = userEvent.setup();
      // A KEY press from an earlier case must not carry over into this one.
      pointerOpen();
      before();
      render(
        <DangerousConfirmDialog
          open
          title="Make this chat public?"
          description="This cannot be undone."
          phrase="def456"
          fieldLabel="Type the last 6 characters of this chat's ID"
          confirmLabel="Make public"
          onConfirm={vi.fn()}
          onCancel={vi.fn()}
        />
      );

      const dialog = screen.getByRole('dialog');
      expect(dialog).toHaveFocus();
      for (const button of screen.getAllByRole('button')) expect(button).not.toHaveFocus();

      // The next Tab reaches the phrase field, then Cancel: both before the destructive button.
      await user.tab();
      expect(screen.getByRole('textbox')).toHaveFocus();
      await user.tab();
      expect(screen.getByRole('button', { name: /Cancel/ })).toHaveFocus();
    }
  );

  it('rests on the box after a pointer open with no phrase, and Tab reaches Cancel first', async () => {
    const user = userEvent.setup();
    pointerOpen();
    render(
      <DangerousConfirmDialog
        open
        title="Make this chat public?"
        confirmLabel="Make public"
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    expect(screen.getByRole('dialog')).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: /Cancel/ })).toHaveFocus();
  });

  it('links a provided description to the dialog', () => {
    render(
      <DangerousConfirmDialog
        open
        title="Disable protection?"
        description="This cannot be undone."
        confirmLabel="Disable"
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    const dialog = screen.getByRole('dialog');
    const descriptionId = dialog.getAttribute('aria-describedby');
    expect(descriptionId).toBeTruthy();
    expect(document.getElementById(descriptionId!)).toHaveTextContent('This cannot be undone.');
  });

  it('opts out of aria-describedby when no description is provided', () => {
    render(
      <DangerousConfirmDialog
        open
        title="Disable protection?"
        confirmLabel="Disable"
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    expect(screen.getByRole('dialog')).not.toHaveAttribute('aria-describedby');
  });
});
