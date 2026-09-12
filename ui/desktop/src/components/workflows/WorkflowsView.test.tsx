import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import WorkflowsView from './WorkflowsView';

const mocks = vi.hoisted(() => ({
  listSavedWorkflows: vi.fn(),
  refreshConfig: vi.fn(),
  setWorkflowSlashCommand: vi.fn(),
  startAgent: vi.fn(),
  setView: vi.fn(),
  userActionHeaders: vi.fn(),
}));

vi.mock('../../workflow/workflow_management', () => ({
  listSavedWorkflows: mocks.listSavedWorkflows,
  convertToLocaleDateString: () => 'Jul 11, 2026',
}));

vi.mock('../../hooks/useNavigation', () => ({ useNavigation: () => mocks.setView }));
vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  setWorkflowSlashCommand: mocks.setWorkflowSlashCommand,
  startAgent: mocks.startAgent,
}));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.userActionHeaders }));
vi.mock('../../utils/workingDir', () => ({ getInitialWorkingDir: () => '/tmp/workspace' }));
vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ refreshConfig: mocks.refreshConfig }),
}));
vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => children,
}));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.refreshConfig.mockResolvedValue(undefined);
  mocks.setWorkflowSlashCommand.mockResolvedValue({ data: undefined });
  mocks.userActionHeaders.mockResolvedValue({ 'X-User-Action': 'proof-of-user' });
});

