import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import WorkflowsView from './WorkflowsView';
import { workflowsCopy as copy } from './copy';

const mocks = vi.hoisted(() => ({
  listSavedWorkflows: vi.fn(),
  refreshConfig: vi.fn(),
  setWorkflowSlashCommand: vi.fn(),
  startAgent: vi.fn(),
  setView: vi.fn(),
  userActionHeaders: vi.fn(),
  scheduleWorkflow: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
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
  scheduleWorkflow: mocks.scheduleWorkflow,
}));
vi.mock('../../toasts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../toasts')>()),
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.userActionHeaders }));
vi.mock('../../utils/workingDir', () => ({ getInitialWorkingDir: () => '/tmp/workspace' }));
vi.mock('../ConfigContext', () => ({
  useConfig: () => ({ refreshConfig: mocks.refreshConfig }),
}));
// The editor is a dialog of its own with its own tests; here it only has to open.
vi.mock('./CreateEditWorkflowModal', () => ({
  default: ({ isOpen, isCreateMode }: { isOpen: boolean; isCreateMode?: boolean }) =>
    isOpen ? <div data-testid={isCreateMode ? 'workflow-creator' : 'workflow-editor'} /> : null,
}));
vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

/** Radix opens a dropdown on pointerdown, not click. */
async function openRowMenu(title: string) {
  fireEvent.pointerDown(await screen.findByRole('button', { name: copy.moreActionsNamed(title) }), {
    button: 0,
    ctrlKey: false,
  });
}

