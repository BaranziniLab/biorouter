import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import KnowledgeView from './KnowledgeView';
import { knowledgeCopy } from './copy';

/**
 * Where focus goes when the knowledge-base manager (or the chooser it starts) is closed (V3-R7-3).
 *
 * This file renders the REAL selector trigger, picker menu, manager dialog and format chooser
 * under the real view: the defect lives in how those four hand focus to each other, and each of
 * them is mocked away in the other test files. Only the data layer and the heavy panels are
 * stubbed.
 *
 * The row that opens the manager is a cmdk row inside the picker's popover, and the popover
 * unmounts as the row acts. The dialog primitive's own return-to-opener therefore resolved a
 * detached row and gave nothing back: Escape from the manager left `document.activeElement` on
 * `<body>` for a keyboard user (and the chooser path ended in the manager, then `<body>`).
 */

const state = vi.hoisted(() => ({
  bases: [
    { id: 'omop', name: 'OMOP', color: '#cf6d47', tier: 'public' },
    { id: 'soul', name: 'Soul', color: '#b85a32', tier: 'public' },
  ] as Array<Record<string, unknown>>,
  primaryKbId: 'omop' as string | null,
}));

const mocks = vi.hoisted(() => ({
  refresh: vi.fn().mockResolvedValue(undefined),
  refreshDefaultPrimary: vi.fn().mockResolvedValue(undefined),
  registerGraphRefresh: vi.fn(),
  refreshGraph: vi.fn(),
  setPrimaryKbId: vi.fn(),
  toggleKbHidden: vi.fn(),
}));

vi.mock('./KnowledgeContext', () => ({
  useKnowledge: () => ({
    refresh: mocks.refresh,
    refreshDefaultPrimary: mocks.refreshDefaultPrimary,
    registerGraphRefresh: mocks.registerGraphRefresh,
    bases: state.bases,
    visibleBases: state.bases,
    primaryKb: state.bases.find((base) => base.id === state.primaryKbId) ?? null,
    primaryKbId: state.primaryKbId,
    defaultPrimaryKb: null,
    canFollowDefaultPrimary: false,
    followDefaultPrimary: vi.fn(),
    setPrimaryKbId: mocks.setPrimaryKbId,
    hiddenKbIds: [],
    toggleKbHidden: mocks.toggleKbHidden,
    loading: false,
  }),
}));

vi.mock('./hooks/useKnowledgeGraph', () => ({
  useKnowledgeGraph: () => ({
    graph: { nodes: [], edges: [] },
    loading: false,
    error: null,
    refresh: mocks.refreshGraph,
  }),
}));

vi.mock('./hooks/useKnowledgeBases', () => ({
  useKnowledgeBases: () => ({
    create: vi.fn(),
    exportArchive: vi.fn(),
    importArchive: vi.fn(),
    remove: vi.fn(),
    rename: vi.fn(),
  }),
}));

vi.mock('./KbTierControl', () => ({
  KbTierPanel: () => null,
}));

vi.mock('../Layout/MainPanelLayout', () => ({
  MainPanelLayout: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

vi.mock('./SourcesRail', () => ({ SourcesRail: () => null }));
vi.mock('./graph/KnowledgeGraphPanel', () => ({ KnowledgeGraphPanel: () => null }));
vi.mock('./changelog/ChangeLogDrawer', () => ({ ChangeLogDrawer: () => null }));
vi.mock('./lint/LintDrawer', () => ({ LintDrawer: () => null }));

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

afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  vi.clearAllMocks();
});

const TRIGGER_NAME = /^Knowledge base: OMOP/;

/** Put the keyboard on the selector the way a person arrives at it, without a pointer press. */
async function focusTrigger() {
  const trigger = screen.getByRole('button', { name: TRIGGER_NAME });
  await act(async () => trigger.focus());
  expect(trigger).toHaveFocus();
  return trigger;
}

/** Open the picker with Enter and walk to the row named `label` with ArrowDown. */
async function keyboardPick(user: ReturnType<typeof userEvent.setup>, label: string) {
  await user.keyboard('{Enter}');
  const combobox = await screen.findByRole('combobox');
  for (let step = 0; step < 6; step += 1) {
    const active = combobox.getAttribute('aria-activedescendant');
    const row = active ? document.getElementById(active) : null;
    if (row?.textContent?.includes(label)) break;
    await user.keyboard('{ArrowDown}');
  }
  await user.keyboard('{Enter}');
}

describe('closing the knowledge-base manager', () => {
  // The reported path: focus "Knowledge base: OMOP", Enter, ArrowDown, Enter on Manage bases…,
  // Escape. Focus was on <body> with no dialog open.
  it('gives focus back to the selector after a keyboard pick and Escape', async () => {
    const user = userEvent.setup();
    render(<KnowledgeView />);
    const trigger = await focusTrigger();

    await keyboardPick(user, 'Manage bases');
    await screen.findByRole('dialog', { name: knowledgeCopy.manager.title });

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it('gives it back through the chooser: Escape returns to the manager, the next Escape to the selector', async () => {
    const user = userEvent.setup();
    render(<KnowledgeView />);
    const trigger = await focusTrigger();

    await keyboardPick(user, 'Create knowledge base');
    await screen.findByRole('dialog', { name: knowledgeCopy.format.title });

    await user.keyboard('{Escape}');
    await screen.findByRole('dialog', { name: knowledgeCopy.manager.title });
    expect(screen.queryByRole('dialog', { name: knowledgeCopy.format.title })).toBeNull();

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  // The menus' own rule (`useReturnFocusToOpener`): a script focus after a POINTER pick could
  // draw a keyboard ring on a control the person clicked, so a pointer pick gives nothing back.
  it('does not move focus to the selector after a pointer pick', async () => {
    const user = userEvent.setup();
    render(<KnowledgeView />);
    const trigger = screen.getByRole('button', { name: TRIGGER_NAME });

    await user.click(trigger);
    await user.click(await screen.findByTestId('knowledge-kb-open-manager'));
    await screen.findByRole('dialog', { name: knowledgeCopy.manager.title });

    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(trigger).not.toHaveFocus();
  });

  // The pick was a key press, but the person then closed the dialog with the mouse: the rule is
  // about how it was CLOSED too, as it is for a menu-opened dialog.
  it('does not move focus to the selector when the dialog is closed with the pointer', async () => {
    const user = userEvent.setup();
    render(<KnowledgeView />);
    const trigger = await focusTrigger();

    await keyboardPick(user, 'Manage bases');
    const dialog = await screen.findByRole('dialog', { name: knowledgeCopy.manager.title });

    await user.click(dialog.querySelector('button[class*="absolute"]') as HTMLElement);
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(trigger).not.toHaveFocus();
  });

  // The manager is also reachable from the empty states' buttons, which are connected controls
  // the dialog primitive returns to by itself. The selector must not take that over.
  it('leaves a manager opened from the empty state to the dialog primitive', async () => {
    const user = userEvent.setup();
    state.bases = [];
    state.primaryKbId = null;
    try {
      render(<KnowledgeView />);
      const choose = screen.getByRole('button', { name: 'Import .brkb' });
      await act(async () => choose.focus());
      await user.keyboard('{Enter}');
      await screen.findByRole('dialog');

      await user.keyboard('{Escape}');
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      await waitFor(() => expect(choose).toHaveFocus());
    } finally {
      state.bases = [
        { id: 'omop', name: 'OMOP', color: '#cf6d47', tier: 'public' },
        { id: 'soul', name: 'Soul', color: '#b85a32', tier: 'public' },
      ];
      state.primaryKbId = 'omop';
    }
  });
});
