import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { KBSelectorMenu } from './KBSelectorMenu';
import { knowledgeCopy } from '../copy';
import type { Manifest } from '../../../api/types.gen';

/**
 * The DR-12 primary-repair contract, ported wholesale from
 * `KBSelectorPalette.test.tsx` when §4.2's file split retired that file.
 *
 * ⚠ These cases are PORTED, not re-derived. Six of the old palette's cases
 * encoded "following the default again", which is the behaviour that makes a
 * chat whose pinned base was deleted recoverable; deleting the file and writing
 * fresh tests for the two new ones would have dropped that contract silently.
 */

const mocks = vi.hoisted(() => ({
  setPrimaryKbId: vi.fn(),
  followDefaultPrimary: vi.fn(),
  refreshDefaultPrimary: vi.fn().mockResolvedValue(undefined),
  refresh: vi.fn().mockResolvedValue(undefined),
  onClose: vi.fn(),
  onManage: vi.fn(),
  onCreate: vi.fn(),
}));

const BASES = vi.hoisted(() => [
  { id: 'alpha', name: 'Alpha', color: '#cf6d47', tier: 'private' },
  { id: 'beta', name: 'Beta', color: '#b85a32', tier: 'public' },
]);

const state = vi.hoisted(() => ({
  primaryKbId: 'alpha' as string | null,
  defaultPrimaryKb: null as Partial<Manifest> | null,
  canFollowDefaultPrimary: false,
  /** The list is a fetch. It is EMPTY while that fetch is in flight. */
  visibleBases: BASES as Array<Record<string, unknown>>,
}));

vi.mock('../KnowledgeContext', () => ({
  useKnowledge: () => ({
    visibleBases: state.visibleBases,
    primaryKbId: state.primaryKbId,
    defaultPrimaryKb: state.defaultPrimaryKb,
    canFollowDefaultPrimary: state.canFollowDefaultPrimary,
    followDefaultPrimary: mocks.followDefaultPrimary,
    refreshDefaultPrimary: mocks.refreshDefaultPrimary,
    refresh: mocks.refresh,
    setPrimaryKbId: mocks.setPrimaryKbId,
  }),
}));

function renderMenu() {
  return render(
    <KBSelectorMenu onClose={mocks.onClose} onManage={mocks.onManage} onCreate={mocks.onCreate} />
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  state.primaryKbId = 'alpha';
  state.defaultPrimaryKb = null;
  state.canFollowDefaultPrimary = false;
  state.visibleBases = BASES;
});

/** The row `aria-activedescendant` names, by its visible text. */
/**
 * Which row the highlight is on, by NAME.
 *
 * ⚠ **It reads `[data-row-name]`, not the row's `textContent`, and the
 * difference is the whole point.** A row is a name plus however many badges it
 * has grown — a format badge, a privacy lock, the primary check — so
 * `textContent` answers "what does this row say" when the question is "which row
 * is this". It read `'Alpha'` until the format badge landed and then read
 * `'AlphaLegacy'`, failing a test about the HIGHLIGHT for a reason that had
 * nothing to do with the highlight. Every row carries the marker, action rows
 * included, so the assertion below still distinguishes a base row from
 * "Manage bases…".
 */
function highlighted(): string | null {
  const id = screen.getByRole('combobox').getAttribute('aria-activedescendant');
  if (!id) return null;
  const row = document.getElementById(id);
  return row?.querySelector('[data-row-name]')?.textContent ?? row?.textContent ?? null;
}