/** Open a row's `⋯` menu and pick one of its items. */
async function chooseFromRowMenu(title: string, item: string) {
  await openRowMenu(title);
  fireEvent.click(await screen.findByRole('menuitem', { name: item }));
}

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

    await chooseFromRowMenu('Cohort Review', copy.slashCommandItem);
    const dialog = await screen.findByRole('dialog');
    // The dialog names the workflow it acts on, and says its one live line.
    expect(dialog).toHaveAccessibleDescription('Cohort Review');
    fireEvent.change(within(dialog).getByPlaceholderText(copy.slash.placeholder), {
      target: { value: 'cohort-review' },
    });
    expect(within(dialog).getByText(copy.slash.helper('cohort-review'))).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole('button', { name: copy.slash.save }));

    await waitFor(() => expect(mocks.setWorkflowSlashCommand).toHaveBeenCalledOnce());
    expect(mocks.refreshConfig).toHaveBeenCalledOnce();
    // One line: the command, saved.
    await waitFor(() =>
      expect(mocks.toastSuccess).toHaveBeenCalledWith({ title: copy.slash.saved('cohort-review') })
    );
  });

  it.each(['Save', 'Remove'])(
    'keeps slash-command errors in the dialog when %s fails',
    async (action) => {
      mocks.listSavedWorkflows.mockResolvedValue([
        {
          id: 'workflow-1',
          file_path: '/tmp/workflow.yaml',
          last_modified: '2026-07-11',
          slash_command: 'cohort-review',
          workflow: { title: 'Cohort Review', description: 'Review cohort results' },
        },
      ]);
      mocks.setWorkflowSlashCommand.mockRejectedValue(
        '/effort is reserved. Choose a different name.'
      );
      render(
        <MemoryRouter>
          <WorkflowsView />
        </MemoryRouter>
      );
      await chooseFromRowMenu('Cohort Review', copy.slashCommandItem);
      fireEvent.change(await screen.findByPlaceholderText(copy.slash.placeholder), {
        target: { value: 'effort' },
      });
      fireEvent.click(screen.getByRole('button', { name: action }));
      expect(await screen.findByRole('alert')).toHaveTextContent('/effort is reserved');
      expect(screen.getByRole('dialog')).toBeInTheDocument();
      expect(screen.getByPlaceholderText(copy.slash.placeholder)).toHaveValue('effort');
      // The error replaces the live helper line, in danger ink.
      expect(screen.queryByText(copy.slash.helper('effort'))).not.toBeInTheDocument();
      expect(screen.queryByText('Couldn’t load workflows')).not.toBeInTheDocument();
      expect(mocks.setWorkflowSlashCommand).toHaveBeenCalledWith(
        expect.objectContaining({ throwOnError: true })
      );
      expect(mocks.refreshConfig).not.toHaveBeenCalled();
      expect(mocks.toastSuccess).not.toHaveBeenCalled();
    }
  );

  it('presents a one-sentence empty state whose only action is a link', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([]);

    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

    const title = await screen.findByRole('heading', { name: copy.emptyTitle });
    const emptyState = title.closest('section');

    expect(emptyState).toHaveAccessibleDescription(copy.emptyDescription);
    // The band holds the view's one accent ("New workflow"); the empty state
    // offers at most a link, never a second filled button.
    const emptyActions = within(emptyState as HTMLElement);
    const actions = emptyActions.getAllByRole('button');
    expect(actions).toHaveLength(1);
    expect(actions[0]).toHaveAccessibleName(copy.importWorkflow);
    expect(actions[0].className).not.toContain('bg-background-accent');
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

    fireEvent.click(await screen.findByRole('button', { name: copy.runNamed('Cohort Review') }));

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
/**
 * Vision round 1, V3: the slash dialog's one live helper line and its in-field "/", Cancel as the
 * filled secondary in every Views-A dialog, "Delete…" for an item that opens a confirmation, and a
 * built-in row that does not claim to have just been edited.
 */
describe('WorkflowsView dialogs and rows after vision round 1', () => {
  const PLAIN = {
    id: 'workflow-1',
    file_path: '/tmp/workflow.yaml',
    last_modified: '2026-07-11',
    workflow: { title: 'Cohort Review', description: 'Review cohort results' },
  };

  const renderView = () =>
    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );

  it('says its one helper line before anything is typed, then follows the name (V3-19)', async () => {
    mocks.listSavedWorkflows.mockResolvedValue([PLAIN]);
    renderView();

    await chooseFromRowMenu('Cohort Review', copy.slashCommandItem);
    const dialog = await screen.findByRole('dialog');
    const input = within(dialog).getByPlaceholderText(copy.slash.placeholder);

    // Empty: the line is already there, and the field is described by it.
    const helper = within(dialog).getByText(copy.slash.helperEmpty);
    expect(helper).toHaveAttribute('id', 'workflow-slash-command-help');
    expect(input).toHaveAttribute('aria-describedby', 'workflow-slash-command-help');
    expect(helper).toHaveClass('text-supporting');

    fireEvent.change(input, { target: { value: 'cohort-review' } });
    expect(within(dialog).queryByText(copy.slash.helperEmpty)).toBeNull();
    expect(within(dialog).getByText(copy.slash.helper('cohort-review'))).toBeInTheDocument();
  });

  it('draws the "/" inside the command field, not loose beside it (V3-19)', async () => {
    mocks.listSavedWorkflows.mockResolvedValue([PLAIN]);
    renderView();

    await chooseFromRowMenu('Cohort Review', copy.slashCommandItem);
    const dialog = await screen.findByRole('dialog');
    const input = within(dialog).getByPlaceholderText(copy.slash.placeholder);

    const field = input.parentElement as HTMLElement;
    expect(field).toHaveClass('br-workflow-slash-field');
    const glyph = field.querySelector('.br-workflow-slash-glyph');
    expect(glyph).toHaveTextContent('/');
    expect(glyph).toHaveAttribute('aria-hidden', 'true');
    expect(input).toHaveClass('br-workflow-slash-input');
    // The field sits straight under its label, with no loose "/" element beside it.
    expect(field.previousElementSibling?.tagName).toBe('LABEL');
    expect(field.parentElement?.querySelectorAll(':scope > span')).toHaveLength(0);
  });

  it('keeps the spellchecker and the keyboard out of the command name (V4-R7-5)', async () => {
    mocks.listSavedWorkflows.mockResolvedValue([PLAIN]);
    renderView();

    await chooseFromRowMenu('Cohort Review', copy.slashCommandItem);
    const dialog = await screen.findByRole('dialog');
    const input = within(dialog).getByPlaceholderText(copy.slash.placeholder);

    // A command name is a machine string: no red line under `cohort-review`, no capital on the
    // first letter of a touch keyboard. (jsdom has no spellchecker; this is the markup it reads.)
    expect(input).toHaveAttribute('spellcheck', 'false');
    expect(input).toHaveAttribute('autocorrect', 'off');
    expect(input).toHaveAttribute('autocapitalize', 'off');
  });

  it('draws Cancel as the filled secondary in the slash and schedule dialogs (V3-06)', async () => {
    mocks.listSavedWorkflows.mockResolvedValue([PLAIN]);
    renderView();

    await chooseFromRowMenu('Cohort Review', copy.slashCommandItem);
    let dialog = await screen.findByRole('dialog');
    const cancel = within(dialog).getByRole('button', { name: copy.cancel });
    expect(cancel).toHaveClass('br-fill-rest');
    expect(cancel).not.toHaveClass('bg-transparent');
    fireEvent.click(cancel);
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    await chooseFromRowMenu('Cohort Review', copy.scheduleItem);
    dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByRole('button', { name: copy.cancel })).toHaveClass('br-fill-rest');
  });

  it('opens the Schedule dialog on the dialog, not on a closed Select (V3-20)', async () => {
    mocks.listSavedWorkflows.mockResolvedValue([PLAIN]);
    renderView();

    await chooseFromRowMenu('Cohort Review', copy.scheduleItem);
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByRole('combobox', { name: 'Repeat every' })).toBeInTheDocument();
    await waitFor(() => expect(document.activeElement).toBe(dialog));
  });

  it('marks the row item that opens a confirmation with an ellipsis (V3-21)', async () => {
    mocks.listSavedWorkflows.mockResolvedValue([PLAIN]);
    renderView();
    await openRowMenu('Cohort Review');

    expect(await screen.findByRole('menuitem', { name: 'Delete…' })).toBeInTheDocument();
    expect(copy.delete).toBe('Delete…');
    // The confirmation's own button is the plain verb.
    expect(copy.deleteConfirm).toBe('Delete');
  });

  it('shows no modified time on a built-in, whose file is rewritten at every launch (V3-22)', async () => {
    mocks.listSavedWorkflows.mockResolvedValue([
      { ...PLAIN, id: 'built-in', file_path: '/tmp/workflows/meditation.yaml' },
      {
        ...PLAIN,
        id: 'plain',
        file_path: '/tmp/workflows/other.yaml',
        workflow: { title: 'Other', description: 'Another one' },
      },
    ]);
    renderView();

    const rows = await screen.findAllByTestId('workflow-row');
    expect(rows).toHaveLength(2);
    const [builtIn, other] = rows;
    expect(within(builtIn).getByText('Cohort Review')).toBeInTheDocument();
    expect(builtIn.querySelector('time')).toBeNull();
    expect(other.querySelector('time')).not.toBeNull();
  });
});

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

  it('puts the title, its help, the filter, Import and one primary on the band', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([]);
    const { container } = renderView();

    expect(screen.getByRole('heading', { level: 1, name: copy.title })).toBeInTheDocument();
    // The description is help behind an InfoTip, not a paragraph under the title.
    expect(screen.getByRole('button', { name: `About ${copy.title}` })).toHaveAccessibleDescription(
      copy.info
    );
    expect(container.querySelector('.biorouter-page-header p')).toBeNull();

    // The band (spec 3.10): the page's actions sit at its trailing edge, in
    // PageHeader's own cluster, not in a hand-rolled `flex gap-3`.
    const strip = container.querySelector('.biorouter-page-header-actions');
    expect(strip).not.toBeNull();
    const stripActions = within(strip as HTMLElement);
    expect(stripActions.getByRole('searchbox', { name: copy.filterLabel })).toBeInTheDocument();
    expect(stripActions.getByRole('button', { name: copy.importWorkflow })).toHaveClass(
      'bg-transparent'
    );
    const primary = stripActions.getByRole('button', { name: copy.newWorkflow });
    expect(primary).toHaveClass('bg-background-accent');
    // One accent on the band, and none anywhere else on the page.
    expect(container.querySelectorAll('button.bg-background-accent')).toHaveLength(1);

    // `page-transition` matches no CSS rule in this repo and resolves to no
    // animation in the running app. It was on this header; it does not come back.
    expect(container.querySelector('.page-transition')).toBeNull();

    await screen.findByRole('heading', { name: copy.emptyTitle });
  });

  it('filters the rows from the band', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([
      WORKFLOW,
      {
        ...WORKFLOW,
        id: 'workflow-2',
        slash_command: null,
        workflow: { title: 'Variant calling', description: 'Call variants' },
      },
    ]);
    renderView();
    await screen.findByText('Variant calling');

    fireEvent.change(screen.getByRole('searchbox', { name: copy.filterLabel }), {
      target: { value: 'variant' },
    });
    expect(screen.getByText('Variant calling')).toBeInTheDocument();
    expect(screen.queryByText(WORKFLOW.workflow.title)).not.toBeInTheDocument();

    fireEvent.change(screen.getByRole('searchbox', { name: copy.filterLabel }), {
      target: { value: 'nothing like it' },
    });
    expect(screen.getByRole('heading', { name: copy.noMatchTitle })).toBeInTheDocument();
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
    await screen.findByRole('button', { name: copy.runNamed(WORKFLOW.workflow.title) });

    const columns = container.querySelectorAll('.biorouter-readable-content');
    // The vacuous pass: with no columns the loop below asserts nothing. The band
    // sits in no column, so the body's is the one there is.
    expect(columns.length).toBeGreaterThan(0);
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

    // The action slot is the box that must NOT give ground (`.br-row-actions-slot` is
    // `flex: none`, authored in main.css).
    const run = screen.getByRole('button', { name: copy.runNamed(WORKFLOW.workflow.title) });
    expect(run.closest('[data-slot="row-actions-slot"]')).toHaveClass('br-row-actions-slot');
  });

  /**
   * V3-R3-4: a `title` attribute on the row title is turned into a tooltip by AppTooltipLayer,
   * and the tooltip said the name the row already shows (the title was whole, not cut). The
   * Scheduler and Skills rows show none, so the three views answered a hover differently.
   */
  it('puts no tooltip on the row title, as the Scheduler and Skills rows have none', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    renderView();

    const title = await screen.findByRole('heading', { level: 3, name: WORKFLOW.workflow.title });
    expect(title).not.toHaveAttribute('title');
    expect(title.closest('[data-testid="workflow-row"]')!.querySelector('h3[title]')).toBeNull();
  });

  /**
   * Two lines and two actions (spec 3.10). Line 1 is the title and its state as neutral
   * badges; line 2 the description. At rest the trailing edge shows only the modified time;
   * on hover or focus Run and `⋯` replace it. The state the two tinted buttons used to carry
   * (a slash command exists, a schedule exists) is the badges now.
   */
  it('says its state in badges on line 1, and keeps two ghost actions', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    renderView();

    const run = await screen.findByRole('button', { name: copy.runNamed(WORKFLOW.workflow.title) });
    const row = run.closest('[data-testid="workflow-row"]') as HTMLElement;
    expect(row).toHaveClass('biorouter-list-row');

    const badges = within(row);
    expect(badges.getByText('/cohort-review')).toBeInTheDocument();
    const cron = badges.getByText('Daily 2 PM');
    // The whole rule is one hover or Tab away, on the badge itself.
    expect(cron).toHaveAccessibleDescription('Every day at 2:00 PM');
    expect(badges.getByText(WORKFLOW.workflow.description)).toHaveClass('truncate');

    // The modified time sits in the sans face with tabular figures, never mono.
    const time = row.querySelector('time');
    expect(time).not.toBeNull();
    expect(time).toHaveClass('tabular-nums');
    expect(time!.className).not.toContain('font-mono');

    // Run and `⋯`, both ghost on the 32px round rung; no other button in the row.
    const more = badges.getByRole('button', {
      name: copy.moreActionsNamed(WORKFLOW.workflow.title),
    });
    for (const action of [run, more]) {
      expect(action).toHaveClass('bg-transparent');
      expect(action).toHaveClass('w-control-md');
      expect(action).toHaveClass('h-control-md');
    }
    expect(badges.getAllByRole('button')).toHaveLength(2);
  });

  it('holds every other verb in the row menu, Delete last and destructive', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    renderView();
    await openRowMenu(WORKFLOW.workflow.title);

    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.textContent)).toEqual([
      copy.runInNewWindow,
      copy.edit,
      copy.slashCommandItem,
      copy.scheduleItem,
      copy.copyLink,
      copy.copyYaml,
      copy.exportToFile,
      copy.delete,
    ]);
    expect(items[items.length - 1]).toHaveAttribute('data-variant', 'destructive');
  });

  it('opens the same menu on a right-click of the row', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    renderView();
    fireEvent.contextMenu(await screen.findByText(WORKFLOW.workflow.description));
    expect(await screen.findByRole('menuitem', { name: copy.delete })).toBeInTheDocument();
  });

  it('opens the editor on a row click, but not on a click of its own controls', async () => {
    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    mocks.startAgent.mockReturnValueOnce(new Promise(() => {}));
    renderView();

    fireEvent.click(
      await screen.findByRole('button', { name: copy.runNamed(WORKFLOW.workflow.title) })
    );
    expect(screen.queryByTestId('workflow-editor')).not.toBeInTheDocument();

    fireEvent.click(screen.getByText(WORKFLOW.workflow.description));
    expect(screen.getByTestId('workflow-editor')).toBeInTheDocument();
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

    const title = await screen.findByRole('heading', { name: copy.loadFailedTitle });
    expect(title.closest('section')).toHaveAccessibleDescription('the daemon is not reachable');

    mocks.listSavedWorkflows.mockResolvedValueOnce([WORKFLOW]);
    fireEvent.click(screen.getByRole('button', { name: copy.tryAgain }));
    expect(
      await screen.findByRole('button', { name: copy.runNamed(WORKFLOW.workflow.title) })
    ).toBeInTheDocument();
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
    await chooseFromRowMenu('Variant calling nightly', copy.scheduleItem);

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
    await chooseFromRowMenu('Cohort review', copy.scheduleItem);

    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveAttribute('aria-describedby');
    expect(dialog).toHaveAccessibleDescription('Cohort review');
  });
});

