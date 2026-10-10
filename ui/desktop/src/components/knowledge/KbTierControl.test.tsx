import { cleanup, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { KbTierControl } from './KbTierControl';
import { knowledgeCopy } from './copy';

/**
 * Spec 3.12 moved the TRIGGER, and only the trigger: the tier is a badge-button
 * in the band (Crew's ClassificationBadge) whose menu holds the one tier action.
 * Everything after the menu item (the typed id, the counts, `onSetTier`) is the
 * frozen confirmation, and the cases below assert it unchanged.
 */
async function chooseTierAction(
  user: ReturnType<typeof userEvent.setup>,
  action: RegExp
): Promise<void> {
  await user.click(screen.getByTestId('kb-tier-chip'));
  await user.click(await screen.findByRole('menuitem', { name: action }));
}

const onSetTier = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => cleanup());

describe('KbTierControl', () => {
  /**
   * The lowering direction, and why the confirmation counts instead of asking.
   * Declassifying a session exposes one transcript the user is looking at;
   * publicizing a base exposes everything every private model ever wrote into
   * it, to every public model, from the next tool call onward.
   */
  it('publicizing names the blast radius and the phrase gates the button', async () => {
    const user = userEvent.setup();
    render(
      <KbTierControl
        kb={{
          id: 'omop',
          name: 'OMOP Cohort',
          tier: 'private',
          pageCount: 214,
          rawSourceCount: 37,
        }}
        onSetTier={onSetTier}
      />
    );
    await chooseTierAction(user, /Make public/);

    // Concrete, not "are you sure". A generic ConfirmationModal gives you the
    // latter for free, which is exactly the wrong implementation this rejects.
    expect(screen.getByText(/214 pages/)).toBeInTheDocument();
    expect(screen.getByText(/37 raw sources/)).toBeInTheDocument();
    expect(
      screen.getByText(/cannot be undone for content that has already been read/i)
    ).toBeInTheDocument();

    const confirm = screen.getByRole('button', { name: /Make public/ });
    expect(confirm).toBeDisabled();

    // The NAME is not the phrase. Default knowledge-base names duplicate the way
    // session names do — `default`, `Notes`, `My Knowledge Base` — so the id is
    // what forces the user to check WHICH base they are releasing.
    await user.type(screen.getByRole('textbox'), 'OMOP Cohort');
    expect(confirm).toBeDisabled();

    await user.clear(screen.getByRole('textbox'));
    await user.type(screen.getByRole('textbox'), 'omop'); // the id is
    expect(confirm).toBeEnabled();

    await user.click(confirm);
    expect(onSetTier).toHaveBeenCalledWith('omop', 'public');
  });

  /**
   * Nothing is disclosed by going the other way, so a phrase here would be
   * theatre and an undo timer a false promise about something already read.
   */
  it('privatizing is single-click and discloses nothing', async () => {
    const user = userEvent.setup();
    render(
      <KbTierControl
        kb={{ id: 'notes', name: 'Notes', tier: 'public', pageCount: 9 }}
        onSetTier={onSetTier}
      />
    );
    await chooseTierAction(user, /Make private/);
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(onSetTier).toHaveBeenCalledWith('notes', 'private');
  });

  /**
   * The counts come from the daemon, which reads them off the tree. When they
   * have not arrived the dialog must not invent a number — a "0 pages" release
   * of a 214-page base is the one sentence a user would act on wrongly.
   */
  it('says what it does not know rather than counting to zero', async () => {
    const user = userEvent.setup();
    render(
      <KbTierControl
        kb={{ id: 'omop', name: 'OMOP Cohort', tier: 'private' }}
        onSetTier={onSetTier}
      />
    );
    await chooseTierAction(user, /Make public/);
    expect(screen.queryByText(/0 pages/)).toBeNull();
    expect(screen.getByText(/every page and raw source/i)).toBeInTheDocument();
  });

  /**
   * The badge is a real control, so a keyboard user reaches the explanation a
   * mouse user gets on hover: its name carries the whole sentence (Crew UXN-15),
   * and it carries no native `title`, which no keyboard could reach.
   */
  it('names the tier and what it means, and is a button', () => {
    render(
      <KbTierControl
        kb={{ id: 'omop', name: 'OMOP Cohort', tier: 'private' }}
        onSetTier={onSetTier}
      />
    );
    const chip = screen.getByRole('button', {
      name: knowledgeCopy.tier.name('Private', knowledgeCopy.tier.privateHint, false),
    });
    expect(chip).toBe(screen.getByTestId('kb-tier-chip'));
    expect(chip).not.toHaveAttribute('title');
  });

  /** The chip itself, which is how the tier is visible without opening anything. */
  it('shows the tier on the base it is attached to', () => {
    const { rerender } = render(
      <KbTierControl
        kb={{ id: 'omop', name: 'OMOP Cohort', tier: 'private' }}
        onSetTier={onSetTier}
      />
    );
    expect(screen.getByTestId('kb-tier-chip')).toHaveTextContent(/private/i);
    rerender(
      <KbTierControl kb={{ id: 'notes', name: 'Notes', tier: 'public' }} onSetTier={onSetTier} />
    );
    expect(screen.getByTestId('kb-tier-chip')).toHaveTextContent(/public/i);
  });
});
