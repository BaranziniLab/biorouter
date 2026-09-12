import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import KnowledgeView from './KnowledgeView';

const mocks = vi.hoisted(() => ({
  refresh: vi.fn(),
  registerGraphRefresh: vi.fn(),
  refreshGraph: vi.fn(),
}));

const state = vi.hoisted(() => ({
  primaryKb: null as { id: string; name: string; tier?: string; color?: string } | null,
  bases: [] as { id: string }[],
}));

vi.mock('./KnowledgeContext', () => ({
  useKnowledge: () => ({
    refresh: mocks.refresh,
    bases: state.bases,
    primaryKb: state.primaryKb,
    primaryKbId: state.primaryKb?.id ?? null,
    registerGraphRefresh: mocks.registerGraphRefresh,
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
  KbTierPanel: ({ kb }: { kb: { id: string } }) => <div>tier control for {kb.id}</div>,
}));

vi.mock('../Layout/MainPanelLayout', () => ({
  MainPanelLayout: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

vi.mock('../Layout/ReadableContent', () => ({
  ReadableContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

vi.mock('./KBSelector/KBSelectorTrigger', () => ({
  KBSelectorTrigger: () => <button type="button">Knowledge base</button>,
}));

vi.mock('./KBSelector/KBManagerDialog', () => ({
  KBManagerDialog: ({ open }: { open: boolean }) =>
    open ? <div role="dialog" aria-label="Knowledge bases" /> : null,
}));

vi.mock('./IngestPanel/IngestPanel', () => ({
  IngestPanel: () => <div>Digest workspace</div>,
}));

vi.mock('./graph/KnowledgeGraphPanel', () => ({
  KnowledgeGraphPanel: () => <div>Graph workspace</div>,
}));

vi.mock('./changelog/ChangeLogDrawer', () => ({
  ChangeLogDrawer: () => null,
}));

// Mocked for the same reason `ChangeLogDrawer` is: the drawer reads the model
// context to know what to lint with, and this file renders the SHELL. The band's
// affordance is asserted below — the drawer's own behaviour is not this file's
// subject.
vi.mock('./lint/LintDrawer', () => ({
  LintDrawer: ({ open }: { open: boolean }) => (open ? <div>lint drawer</div> : null),
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

afterAll(() => vi.unstubAllGlobals());

beforeEach(() => {
  vi.clearAllMocks();
  state.primaryKb = { id: 'omop', name: 'OMOP', tier: 'private', color: '#cf6d47' };
  state.bases = [{ id: 'omop' }];
});

describe('KnowledgeView compact workspace', () => {
  // ⚠ This case CHANGED with §3.4, deliberately. The tabs are renamed
  // Digest/Graph → Sources/Graph, the pair moves into the subject band so the
  // band still names the base, and the hand-rolled segmented pill becomes the
  // `<Tabs>` primitive — which is where role="tablist", the roving focus and
  // ←/→/Home/End come from. Both `data-testid`s survive; §9 requires it.
  it('lets compact windows give the sources rail and graph their own workspace', async () => {
    render(<KnowledgeView />);

    const sourcesTab = screen.getByRole('tab', { name: 'Sources' });
    const graphTab = screen.getByRole('tab', { name: 'Graph' });
    const digestPanel = screen.getByTestId('knowledge-digest-panel');
    const graphPanel = screen.getByTestId('knowledge-graph-panel');

    expect(graphTab).toHaveAttribute('aria-selected', 'true');
    expect(graphPanel).toHaveClass('flex');
    expect(digestPanel).toHaveClass('hidden');

    await userEvent.click(sourcesTab);

    expect(sourcesTab).toHaveAttribute('aria-selected', 'true');
    expect(digestPanel).toHaveClass('flex');
    expect(graphPanel).toHaveClass('hidden');

    await userEvent.click(graphTab);

    expect(graphTab).toHaveAttribute('aria-selected', 'true');
    expect(graphPanel).toHaveClass('flex');
    expect(digestPanel).toHaveClass('hidden');
  });

  // ⚠ This is why the `Tabs` root spans the subject band AND the workspace. The
  // triggers live in the band and the panels live in the workspace, and Radix
  // links them by generated id — two roots would render two working tab strips
  // whose `aria-controls` pointed at nothing, which no visual check would show.
  it('links each trigger to the panel it actually controls', () => {
    render(<KnowledgeView />);

    const pairs: [HTMLElement, HTMLElement][] = [
      [screen.getByRole('tab', { name: 'Sources' }), screen.getByTestId('knowledge-digest-panel')],
      [screen.getByRole('tab', { name: 'Graph' }), screen.getByTestId('knowledge-graph-panel')],
    ];

    for (const [tab, panel] of pairs) {
      const controls = tab.getAttribute('aria-controls');
      expect(controls).toBeTruthy();
      expect(document.getElementById(controls!)).toBe(panel);
    }
  });
});

// The lint route and its generated client both shipped with no caller, so the
// only validation the user was promised had no way to be run. The band is where
// it lives: read-only, so NOT behind the `⋯` with the destructive actions.
describe('KnowledgeView lint affordance', () => {
  it('offers a way to run the check, and opens it', async () => {
    render(<KnowledgeView />);

    const check = screen.getByRole('button', { name: 'Check for problems' });
    expect(check).toBeEnabled();
    expect(screen.queryByText('lint drawer')).toBeNull();

    await userEvent.click(check);
    expect(screen.getByText('lint drawer')).toBeInTheDocument();
  });

  it('does not offer a check when there is no base to check', () => {
    state.primaryKb = null;
    state.bases = [{ id: 'omop' }];
    render(<KnowledgeView />);
    expect(screen.getByRole('button', { name: 'Check for problems' })).toBeDisabled();
  });
});

// Issue #56 DR-18. The tier control lives beside the base it acts on — at the
// top of the Sources rail — not in a settings page, where a user reading a
// private base would never meet it.
describe('KnowledgeView tier control', () => {
  it('offers the tier control for the base the view is showing', () => {
    render(<KnowledgeView />);
    expect(screen.getByText(/tier control for omop/)).toBeInTheDocument();
  });

  it('offers nothing when there is no base to act on', () => {
    state.primaryKb = null;
    render(<KnowledgeView />);
    expect(screen.queryByText(/tier control for/)).toBeNull();
  });
});

// §4.12 #1 and #2. The section used to hand-roll seven of these as bare centred
// sentences, which is what made it read thinner than its siblings on exactly
// the screen a new user sees first.
describe('KnowledgeView empty states', () => {
  it('offers a way to make the first base when there are none', () => {
    state.primaryKb = null;
    state.bases = [];
    render(<KnowledgeView />);
    expect(screen.getByRole('heading', { name: 'No knowledge bases yet' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create knowledge base' })).toBeInTheDocument();
  });

  it('opens the manager when a chat has bases but no primary', async () => {
    state.primaryKb = null;
    state.bases = [{ id: 'omop' }];
    render(<KnowledgeView />);
    // The subject band names the same absence, so scope to the EmptyState's own
    // heading rather than to the words.
    expect(screen.getByRole('heading', { name: 'No primary knowledge base' })).toBeInTheDocument();
    // D13: the EmptyState replaces the WHOLE body, the Sources rail and its
    // ingest panel included. That is why `IngestPanel` carries no blocked reason
    // for a missing primary — it can never be mounted in that state, and this
    // EmptyState is the better answer anyway, because it carries the action.
    expect(screen.queryByText('Digest workspace')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Choose a base' }));
    expect(screen.getByRole('dialog', { name: 'Knowledge bases' })).toBeInTheDocument();
  });
});