describe('KBSelectorMenu', () => {
  // The picker is the "which base am I pointed at" surface, so picking one is a
  // navigation and the popover closes behind it. (The manager, which is where
  // the collection is edited, deliberately does NOT close — see its own test.)
  it('makes a base primary and closes', async () => {
    renderMenu();
    await userEvent.click(screen.getByText('Beta'));
    expect(mocks.setPrimaryKbId).toHaveBeenCalledWith('beta');
    expect(mocks.onClose).toHaveBeenCalledTimes(1);
  });

  // Issue #56 DR-18. The picker is the switch, so the tier has to be legible
  // BEFORE the user switches — Public is quiet so the private marking stays a
  // marking.
  it('marks a private base before the user switches to it', () => {
    renderMenu();
    const priv = screen.getByRole('option', { name: /Alpha/ });
    const pub = screen.getByRole('option', { name: /Beta/ });
    expect(within(priv).queryByTestId('privacy-badge')).not.toBeNull();
    expect(within(pub).queryByTestId('privacy-badge')).toBeNull();
  });

  // The picker opens over a list that is still being read — `refresh()` runs on
  // mount — so for a frame the only rows are "Manage bases…" and "Create
  // knowledge base…". A highlight seeded then and kept because it still exists
  // leaves Enter, as the first keystroke on an untouched picker, opening the
  // manager instead of committing a base.
  it('does not leave the highlight on an action row when the bases arrive late', async () => {
    state.visibleBases = [];
    const { rerender } = renderMenu();
    await waitFor(() => expect(highlighted()).toBe('Manage bases…'));

    state.visibleBases = BASES;
    rerender(
      <KBSelectorMenu onClose={mocks.onClose} onManage={mocks.onManage} onCreate={mocks.onCreate} />
    );
    await waitFor(() => expect(highlighted()).toBe('Alpha'));

    screen.getByRole('combobox').focus();
    await userEvent.keyboard('{Enter}');
    expect(mocks.onManage).not.toHaveBeenCalled();
    expect(mocks.setPrimaryKbId).toHaveBeenCalledWith('alpha');
  });

  it('routes management and creation out of the picker', async () => {
    renderMenu();
    await userEvent.click(screen.getByTestId('knowledge-kb-open-manager'));
    expect(mocks.onManage).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByTestId('knowledge-kb-open-create'));
    expect(mocks.onCreate).toHaveBeenCalledTimes(1);
  });

  describe('following the default again', () => {
    // The machine-wide default is not part of the base list, so the picker has
    // to ask for it — and it asks when it opens, so a default changed in
    // another chat is not stale here.
    it('reads the machine-wide default when it opens', () => {
      renderMenu();
      expect(mocks.refreshDefaultPrimary).toHaveBeenCalled();
    });

    // A chat that is already following the default has nothing to inherit, so
    // the picker says nothing about it.
    it('says nothing to a chat that has not overridden the default', () => {
      renderMenu();
      expect(screen.queryByTestId('knowledge-kb-follow-default')).not.toBeInTheDocument();
    });

    // The state deleting a chat's pinned base leaves behind. Naming the base
    // makes the outcome of the click concrete rather than systemic.
    it('offers a chat with no primary the way back to the default', async () => {
      state.primaryKbId = null;
      state.defaultPrimaryKb = { id: 'alpha', name: 'Alpha' };
      state.canFollowDefaultPrimary = true;
      renderMenu();

      // Spec 3.12: one line, "Not following your default", and the way back.
      // What the click does is the button's description (and its Tooltip).
      expect(screen.getByText(knowledgeCopy.selector.notFollowing)).toBeInTheDocument();
      const follow = screen.getByTestId('knowledge-kb-follow-default');
      expect(follow).toHaveTextContent('Use Alpha');
      expect(follow).toHaveAccessibleDescription(knowledgeCopy.selector.useDefaultHint('Alpha'));
      expect(follow).not.toHaveAttribute('title');

      await userEvent.click(follow);
      expect(mocks.followDefaultPrimary).toHaveBeenCalledTimes(1);
      expect(mocks.onClose).not.toHaveBeenCalled();
    });

    it('offers a chat that pinned its own primary the way back too', () => {
      state.primaryKbId = 'beta';
      state.defaultPrimaryKb = { id: 'alpha', name: 'Alpha' };
      state.canFollowDefaultPrimary = true;
      renderMenu();

      expect(screen.getByText(knowledgeCopy.selector.notFollowing)).toBeInTheDocument();
      expect(screen.getByTestId('knowledge-kb-follow-default')).toBeInTheDocument();
    });

    // The way back is one control for the whole chat, not a third state on
    // every row: the row stays "pick this base", and nothing else.
    it('does not add a control to any row', () => {
      state.primaryKbId = null;
      state.defaultPrimaryKb = { id: 'alpha', name: 'Alpha' };
      state.canFollowDefaultPrimary = true;
      renderMenu();

      expect(screen.getAllByTestId('knowledge-kb-follow-default')).toHaveLength(1);
      for (const row of screen.getAllByRole('option')) {
        expect(within(row).queryByRole('button')).toBeNull();
      }
    });
  });
});
