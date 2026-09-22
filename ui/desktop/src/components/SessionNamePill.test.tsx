import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { SessionNamePill } from './SessionNamePill';

describe('session title controls', () => {
  it('keeps the main chat title read-only until Rename is selected from the title menu', async () => {
    const user = userEvent.setup();

    render(<SessionNamePill name="Weather summary website" onRename={vi.fn()} />);

    await user.click(screen.getByText('Weather summary website'));
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /chat title actions/i }));
    await user.click(await screen.findByRole('menuitem', { name: 'Rename' }));
    expect(screen.getByRole('textbox')).toHaveValue('Weather summary website');
  });

  it('does not rename the main chat title from pointer down on the title text', () => {
    render(<SessionNamePill name="Weather summary website" onRename={vi.fn()} />);

    fireEvent.pointerDown(screen.getByText('Weather summary website'));

    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
  });

  it('offers Diverge from the main chat title menu when a response exists', async () => {
    const user = userEvent.setup();
    const onDiverge = vi.fn();

    render(
      <SessionNamePill
        name="Weather summary website"
        onRename={vi.fn()}
        onDiverge={onDiverge}
        canDiverge
      />
    );

    await user.click(screen.getByRole('button', { name: /chat title actions/i }));
    await user.click(await screen.findByRole('menuitem', { name: 'Diverge' }));

    expect(onDiverge).toHaveBeenCalledTimes(1);
  });

  // Issue #56 §12.1, the paired half of
  // `SessionListView declassification entry point`. This menu is the obvious
  // slot for the action and the one the design forbids: it is rename/diverge,
  // one careless click from the chat title, and lowering a chat's tier is a
  // decision that owes its own confirmed surface. The pill shows the tier as a
  // LABEL and offers no control over it.
  it('never offers declassification from the chat title menu', async () => {
    const user = userEvent.setup();

    render(<SessionNamePill name="x" privacyTier="private" onRename={vi.fn()} />);

    // Asserted with the menu OPEN: a closed Radix menu renders no items, so the
    // same expectation on an unopened menu would pass against an implementation
    // that does put the action here.
    await user.click(screen.getByRole('button', { name: /chat title actions/i }));
    await screen.findByRole('menuitem', { name: 'Rename' });
    expect(screen.queryByText(/Make this chat public/)).toBeNull();
  });

  /**
   * Issue #56, DR-26 — the third axis beside the tier, on the surface a user
   * looks at to know which chat they are in.
   *
   * ⚠ The two badges answer different questions about different objects: the
   * tier is the SESSION's ratcheted classification, the affiliation is the BOUND
   * MODEL's institution. They are rendered together because the user has both
   * questions at once, and they arrive as separate props because nothing derives
   * one from the other.
   */
  it('shows the bound model’s institution beside the chat’s tier', () => {
    render(
      <SessionNamePill
        name="Cohort query"
        privacyTier="private"
        affiliation={{ kind: 'institutions', institutions: [{ id: 'ucsf', display_name: 'UCSF' }] }}
        onRename={vi.fn()}
      />
    );

    expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'private');
    const affiliation = screen.getByTestId('affiliation-badge');
    expect(affiliation).toHaveAttribute('data-affiliation', 'institutions');
    expect(affiliation).toHaveTextContent('UCSF');
  });

  /**
   * ⚠ **A public model has no affiliation, and an unresolved one has no claim to
   * make.** Both render nothing — a chip saying "no institution" would read as a
   * constraint, and one guessed before the provider row loads would be a claim
   * about a transcript's compliance made from no evidence.
   */
  it('shows no affiliation badge when there is nothing to say', () => {
    render(<SessionNamePill name="Cohort query" privacyTier="private" onRename={vi.fn()} />);
    expect(screen.getByTestId('privacy-badge')).toBeInTheDocument();
    expect(screen.queryByTestId('affiliation-badge')).toBeNull();
  });
});
