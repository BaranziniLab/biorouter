import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { KnowledgeProvider } from '../knowledge/KnowledgeContext';
import { BottomMenuKnowledgeSelection } from './BottomMenuKnowledgeSelection';

/**
 * QA 2026-09-10 F6: the chip read "Manage knowledge bases (2 visible)" over
 * three bases on disk, none hidden, until navigating to the Knowledge view and
 * back remounted it. The agent had created the third base from inside the chat
 * — from `execute_code`, where no knowledge tool call reaches the renderer — and
 * nothing told the chip.
 *
 * Unlike `BottomMenuKnowledgeSelection.test.tsx`, which stubs the context to
 * pin how the chip DRIVES it, this runs the real `KnowledgeProvider` over a
 * mocked daemon: the defect is in what the provider listens to.
 */

const mocks = vi.hoisted(() => ({
  listBases: vi.fn(),
  getActive: vi.fn(),
  setActive: vi.fn(),
}));

vi.mock('../../api', () => ({
  listBases: mocks.listBases,
  getActive: mocks.getActive,
  setActive: mocks.setActive,
}));

vi.mock('../../toasts', () => ({ toastError: vi.fn() }));

function base(id: string) {
  return { id, name: id, color: '#cf6d47', created_at: '', schema_version: 3, tier: 'public' };
}

/** What the daemon holds. Tests move it the way the agent would. */
const daemon = {
  bases: [base('soul'), base('brainstorm')],
  selection: {
    kb_ids: ['brainstorm', 'soul'],
    primary_kb: 'soul' as string | null,
    active_kb: 'soul' as string | null,
    hidden_kbs: [] as string[],
  },
};

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

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  daemon.bases = [base('soul'), base('brainstorm')];
  daemon.selection = {
    kb_ids: ['brainstorm', 'soul'],
    primary_kb: 'soul',
    active_kb: 'soul',
    hidden_kbs: [],
  };
  mocks.listBases.mockImplementation(() => Promise.resolve({ data: [...daemon.bases] }));
  mocks.getActive.mockImplementation(() => Promise.resolve({ data: { ...daemon.selection } }));
});

function renderChip() {
  return render(
    <KnowledgeProvider sessionId="chat-1">
      <BottomMenuKnowledgeSelection />
    </KnowledgeProvider>
  );
}

function chipLabel() {
  return screen.getByRole('button', { name: /Manage knowledge bases/ }).getAttribute('aria-label');
}

/** The app's end-of-turn signal, as `ChatStreamController` dispatches it. */
async function finishATurn() {
  await act(async () => {
    window.dispatchEvent(new CustomEvent('message-stream-finished'));
    await Promise.resolve();
  });
}

describe('BottomMenuKnowledgeSelection follows the daemon', () => {
  it('counts a base the agent created once the turn ends, without a remount', async () => {
    renderChip();
    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (2 visible)'));

    // The agent creates a base during the turn.
    daemon.bases = [...daemon.bases, base('f6-chip-probe')];
    daemon.selection = { ...daemon.selection, kb_ids: ['brainstorm', 'f6-chip-probe', 'soul'] };
    await finishATurn();

    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (3 visible)'));
  });

  it('drops a base the agent deleted once the turn ends', async () => {
    renderChip();
    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (2 visible)'));

    daemon.bases = [base('soul')];
    daemon.selection = { ...daemon.selection, kb_ids: ['soul'] };
    await finishATurn();

    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (1 visible)'));
  });

  // The selection rides with the list: the agent can hide a base from this chat
  // (`kb_set_active`, `workspace_set_tools`) in the same turn it creates one.
  it("follows the chat's own set when the agent changed it", async () => {
    renderChip();
    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (2 visible)'));

    daemon.selection = { ...daemon.selection, kb_ids: ['soul'], hidden_kbs: ['brainstorm'] };
    await finishATurn();

    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (1 visible)'));
  });

  // A base created from the CLI or another window ends no turn here. Opening
  // the chip is when the person asks, so it asks the daemon then.
  it('re-reads the bases when the chip is opened', async () => {
    renderChip();
    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (2 visible)'));

    daemon.bases = [...daemon.bases, base('from-the-cli')];
    daemon.selection = { ...daemon.selection, kb_ids: ['brainstorm', 'from-the-cli', 'soul'] };
    await userEvent.click(screen.getByRole('button', { name: /Manage knowledge bases/ }));

    await waitFor(() => expect(screen.getByText('from-the-cli')).toBeInTheDocument());
    expect(chipLabel()).toBe('Manage knowledge bases (3 visible)');
  });

  // pin-outranks-the-row, for knowledge: a refresh must never be what resets the
  // chat's primary. A list that arrives without the primary's base (stale, or a
  // daemon that filters it) used to trigger a durable `clear_primary` write.
  it('never writes the selection on its own initiative', async () => {
    renderChip();
    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (2 visible)'));

    daemon.bases = [base('brainstorm')];
    await finishATurn();
    await waitFor(() => expect(chipLabel()).toBe('Manage knowledge bases (1 visible)'));

    expect(mocks.setActive).not.toHaveBeenCalled();
    expect(localStorage.getItem('knowledge_active_kb:chat-1')).toBe('soul');
  });
});
