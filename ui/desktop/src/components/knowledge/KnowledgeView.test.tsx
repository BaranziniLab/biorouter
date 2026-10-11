import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import KnowledgeView from './KnowledgeView';
import { knowledgeCopy } from './copy';
import { LEGACY_FORMAT_TITLE } from './kbFormat';

const mocks = vi.hoisted(() => ({
  refresh: vi.fn(),
  registerGraphRefresh: vi.fn(),
  refreshGraph: vi.fn(),
}));

const state = vi.hoisted(() => ({
  primaryKb: null as {
    id: string;
    name: string;
    tier?: string;
    color?: string;
    schema_version?: number;
    format?: string;
  } | null,
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
  state.primaryKb = {
    id: 'omop',
    name: 'OMOP',
    tier: 'private',
    color: '#cf6d47',
    schema_version: 3,
    format: 'okf',
  };
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

  // V3-01: a panel fades in for a PICK only. The layout also flips a panel from hidden to shown
  // (the 860px step), and the browser would replay the fade then, so `data-entering` is what the
  // animation rule in `main.css` keys on (`knowledgeBand.browser.test.tsx` measures the rule).
  it('marks a panel as entering only for a tab a person picked, until its own animation ends', async () => {
    render(<KnowledgeView />);
    const digestPanel = screen.getByTestId('knowledge-digest-panel');
    const graphPanel = screen.getByTestId('knowledge-graph-panel');
    expect(digestPanel).not.toHaveAttribute('data-entering');
    expect(graphPanel).not.toHaveAttribute('data-entering');

    await userEvent.click(screen.getByRole('tab', { name: 'Sources' }));
    expect(digestPanel).toHaveAttribute('data-entering');
    expect(graphPanel).not.toHaveAttribute('data-entering');

    // Something animating INSIDE the panel is not the panel's enter.
    fireEvent.animationEnd(within(digestPanel).getByText('Digest workspace'));
    expect(digestPanel).toHaveAttribute('data-entering');

    fireEvent.animationEnd(digestPanel);
    expect(digestPanel).not.toHaveAttribute('data-entering');

    await userEvent.click(screen.getByRole('tab', { name: 'Graph' }));
    expect(graphPanel).toHaveAttribute('data-entering');
    expect(digestPanel).not.toHaveAttribute('data-entering');
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

// Issue #56 DR-18. The tier control lives beside the base it acts on, not in a
// settings page, where a user reading a private base would never meet it. Spec
// 3.12 moved it from the top of the Sources rail into the band: below the 860px
// step the rail hides behind the default Graph tab, so the tier used to be off
// screen on a narrow pane. In the band it is outside both tab panels.
describe('KnowledgeView tier control', () => {
  it('offers the tier control for the base the view is showing, in the band', () => {
    render(<KnowledgeView />);
    const band = screen.getByTestId('page-header');
    expect(within(band).getByText(/tier control for omop/)).toBeInTheDocument();
    expect(
      within(screen.getByTestId('knowledge-digest-panel')).queryByText(/tier control/)
    ).toBeNull();
    expect(
      within(screen.getByTestId('knowledge-graph-panel')).queryByText(/tier control/)
    ).toBeNull();
  });

  it('offers nothing when there is no base to act on', () => {
    state.primaryKb = null;
    render(<KnowledgeView />);
    expect(screen.queryByText(/tier control for/)).toBeNull();
  });
});

// Spec 3.12: one 44px band through `PageHeader`. The h1 is still named
// "Knowledge" (e2e `knowledge-ingest.spec.ts` finds it), its description is an
// InfoTip rather than a visible line, and the counts are sans tabular figures.
describe('KnowledgeView band', () => {
  it('is the one page band, with the title, its help, the subject and the actions', () => {
    render(<KnowledgeView />);
    const band = screen.getByTestId('page-header');
    expect(
      within(band).getByRole('heading', { level: 1, name: knowledgeCopy.title })
    ).toBeVisible();
    expect(
      within(band).getByRole('button', { name: `About ${knowledgeCopy.title}` })
    ).toHaveAccessibleDescription(knowledgeCopy.viewInfo);
    expect(within(band).getByRole('button', { name: 'Knowledge base' })).toBeInTheDocument();
    expect(within(band).getByRole('button', { name: knowledgeCopy.actions.check })).toBeVisible();
    expect(within(band).getByRole('button', { name: knowledgeCopy.actions.refresh })).toBeVisible();
    expect(within(band).getByRole('button', { name: knowledgeCopy.actions.more })).toBeVisible();
    expect(screen.getAllByRole('heading', { level: 1 })).toHaveLength(1);
  });

  it('draws the counts in the sans face and the format in sentence case', () => {
    render(<KnowledgeView />);
    const summary = screen.getByTestId('knowledge-graph-summary');
    expect(summary).toHaveTextContent(knowledgeCopy.summary(0, 0));
    expect(summary.className).not.toMatch(/font-mono/);
    const format = screen.getByTestId('knowledge-format-badge');
    expect(format).toHaveTextContent('OKF');
    expect(format.className).not.toMatch(/text-caps|uppercase/);
  });

  it('explains a legacy base in an InfoTip on the badge, not a native title', () => {
    state.primaryKb = { id: 'omop', name: 'OMOP', tier: 'private', schema_version: 1 };
    render(<KnowledgeView />);
    const format = screen.getByTestId('knowledge-format-badge');
    expect(format).toHaveTextContent('Legacy');
    expect(format).not.toHaveAttribute('title');
    expect(format).toHaveAccessibleDescription(LEGACY_FORMAT_TITLE);
  });

  it('puts help in tooltips, never in a native title', () => {
    render(<KnowledgeView />);
    const band = screen.getByTestId('page-header');
    expect(band.querySelectorAll('[title]')).toHaveLength(0);
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
    // The band names the same absence, so scope to the EmptyState's own
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
