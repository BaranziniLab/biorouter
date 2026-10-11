import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { KBManagerDialog } from './KBManagerDialog';
import { knowledgeCopy } from '../copy';

/**
 * The list-and-search half of the old `KBSelectorPalette.test.tsx`, ported when
 * §4.2 split that file. The DR-12 primary-repair cases went the other way, onto
 * `KBSelectorMenu.test.tsx`.
 */

const mocks = vi.hoisted(() => ({
  setPrimaryKbId: vi.fn(),
  toggleKbHidden: vi.fn(),
  refresh: vi.fn().mockResolvedValue(undefined),
  onOpenChange: vi.fn(),
  create: vi.fn(),
}));

const state = vi.hoisted(() => ({
  primaryKbId: 'alpha' as string | null,
  bases: [
    { id: 'alpha', name: 'Alpha', color: '#cf6d47', tier: 'private' },
    { id: 'beta', name: 'Beta', color: '#b85a32', tier: 'public' },
  ],
}));

vi.mock('../KnowledgeContext', () => ({
  useKnowledge: () => ({
    bases: state.bases,
    loading: false,
    primaryKbId: state.primaryKbId,
    hiddenKbIds: ['beta'],
    refresh: mocks.refresh,
    setPrimaryKbId: mocks.setPrimaryKbId,
    toggleKbHidden: mocks.toggleKbHidden,
  }),
}));

vi.mock('../hooks/useKnowledgeBases', () => ({
  useKnowledgeBases: () => ({
    create: mocks.create,
    exportArchive: vi.fn(),
    importArchive: vi.fn(),
    remove: vi.fn(),
    rename: vi.fn(),
  }),
}));

beforeAll(() => {
  // The dialog pulls in Radix primitives that observe their trigger.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  vi.clearAllMocks();
  state.primaryKbId = 'alpha';
  state.bases = [
    { id: 'alpha', name: 'Alpha', color: '#cf6d47', tier: 'private' },
    { id: 'beta', name: 'Beta', color: '#b85a32', tier: 'public' },
  ];
  mocks.create.mockResolvedValue({ id: 'first-base' });
});

function open() {
  return render(<KBManagerDialog open onOpenChange={mocks.onOpenChange} />);
}

