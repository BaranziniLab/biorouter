import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { BottomMenuKnowledgeSelection } from './BottomMenuKnowledgeSelection';

const DEFAULT_BASES = [
  { id: 'soul', name: 'Soul' },
  { id: 'brainstorm', name: 'brainstorm' },
];

const mocks = vi.hoisted(() => ({
  toggleKbHidden: vi.fn(),
  setHiddenKbIds: vi.fn(),
  hideAllKnowledgeBases: vi.fn(),
  showAllKnowledgeBases: vi.fn(),
  refresh: vi.fn(async () => {}),
  state: { bases: [] as { id: string; name: string }[], hiddenKbIds: [] as string[] },
}));

// A faithful stand-in for the real context, because the defect is in *how* the
// context is driven. `toggleKbHidden` there derives its next set from the
// `hiddenKbIds` captured at render — not from a live value — so a caller that
// toggles several ids in one pass has every update but the last overwritten.
vi.mock('../knowledge/KnowledgeContext', () => ({
  useKnowledge: () => {
    const captured = mocks.state.hiddenKbIds;
    const commit = (ids: string[]) => {
      mocks.state.hiddenKbIds = Array.from(new Set(ids)).sort();
    };
    return {
      bases: mocks.state.bases,
      visibleBases: mocks.state.bases.filter((base) => !captured.includes(base.id)),
      hiddenKbIds: captured,
      toggleKbHidden: (id: string) => {
        mocks.toggleKbHidden(id);
        commit(captured.includes(id) ? captured.filter((held) => held !== id) : [...captured, id]);
      },
      setHiddenKbIds: (ids: string[]) => {
        mocks.setHiddenKbIds(ids);
        commit(ids);
      },
      hideAllKnowledgeBases: () => {
        mocks.hideAllKnowledgeBases();
        commit(mocks.state.bases.map((base) => base.id));
      },
      showAllKnowledgeBases: () => {
        mocks.showAllKnowledgeBases();
        commit([]);
      },
      refresh: mocks.refresh,
    };
  },
}));

beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

afterAll(() => {
  vi.unstubAllGlobals();
});

/** Every write the component asked the context to make during one gesture. */
function contextWrites() {
  return (
    mocks.toggleKbHidden.mock.calls.length +
    mocks.setHiddenKbIds.mock.calls.length +
    mocks.hideAllKnowledgeBases.mock.calls.length +
    mocks.showAllKnowledgeBases.mock.calls.length
  );
}

describe('BottomMenuKnowledgeSelection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.state.bases = [...DEFAULT_BASES];
    mocks.state.hiddenKbIds = [];
  });

  it('uses the same compact searchable menu layout as skills', async () => {
    const user = userEvent.setup();
    render(<BottomMenuKnowledgeSelection />);

    const trigger = screen.getByRole('button', { name: 'Manage knowledge bases (2 visible)' });
    expect(trigger).not.toHaveAttribute('title');

    await user.hover(trigger);
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Manage knowledge bases');
    expect(screen.queryByText('2 KBs visible')).not.toBeInTheDocument();

    fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false });

    const menu = await screen.findByRole('menu');
    // ⚠ Width only. `font-sans` used to be pinned here and on two of the five
    // popups, absent on the other three — a no-op today (`--font-sans` is
    // already the body font) but a latent divergence that read as deliberate
    // intent which was not there. The search input carries no type class
    // either: the Input primitive already owns it, and a call site restating
    // the size is how the four popups on this rail drifted to three different
    // row sizes in the first place.
    expect(menu).toHaveClass('w-64');
    expect(menu).not.toHaveClass('font-sans');
    expect(screen.getByPlaceholderText('Search knowledge bases...')).toHaveClass('h-8');
    expect(screen.queryByText('Chat knowledge discovery')).not.toBeInTheDocument();

    const soul = screen.getByRole('menuitemcheckbox', { name: /Soul/ });
    expect(soul).toHaveClass('px-2', 'py-2');
    // ⚠ No size class on the row label. It inherits `text-secondary` (13/18)
    // from the shared menu row, which is the declared token for menu rows.
    // Pinning `text-sm` here is what held this popup a step larger than the
    // Extensions popup beside it on the same rail.
    expect(screen.getByText('Soul')).toHaveClass('font-medium');
    expect(screen.getByText('Soul')).not.toHaveClass('text-sm');

    fireEvent.change(screen.getByPlaceholderText('Search knowledge bases...'), {
      target: { value: 'brain' },
    });
    expect(screen.queryByText('Soul')).not.toBeInTheDocument();
    expect(screen.getByText('brainstorm')).toBeInTheDocument();
  });

  // "Hide all (2)" under a search must hide both matches. Applying the bulk
  // toggle one id at a time loses every change but the last, because each of
  // those updates is derived from the same captured set — so the user sees one
  // of the two bases they just hid stay in the chat.
  it('hides every filtered base in one complete set', async () => {
    const user = userEvent.setup();
    mocks.state.bases = [
      { id: 'soul', name: 'Soul' },
      { id: 'brainstorm', name: 'brainstorm' },
      { id: 'brainmap', name: 'brainmap' },
    ];
    mocks.state.hiddenKbIds = ['soul'];
    render(<BottomMenuKnowledgeSelection />);

    fireEvent.pointerDown(screen.getByRole('button', { name: /Manage knowledge bases/ }), {
      button: 0,
      ctrlKey: false,
    });
    await screen.findByRole('menu');
    fireEvent.change(screen.getByPlaceholderText('Search knowledge bases...'), {
      target: { value: 'brain' },
    });

    await user.click(screen.getByRole('button', { name: 'Hide all (2)' }));

    expect(mocks.state.hiddenKbIds).toEqual(['brainmap', 'brainstorm', 'soul']);
    // One gesture, one write — each one is a daemon round-trip.
    expect(contextWrites()).toBe(1);
  });

  // The mirror case: showing the filtered matches must not disturb the bases
  // the search did not match.
  it('shows every filtered base in one complete set, leaving the rest alone', async () => {
    const user = userEvent.setup();
    mocks.state.bases = [
      { id: 'soul', name: 'Soul' },
      { id: 'brainstorm', name: 'brainstorm' },
      { id: 'brainmap', name: 'brainmap' },
    ];
    mocks.state.hiddenKbIds = ['brainmap', 'brainstorm', 'soul'];
    render(<BottomMenuKnowledgeSelection />);

    fireEvent.pointerDown(screen.getByRole('button', { name: /Manage knowledge bases/ }), {
      button: 0,
      ctrlKey: false,
    });
    await screen.findByRole('menu');
    fireEvent.change(screen.getByPlaceholderText('Search knowledge bases...'), {
      target: { value: 'brain' },
    });

    await user.click(screen.getByRole('button', { name: 'Show all (2)' }));

    expect(mocks.state.hiddenKbIds).toEqual(['soul']);
    expect(contextWrites()).toBe(1);
  });
});