/**
 * Issue #56. The daemon refuses `POST /workflows/schedule` for a schedule whose
 * work is private unless the request carries the person's proof. This page
 * called the generated client itself, sent no proof and never read the answer,
 * so in the dev GUI on a private default (measured 2026-09-14) "Save" got a 403
 * and toasted "Schedule saved", and "Remove schedule" got a 403 and toasted
 * "will no longer run automatically" while the row still read "Runs at 03:00 am"
 * and the scheduler still held the job.
 */
describe("the Workflows page's schedule writes carry the proof and read the answer", () => {
  const REFUSAL = "That schedule's work is private, or there is no schedule with that id.";
  const unscheduled = {
    id: 'workflow-1',
    file_path: '/tmp/one.yaml',
    last_modified: '2026-07-11',
    workflow: { title: 'Cohort review', description: 'one' },
  };
  const scheduled = { ...unscheduled, schedule_cron: '0 0 3 * * *' };

  async function openDialog(row: typeof unscheduled) {
    mocks.listSavedWorkflows.mockResolvedValue([row]);
    render(
      <MemoryRouter>
        <WorkflowsView />
      </MemoryRouter>
    );
    await screen.findByText('Cohort review');
    await chooseFromRowMenu('Cohort review', copy.scheduleItem);
    return screen.findByRole('dialog');
  }

  it('Save sends the proof, and a refusal is an error rather than "Schedule saved"', async () => {
    mocks.scheduleWorkflow.mockResolvedValue({
      error: REFUSAL,
      response: { ok: false, status: 403 },
    });
    const dialog = await openDialog(unscheduled);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    expect(mocks.scheduleWorkflow).toHaveBeenCalledTimes(1);
    expect(mocks.scheduleWorkflow.mock.calls[0][0]).toEqual(
      expect.objectContaining({
        body: expect.objectContaining({ id: 'workflow-1' }),
        headers: { 'X-User-Action': 'proof-of-user' },
      })
    );
    expect(mocks.toastError.mock.calls[0][0].msg).toContain(REFUSAL);
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    // The dialog stays on the schedule as it still is, and the list is not
    // replaced by "Couldn't load workflows".
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.queryByText(/Couldn.t load workflows/)).not.toBeInTheDocument();
  });

  it('Remove schedule sends the proof, and a refusal never says the schedule stopped', async () => {
    mocks.scheduleWorkflow.mockResolvedValue({
      error: REFUSAL,
      response: { ok: false, status: 403 },
    });
    const dialog = await openDialog(scheduled);
    fireEvent.click(within(dialog).getByRole('button', { name: copy.schedule.remove }));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    expect(mocks.scheduleWorkflow.mock.calls[0][0]).toEqual(
      expect.objectContaining({
        body: { id: 'workflow-1', cron_schedule: null },
        headers: { 'X-User-Action': 'proof-of-user' },
      })
    );
    expect(mocks.toastError.mock.calls[0][0].msg).toContain(REFUSAL);
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    expect(screen.getByRole('dialog')).toBeInTheDocument();
  });

  it('an accepted save still says so, and closes the dialog', async () => {
    mocks.scheduleWorkflow.mockResolvedValue({ data: {}, response: { ok: true, status: 200 } });
    const dialog = await openDialog(unscheduled);
    fireEvent.click(within(dialog).getByRole('button', { name: 'Save' }));

    await waitFor(() => expect(mocks.toastSuccess).toHaveBeenCalledTimes(1));
    // One line, and it names the workflow: the last chance to notice the wrong row.
    expect(mocks.toastSuccess.mock.calls[0][0]).toEqual({
      title: copy.schedule.saved('Cohort review'),
    });
    expect(mocks.toastError).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });
});