describe('KBManagerDialog', () => {
  it('lets a zero-base install create its first OKF base through one usable dialog', async () => {
    state.primaryKbId = null;
    state.bases = [];
    render(<KBManagerDialog open startInCreate onOpenChange={mocks.onOpenChange} />);

    expect(screen.getByRole('dialog', { name: 'Create knowledge base' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Knowledge bases' })).toBeNull();

    await userEvent.type(screen.getByTestId('knowledge-format-name'), 'First Base');
    await userEvent.click(screen.getByTestId('knowledge-format-submit'));

    expect(mocks.create).toHaveBeenCalledWith('first-base', 'First Base', { format: 'okf' });
    expect(mocks.setPrimaryKbId).toHaveBeenCalledWith('first-base');
    expect(mocks.refresh).toHaveBeenCalled();
  });

  // Two states per row, never three. Under the merged model membership and the
  // primary are the only two things a base can be, and the row body is the
  // "make primary" affordance.
  //
  // Spec 2.6: a switch is named by its row's visible label, the base's name, so
  // the name never depends on state and never reads "Toggle …" or "Include …".
  it('offers exactly one membership switch per row', () => {
    open();
    expect(screen.getByRole('switch', { name: 'Alpha' })).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Beta' })).toBeInTheDocument();
    expect(screen.getAllByRole('switch')).toHaveLength(2);
  });

  // Spec 3.12: one sentence under the title (it is the dialog's description),
  // the rest in an InfoTip, no footer Tip, and no mono id on any row: the id is
  // behind "Copy ID" in the row's menu.
  it('says one sentence, keeps ids off the rows, and copies an id from the menu', async () => {
    open();
    const dialog = screen.getByRole('dialog', { name: knowledgeCopy.manager.title });
    expect(dialog).toHaveAccessibleDescription(
      expect.stringContaining(knowledgeCopy.manager.subtitle)
    );
    expect(
      screen.getByRole('button', { name: `About ${knowledgeCopy.manager.infoLabel}` })
    ).toHaveAccessibleDescription(knowledgeCopy.manager.info);
    expect(screen.queryByText(/^Tip:/)).toBeNull();

    const row = screen.getByRole('option', { name: /Alpha/ }).closest('.biorouter-list-row')!;
    expect(within(row as HTMLElement).queryByText('alpha')).toBeNull();

    await userEvent.click(screen.getByLabelText('More actions for Alpha'));
    expect(
      await screen.findByRole('menuitem', { name: knowledgeCopy.manager.copyId })
    ).toBeInTheDocument();
  });

  // V3-09: the row's ⋯ is a RowActions cluster, so it rests hidden and shows on hover, focus or an
  // open menu like every other list row (the rule is `.br-row-actions` in main.css; jsdom draws no
  // CSS, so this pins the markup the rule hangs on). The switch is state and stays drawn.
  it('draws the row ⋯ as a RowActions cluster, with the switch outside it', () => {
    open();
    const row = screen.getByRole('option', { name: /Alpha/ }).closest('.biorouter-list-row')!;
    const more = within(row as HTMLElement).getByLabelText('More actions for Alpha');
    expect(more.closest('[data-slot="row-actions"]')).not.toBeNull();
    expect(more.closest('.br-row-actions')).not.toBeNull();
    expect(
      within(row as HTMLElement)
        .getByRole('switch')
        .closest('.br-row-actions')
    ).toBeNull();
  });

  // V3-21: an item that opens a dialog or asks first takes the ellipsis, as Skills' Delete… does.
  it('marks the items that open a dialog with an ellipsis', async () => {
    open();
    await userEvent.click(screen.getByLabelText('More actions for Alpha'));
    expect(await screen.findByRole('menuitem', { name: 'Export as .brkb…' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Delete knowledge base…' })).toBeInTheDocument();
    expect(
      screen.getByRole('menuitem', { name: knowledgeCopy.manager.rename })
    ).toBeInTheDocument();
  });

  // Picking a primary used to close the palette, which made the selector feel
  // like a radio group over a single-active model. The MANAGER is a place you
  // stay — the picker (§4.1) is the surface that closes.
  it('makes a base primary without closing the dialog', async () => {
    open();
    await userEvent.click(screen.getByText('Beta'));
    expect(mocks.setPrimaryKbId).toHaveBeenCalledWith('beta');
    expect(mocks.onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it('marks the primary', () => {
    open();
    expect(screen.getByText('Primary')).toBeInTheDocument();
  });

  // Issue #56 DR-18. The manager is a switch too, so the tier has to be legible
  // BEFORE the user switches — a badge only on the base you already chose tells
  // you what you did, not what you are about to do.
  it('the tier is visible before the user switches to a base', () => {
    open();
    const priv = screen.getByRole('option', { name: /Alpha/ });
    const pub = screen.getByRole('option', { name: /Beta/ });
    expect(within(priv).getByText(/Private/)).toBeInTheDocument();
    // Public is the ordinary state and carries no badge, so the private one
    // reads as a marking rather than as a label everything wears.
    expect(within(pub).queryByText(/Private/)).toBeNull();
  });

  // §4.12 #9. The hand-rolled centred sentence this replaced is one of the
  // seven that made the section read thinner than its siblings.
  it('answers a search that matches nothing with the EmptyState primitive', async () => {
    open();
    await userEvent.type(screen.getByTestId('knowledge-kb-search'), 'nothing-matches-this');
    expect(screen.getByText('No knowledge bases match')).toBeInTheDocument();
    expect(screen.queryByRole('option')).toBeNull();
  });

  // ROWS-3: a destructive control never sits visible in a hover cluster. Delete
  // lives behind the row's one `⋯` overflow.
  it('keeps delete behind the row overflow', () => {
    open();
    expect(screen.queryByLabelText('Delete Alpha')).toBeNull();
    expect(screen.getByLabelText('More actions for Alpha')).toBeInTheDocument();
  });

  /**
   * R-07. A row packed 5 focusable controls and up to 12 visual objects into a
   * 40 x 582px box, reserving a fixed 160px cluster at its end — a 40px switch,
   * three 32px icon buttons and three 8px gaps — before the name column got
   * anything. Export and rename join the menu that was already there.
   */
  it('reserves ONE overflow control per row, not three icon buttons', () => {
    open();
    const row = screen.getByRole('option', { name: /Alpha/ }).closest('.biorouter-list-row')!;
    expect(within(row as HTMLElement).queryByLabelText(/Export Alpha/)).toBeNull();
    expect(within(row as HTMLElement).queryByLabelText(/Rename Alpha/)).toBeNull();
    // The switch and the one overflow are what remain.
    expect(within(row as HTMLElement).getByRole('switch')).toBeInTheDocument();
    expect(within(row as HTMLElement).getByLabelText('More actions for Alpha')).toBeInTheDocument();
  });

  /**
   * Two facts were each drawn twice: membership as a switch AND a
   * "Not in this chat" badge, primary as a `PRIMARY` badge AND a row-wide
   * `tint-selected` fill. The badge survives and the tint does not — D-15 makes
   * focus a SURFACE shift, so a row tint competes with the focus surface of
   * every control inside the row.
   */
  it('states membership and primary exactly once each', () => {
    open();
    expect(screen.queryByText('Not in this chat')).toBeNull();
    const primary = screen.getByRole('option', { selected: true }).closest('.biorouter-list-row')!;
    expect(within(primary as HTMLElement).getByText(/Primary/i)).toBeInTheDocument();
    expect((primary as HTMLElement).className).not.toContain('tint-selected');
  });
});
