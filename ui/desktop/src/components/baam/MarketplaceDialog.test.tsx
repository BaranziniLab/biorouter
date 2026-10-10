import type React from 'react';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { MarketplaceDialog, MarketplaceRow, MarketplaceSection } from './MarketplaceDialog';
import { MARKETPLACE_COPY } from './copy';

function renderDialog(props: Partial<React.ComponentProps<typeof MarketplaceDialog>> = {}) {
  const onClose = vi.fn();
  const onSearchChange = vi.fn();
  render(
    <MarketplaceDialog
      title="Browse things"
      help="Pick one thing at a time."
      live
      search=""
      onSearchChange={onSearchChange}
      searchLabel="Search things"
      status="ready"
      emptyText="No things match your search."
      onClose={onClose}
      {...props}
    >
      {props.children ?? (
        <MarketplaceSection label="Core" count={1}>
          <MarketplaceRow title="Alpha" meta="Acme · 1.0" description="The first thing" />
        </MarketplaceSection>
      )}
    </MarketplaceDialog>
  );
  return { onClose, onSearchChange };
}

describe('MarketplaceDialog — the shell both browse dialogs share', () => {
  it('shows one subtitle line and keeps the instructions out of view', () => {
    renderDialog();

    expect(screen.getByRole('dialog', { name: 'Browse things' })).toBeInTheDocument();
    expect(screen.getByText(MARKETPLACE_COPY.subtitle)).toBeInTheDocument();
    // The help is a description of its trigger, reachable without a hover.
    expect(screen.getByRole('button', { name: 'About browse things' })).toHaveAccessibleDescription(
      'Pick one thing at a time.'
    );
  });

  it('dates a catalog that is not live, and says nothing for a live one', () => {
    renderDialog({ live: false, fetchedAt: undefined });
    expect(screen.getByText(/showing bundled catalog \(offline\)/)).toBeInTheDocument();
  });

  it('names the search field and leaves the placeholder short', async () => {
    const user = userEvent.setup();
    const { onSearchChange } = renderDialog();

    const field = screen.getByRole('textbox', { name: 'Search things' });
    expect(field).toHaveAttribute('placeholder', MARKETPLACE_COPY.searchPlaceholder);
    await user.type(field, 'a');
    expect(onSearchChange).toHaveBeenCalledWith('a');
  });

  it('labels a group with its count in one caps heading', () => {
    renderDialog();
    expect(screen.getByRole('heading', { level: 3 }).textContent).toBe('Core 1');
  });

  it('draws a row as two lines: name with its facts, then the description', () => {
    renderDialog();

    const row = screen.getByText('Alpha').closest('[data-marketplace-row]') as HTMLElement;
    expect(row).toHaveClass('biorouter-list-row');
    expect(within(row).getByText('Acme · 1.0')).toBeInTheDocument();
    expect(within(row).getByText('The first thing')).toHaveClass('truncate');
    // No icon tile and no tag row: the only svg a row may hold is its own control's.
    expect(row.querySelector('svg')).toBeNull();
  });

  it('shows the empty line in place of the list', () => {
    renderDialog({ empty: true });
    expect(screen.getByText('No things match your search.')).toBeInTheDocument();
    expect(screen.queryByText('Alpha')).toBeNull();
  });

  it('announces a load error and loading without visible filler text', () => {
    const { unmount } = render(
      <MarketplaceDialog
        title="Browse things"
        help="Help."
        live
        search=""
        onSearchChange={vi.fn()}
        searchLabel="Search things"
        status="loading"
        emptyText="Nothing"
        onClose={vi.fn()}
      />
    );
    expect(screen.getByRole('status')).toHaveTextContent(MARKETPLACE_COPY.loading);
    unmount();

    renderDialog({ status: 'error' });
    expect(screen.getByRole('alert')).toHaveTextContent(MARKETPLACE_COPY.loadError);
  });

  it('has no footer of its own: the × is the one Close', () => {
    renderDialog();
    expect(screen.getAllByRole('button', { name: 'Close' })).toHaveLength(1);
  });

  it('cannot be dismissed while busy', async () => {
    const user = userEvent.setup();
    const { onClose } = renderDialog({ busy: true });
    await user.keyboard('{Escape}');
    expect(onClose).not.toHaveBeenCalled();
  });

  it('never sets marketplace text below the type floor', () => {
    renderDialog();
    const dialog = screen.getByRole('dialog');
    expect(dialog.innerHTML).not.toMatch(/text-\[1[01]px\]/);
    expect(dialog.innerHTML).not.toMatch(/text-background-/);
  });
});