describe('WorkflowsView loading transition', () => {
  it('shows its skeleton immediately and reveals loaded workflows without a blank timer gap', async () => {
    let finishLoad: ((value: unknown[]) => void) | undefined;
    mocks.listSavedWorkflows.mockReturnValueOnce(
      new Promise<unknown[]>((resolve) => {
        finishLoad = resolve;
      })
    );

    const { container } = render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

    expect(container.querySelectorAll('[data-slot="skeleton"]').length).toBeGreaterThan(0);
    finishLoad?.([
      {
        id: 'workflow-1',
        file_path: '/tmp/workflow.yaml',
        last_modified: '2026-07-11',
        workflow: { title: 'Cohort Review', description: 'Review cohort results' },
      },
    ]);

    expect(await screen.findByText('Cohort Review')).toBeInTheDocument();
    await waitFor(() => {
      expect(container.querySelector('[data-slot="skeleton"]')).not.toBeInTheDocument();
    });
  });

  it('refreshes the config cache after writing a workflow slash command', async () => {
    mocks.listSavedWorkflows.mockResolvedValue([
      {
        id: 'workflow-1',
        file_path: '/tmp/workflow.yaml',
        last_modified: '2026-07-11',
        workflow: { title: 'Cohort Review', description: 'Review cohort results' },
      },
    ]);

    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

    fireEvent.click(await screen.findByTitle('Add slash command'));
    fireEvent.change(screen.getByPlaceholderText('command-name'), {
      target: { value: 'cohort-review' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(mocks.setWorkflowSlashCommand).toHaveBeenCalledOnce());
    expect(mocks.refreshConfig).toHaveBeenCalledOnce();
  });

  it('presents an accessible empty state with create and import actions', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([]);

    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

    const title = await screen.findByRole('heading', { name: 'No workflows yet' });
    const emptyState = title.closest('section');

    expect(emptyState).toHaveAccessibleDescription(
      'Create a reusable workflow here, save one from a chat, or import an existing workflow.'
    );
    // ⚠ Scoped to the empty state, not to the page. The header offers the SAME
    // two actions, and until the casing sweep it offered them under different
    // names ("Create Workflow"/"Import Workflow"), which is the only reason an
    // unscoped `getByRole` ever resolved to one element here.
    const emptyActions = within(emptyState as HTMLElement);
    expect(emptyActions.getByRole('button', { name: 'Create workflow' })).toBeInTheDocument();
    expect(emptyActions.getByRole('button', { name: 'Import workflow' })).toBeInTheDocument();
  });

  it('proves that starting a workflow came from the renderer user', async () => {
    const workflow = { title: 'Cohort Review', description: 'Review cohort results' };
    mocks.listSavedWorkflows.mockResolvedValueOnce([
      {
        id: 'workflow-1',
        file_path: '/tmp/workflow.yaml',
        last_modified: '2026-07-11',
        workflow,
      },
    ]);
    mocks.startAgent.mockResolvedValueOnce({ data: { id: 'workflow-session' } });

    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

    fireEvent.click(await screen.findByTitle('Use workflow'));

    await waitFor(() => {
      expect(mocks.startAgent).toHaveBeenCalledWith({
        body: { working_dir: '/tmp/workspace', workflow },
        headers: { 'X-User-Action': 'proof-of-user' },
        throwOnError: true,
      });
    });
    expect(mocks.setView).toHaveBeenCalledWith('pair', {
      disableAnimation: true,
      resumeSessionId: 'workflow-session',
    });
  });
});

/**
 * The visual vocabulary, where a DOM test can actually see it.
 *
 * jsdom runs no Tailwind, so nothing here reads a colour or a width — a class
 * string that paints a row computes to nothing in this environment. What these
 * assertions prove is which class string is PRESENT, which is the same
 * statement `Layout/PageHeader.test.tsx` makes about the strip and the
 * hairline, and the companion `styles/measures.test.ts` makes at the source.
 */
describe('WorkflowsView on the settings visual vocabulary', () => {
  const WORKFLOW = {
    id: 'workflow-1',
    file_path: '/tmp/workflow.yaml',
    last_modified: '2026-07-11',
    schedule_cron: '0 0 14 * * *',
    slash_command: 'cohort-review',
    workflow: {
      title:
        'A workflow whose title is long enough to need the whole column and then some more besides',
      description: 'Review cohort results across every arm of the study, then summarise them',
    },
  };

  const renderView = () =>
    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

  it('mounts the shared page header rather than a ninth copy of it', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([]);
    const { container } = renderView();

    expect(screen.getByRole('heading', { level: 1, name: 'Workflows' })).toBeInTheDocument();

    // The operator's decision, pinned here as well as in PageHeader's own
    // suite: the page's actions sit in the control strip under the description,
    // not in a hand-rolled `flex gap-3` beside the title.
    const strip = container.querySelector('.biorouter-settings-control-strip');
    expect(strip).not.toBeNull();
    // Queried INSIDE the strip rather than page-wide and asserted to be
    // contained: the empty state below offers the same two actions under the
    // same two names, so a page-wide `getByRole` finds two of each.
    const stripActions = within(strip as HTMLElement);
    expect(stripActions.getByRole('button', { name: 'Create workflow' })).toBeInTheDocument();
    expect(stripActions.getByRole('button', { name: 'Import workflow' })).toBeInTheDocument();

    // `page-transition` matches no CSS rule in this repo and resolves to no
    // animation in the running app. It was on this header; it does not come back.
    expect(container.querySelector('.page-transition')).toBeNull();

    await screen.findByRole('heading', { name: 'No workflows yet' });
  });

  it('keeps the description’s words, including the search shortcut', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([]);
    renderView();

    expect(
      screen.getByText(
        /^View and manage your saved workflows to quickly start new chats with predefined configurations\. .+ to search\.$/
      )
    ).toBeInTheDocument();

    await screen.findByRole('heading', { name: 'No workflows yet' });
  });

  /**
   * Both reading columns, not just the body's. The header's hairline is
   * full-bleed, so a header on one measure and a body on another shows as a
   * step in the left edge they share — which is invisible to jsdom and visible
   * immediately in the app.
   */
  it('puts every reading column on the chat measure', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    const { container } = renderView();
    await screen.findByTitle('Use workflow');

    const columns = container.querySelectorAll('.biorouter-readable-content');
    // The vacuous pass: with no columns the loop below asserts nothing.
    expect(columns.length).toBeGreaterThan(1);
    for (const column of columns) expect(column).toHaveAttribute('data-size', 'chat');
  });

  /**
   * The row has to survive ~704px of content box with seven actions in it. A
   * flex item's `min-width: auto` is what let the title push its siblings out,
   * and `truncate` makes that min-content width the WHOLE string — so the two
   * classes are a pair and neither alone is the fix.
   *
   * The old cap this replaced was `max-w-[50vw]`: keyed to the viewport rather
   * than to the pane, so at 1440px it resolved wider than the column it was
   * supposed to fit inside. Asserted as "no `vw` cap anywhere in the row"
   * rather than as the one spelling, because any viewport-keyed ceiling on a
   * row inside a fixed measure is the same mistake.
   */
  it('lets a long row title shrink instead of pushing the actions out', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    renderView();

    const title = await screen.findByRole('heading', { level: 3, name: WORKFLOW.workflow.title });
    expect(title).toHaveClass('min-w-0');
    expect(title).toHaveClass('truncate');

    const row = title.closest('.biorouter-list-row');
    expect(row).not.toBeNull();
    // `getAttribute` rather than `.className`: on an SVG element the property
    // is an `SVGAnimatedString`, which stringifies to `[object
    // SVGAnimatedString]` and would make the regex below vacuously true there.
    const viewportCapped = Array.from(row!.querySelectorAll('*'))
      .map((element) => element.getAttribute('class') ?? '')
      .filter((classes) => /max-w-\[[^\]]*vw\]/.test(classes));
    expect(viewportCapped).toEqual([]);

    // The action cluster is the box that must NOT give ground.
    const actions = screen.getByTitle('Use workflow').parentElement;
    expect(actions).toHaveClass('shrink-0');
  });

  /**
   * A CTA does not live inside a list row (operator decision). The Run action
   * was the one accent-filled button in the list; it is a ghost like its six
   * siblings now, and the two `tint-selected` buttons are the only fills left —
   * those say something about the WORKFLOW (it has a slash command, it has a
   * schedule) rather than about what the button does.
   */
  it('draws every row action as a ghost on the 32px round rung', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    renderView();

    const run = await screen.findByTitle('Use workflow');
    expect(run).toHaveClass('bg-transparent');
    expect(run.className).not.toContain('bg-background-accent');

    for (const label of [
      'Use workflow',
      'Launch workflow',
      'Edit workflow',
      'Share workflow',
      'Delete workflow',
      'Edit slash command',
      'Edit schedule',
    ]) {
      const action = screen.getByTitle(label);
      expect(action.className).not.toContain('bg-background-accent');
      // `shape="round"` at the default rung — the one row-action size. The
      // delete button used to be `size="sm"` with no shape, so it alone was a
      // 28px pill in a line of 32px squares.
      expect(action).toHaveClass('w-8');
      expect(action).toHaveClass('h-8');
    }
  });

  /**
   * Loading is rows that are the shape of rows. The list shell is what makes
   * them share the loaded list's hairlines rather than sitting in a second,
   * differently-spaced stack.
   */
  it('loads as skeleton rows inside the list shell', () => {
    mocks.listSavedWorkflows.mockReturnValueOnce(new Promise(() => {}));
    const { container } = renderView();

    const shell = container.querySelector('.biorouter-list-shell');
    expect(shell).not.toBeNull();
    expect(shell!.querySelectorAll('.biorouter-list-row').length).toBe(3);
    expect(shell!.querySelectorAll('[data-slot="skeleton"]').length).toBeGreaterThan(0);
  });

  /**
   * The error state fills the body, so it is the shared `EmptyState` — not a
   * hand-rolled centred stack with its own icon size and its own type ramp.
   * The daemon's message is the description, so a failure still says what
   * failed.
   */
  it('reports a failed load through the shared empty state, with a retry', async () => {
    mocks.listSavedWorkflows.mockRejectedValueOnce(new Error('the daemon is not reachable'));
    renderView();

    const title = await screen.findByRole('heading', { name: 'Couldn’t load workflows' });
    expect(title.closest('section')).toHaveAccessibleDescription('the daemon is not reachable');

    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await screen.findByTitle('Use workflow')).toBeInTheDocument();
  });
});

