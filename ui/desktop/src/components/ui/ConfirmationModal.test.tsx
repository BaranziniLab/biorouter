import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CONFIRMATION_DETAILS_LABEL, ConfirmationModal } from './ConfirmationModal';
import { uiCopy } from './copy';

afterEach(cleanup);

describe('ConfirmationModal', () => {
  it('blocks every dismissal path while submitting', async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();

    render(
      <ConfirmationModal
        isOpen
        title="Delete item"
        message="This cannot be undone."
        onConfirm={() => {}}
        onCancel={onCancel}
        isSubmitting
      />
    );

    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();
    await user.keyboard('{Escape}');

    const overlay = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]');
    expect(overlay).not.toBeNull();
    fireEvent.pointerDown(overlay!);

    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(onCancel).not.toHaveBeenCalled();
  });

  it('is the shared dialog anatomy: ModalShell sm, pinned at the top, the header rule (V4-10, V1-L5)', () => {
    render(
      <ConfirmationModal
        isOpen
        title="Delete chat?"
        message="This removes it from every window."
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        onConfirm={() => {}}
        onCancel={() => {}}
      />
    );
    const dialog = screen.getByRole('dialog', { name: 'Delete chat?' });
    expect(dialog).toHaveAttribute('data-size', 'sm');
    expect(dialog).toHaveAttribute('data-anchor', 'top');
    // The message is the description, read in body text rather than a 12px subtitle.
    expect(dialog).toHaveAccessibleDescription('This removes it from every window.');
    expect(screen.getByText('This removes it from every window.')).toHaveClass('text-body');
    const header = screen.getByRole('heading', { name: 'Delete chat?' }).parentElement!;
    expect(header).toHaveClass('border-b');
    // Cancel first, the action last, at the trailing edge.
    const buttons = screen
      .getAllByRole('button')
      .map((button) => button.textContent)
      .filter((text) => text !== 'Close');
    expect(buttons).toEqual(['Cancel', 'Delete']);
  });

  it('cancels on Escape and says the action is running while it runs', async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    const { rerender } = render(
      <ConfirmationModal
        isOpen
        title="Reset"
        message="Start over."
        onConfirm={() => {}}
        onCancel={onCancel}
      />
    );
    await user.keyboard('{Escape}');
    expect(onCancel).toHaveBeenCalledTimes(1);
    rerender(
      <ConfirmationModal
        isOpen
        isSubmitting
        title="Reset"
        message="Start over."
        onConfirm={() => {}}
        onCancel={onCancel}
      />
    );
    expect(screen.getByRole('button', { name: uiCopy.processing })).toBeDisabled();
  });

  // Owner, 2026-11-13: the message is one sentence; the rest folds under it.
  it('says one sentence and folds `details` under it, outside the description', () => {
    render(
      <ConfirmationModal
        isOpen
        title="Load it anyway?"
        message="It may run slowly here."
        details="On Apple Silicon, unified memory is the relevant budget."
        detailsLabel="Why?"
        onConfirm={() => {}}
        onCancel={() => {}}
      />
    );
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAccessibleDescription('It may run slowly here.');
    expect(screen.queryByText(/unified memory/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Why?' }));
    expect(screen.getByText(/unified memory/)).toBeInTheDocument();
    expect(CONFIRMATION_DETAILS_LABEL).toBe('Details');
  });
});
