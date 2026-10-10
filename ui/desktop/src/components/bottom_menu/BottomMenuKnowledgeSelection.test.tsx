import * as React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { KnowledgePanel, useKnowledgePicker } from './BottomMenuKnowledgeSelection';
import { KNOWLEDGE_COPY, TOOLS_COPY } from './copy';

/**
 * The knowledge list as the Tools chip holds it (spec 3.7). The harness's
 * button stands in for the chip and states the hook's count.
 */
function BottomMenuKnowledgeSelection() {
  const [open, setOpen] = React.useState(false);
  const picker = useKnowledgePicker({ open });
  return (
    <>
      <button
        type="button"
        aria-label={`Manage knowledge bases (${picker.visibleCount} visible)`}
        onClick={() => setOpen((current) => !current)}
      />
      {open && <KnowledgePanel picker={picker} inChat />}
    </>
  );
}

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

  it('uses the one picker shell: a search field, a scope line and option rows', async () => {
    render(<BottomMenuKnowledgeSelection />);

    const trigger = screen.getByRole('button', { name: 'Manage knowledge bases (2 visible)' });
    fireEvent.click(trigger);

    const list = await screen.findByRole('listbox', { name: KNOWLEDGE_COPY.listLabel });
    expect(list).toHaveAttribute('aria-multiselectable', 'true');
    const field = screen.getByPlaceholderText(KNOWLEDGE_COPY.search);
    expect(field).toHaveAttribute('role', 'combobox');
    expect(screen.getByText(TOOLS_COPY.scope.thisChat)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: `About ${TOOLS_COPY.scope.about}` })
    ).toHaveAccessibleDescription(KNOWLEDGE_COPY.helpChat);

    // One verb pair per popup (#82): each row says VISIBILITY in its name.
    const soul = screen.getByRole('option', { name: 'Soul, visible to this chat' });
    expect(soul).toHaveAttribute('aria-selected', 'true');
    // No size class on the row label: the shared menu row owns it.
    expect(screen.getByText('Soul')).not.toHaveClass('text-sm');

    fireEvent.change(field, { target: { value: 'brain' } });
    expect(screen.queryByText('Soul')).not.toBeInTheDocument();
    expect(screen.getByText('brainstorm')).toBeInTheDocument();
  });

  it('toggles a base with one write when its row is chosen', () => {
    render(<BottomMenuKnowledgeSelection />);
    fireEvent.click(screen.getByRole('button', { name: /Manage knowledge bases/ }));
    fireEvent.click(screen.getByRole('option', { name: /^brainstorm/ }));
    expect(mocks.toggleKbHidden).toHaveBeenCalledWith('brainstorm');
    expect(contextWrites()).toBe(1);
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

    fireEvent.click(screen.getByRole('button', { name: /Manage knowledge bases/ }));
    await screen.findByRole('listbox');
    fireEvent.change(screen.getByPlaceholderText(KNOWLEDGE_COPY.search), {
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

    fireEvent.click(screen.getByRole('button', { name: /Manage knowledge bases/ }));
    await screen.findByRole('listbox');
    fireEvent.change(screen.getByPlaceholderText(KNOWLEDGE_COPY.search), {
      target: { value: 'brain' },
    });

    await user.click(screen.getByRole('button', { name: 'Show all (2)' }));

    expect(mocks.state.hiddenKbIds).toEqual(['soul']);
    expect(contextWrites()).toBe(1);
  });
});