/**
 * Defect 3.4. This is a MIS-ACTION risk, not missing copy: with two rows and
 * two identically-titled "Add schedule" buttons, QA scheduled the wrong
 * workflow on the first try. The dialog held the subject in state the whole
 * time (`scheduleWorkflowManifest`) and simply never showed it, and
 * `aria-describedby={undefined}` opted the dialog out of a description, so a
 * screen reader heard "Add schedule" and nothing else either.
 */
describe('the schedule dialog names the workflow it is about to schedule', () => {
  const rows = [
    {
      id: 'workflow-1',
      file_path: '/tmp/one.yaml',
      last_modified: '2026-07-11',
      workflow: { title: 'Cohort review', description: 'one' },
    },
    {
      id: 'workflow-2',
      file_path: '/tmp/two.yaml',
      last_modified: '2026-07-11',
      workflow: { title: 'Variant calling nightly', description: 'two' },
    },
  ];

  it('names the row the button belonged to, not merely "Add schedule"', async () => {
    mocks.listSavedWorkflows.mockResolvedValue(rows);
    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

    await screen.findByText('Variant calling nightly');
    const buttons = screen.getAllByTitle('Add schedule');
    expect(buttons).toHaveLength(2);
    fireEvent.click(buttons[1]);

    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Variant calling nightly/)).toBeInTheDocument();
    // The wrong subject must not be reachable from the dialog at all.
    expect(within(dialog).queryByText(/Cohort review/)).not.toBeInTheDocument();
  });

  it('describes itself to assistive technology instead of opting out', async () => {
    mocks.listSavedWorkflows.mockResolvedValue(rows);
    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

    await screen.findByText('Cohort review');
    fireEvent.click(screen.getAllByTitle('Add schedule')[0]);

    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveAttribute('aria-describedby');
  });
});
