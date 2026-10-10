import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import type { ReactNode } from 'react';
import SchedulesView from './SchedulesView';
import { DELETE_SCHEDULE_MESSAGE, scheduleCopy } from './copy';

const mocks = vi.hoisted(() => ({
  listSchedules: vi.fn(),
  createSchedule: vi.fn(),
  deleteSchedule: vi.fn(),
  pauseSchedule: vi.fn(),
  unpauseSchedule: vi.fn(),
  updateSchedule: vi.fn(),
  killRunningJob: vi.fn(),
  inspectRunningJob: vi.fn(),
  runScheduleNow: vi.fn(),
}));

vi.mock('../../schedule', () => ({
  listSchedules: mocks.listSchedules,
  createSchedule: mocks.createSchedule,
  deleteSchedule: mocks.deleteSchedule,
  pauseSchedule: mocks.pauseSchedule,
  unpauseSchedule: mocks.unpauseSchedule,
  updateSchedule: mocks.updateSchedule,
  killRunningJob: mocks.killRunningJob,
  inspectRunningJob: mocks.inspectRunningJob,
  runScheduleNow: mocks.runScheduleNow,
}));

vi.mock('./ScheduleDetailView', () => ({
  default: ({ scheduleId }: { scheduleId: string }) => (
    <div aria-label="Schedule detail">Schedule detail for {scheduleId}</div>
  ),
}));

vi.mock('./ScheduleModal', () => ({
  ScheduleModal: ({ isOpen }: { isOpen: boolean }) =>
    isOpen ? <div role="dialog" aria-label="Create schedule form" /> : null,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const toasts = vi.hoisted(() => ({ toastError: vi.fn(), toastSuccess: vi.fn() }));
vi.mock('../../toasts', () => toasts);

const schedule = {
  id: 'nightly-cohort',
  source: '/tmp/nightly-cohort.yaml',
  cron: '0 0 1 * * *',
  last_run: null,
  currently_running: false,
  paused: false,
};

function renderSchedules() {
  return render(
    <MemoryRouter>
      <SchedulesView />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.listSchedules.mockResolvedValue([schedule]);
  mocks.createSchedule.mockResolvedValue(undefined);
  mocks.deleteSchedule.mockResolvedValue(undefined);
  mocks.pauseSchedule.mockResolvedValue(undefined);
  mocks.unpauseSchedule.mockResolvedValue(undefined);
  mocks.updateSchedule.mockResolvedValue(undefined);
  mocks.killRunningJob.mockResolvedValue({ message: 'Killed' });
  mocks.inspectRunningJob.mockResolvedValue({});
  mocks.runScheduleNow.mockResolvedValue('run-session-1');
});

/** Radix opens a dropdown on pointerdown, not click. */
function openRowMenu(name = 'nightly-cohort') {
  fireEvent.pointerDown(screen.getByRole('button', { name: scheduleCopy.moreActionsNamed(name) }), {
    button: 0,
    ctrlKey: false,
  });
}

describe('SchedulesView interactions', () => {
  it('exposes schedule navigation as a keyboard-activatable button without nesting actions', async () => {
    const user = userEvent.setup();
    renderSchedules();

    const openSchedule = await screen.findByRole('button', {
      name: 'View schedule nightly-cohort',
    });
    expect(openSchedule.tagName).toBe('BUTTON');
    expect(openSchedule.querySelector('button')).toBeNull();

    openSchedule.focus();
    await user.keyboard('{Enter}');

    expect(await screen.findByText('Schedule detail for nightly-cohort')).toBeInTheDocument();
  });

  it('asks for deletion confirmation and dismisses it when clicking outside', async () => {
    const user = userEvent.setup();
    renderSchedules();

    await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: scheduleCopy.delete }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Delete nightly-cohort?' })).toBeInTheDocument();

    fireEvent.pointerDown(document.body);

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(mocks.deleteSchedule).not.toHaveBeenCalled();
  });

  /**
   * Deleting a schedule used to delete the workflow it was created from
   * (finding F1), and the confirmation's "removes the schedule and its run
   * configuration" was the only warning the user got — which described the bug
   * accurately enough that it read as intended behaviour.
   *
   * `scheduler::scheduler_owns_source` now confines the unlink to the copy the
   * scheduler made for itself, so the dialog must promise what actually happens.
   * Fails the old wording, and fails a future one that stops saying which file
   * survives.
   */
  it('promises the workflow survives the deletion', async () => {
    const user = userEvent.setup();
    renderSchedules();

    await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: scheduleCopy.delete }));

    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveTextContent(DELETE_SCHEDULE_MESSAGE);
    expect(DELETE_SCHEDULE_MESSAGE).toMatch(/workflow it runs is left in place/i);
    expect(DELETE_SCHEDULE_MESSAGE).not.toMatch(/run configuration/i);
  });

  it('suppresses duplicate rapid actions while the first request is pending', async () => {
    let finishPause: (() => void) | undefined;
    mocks.pauseSchedule.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        finishPause = resolve;
      })
    );
    renderSchedules();

    const pause = await screen.findByRole('button', { name: 'Pause nightly-cohort' });
    fireEvent.click(pause);
    fireEvent.click(pause);

    expect(mocks.pauseSchedule).toHaveBeenCalledTimes(1);
    expect(pause).toBeDisabled();

    await act(async () => finishPause?.());
    await waitFor(() => expect(pause).not.toBeDisabled());
  });

  // Issue #56, task 24 step 3(a). A cron tick that returns `Err` used to leave
  // nothing behind but a log line, and a scheduled run mints a fresh session
  // each time — so a job that had been failing since the day it was created had
  // no surface anywhere. The backend now records `last_error` on the job; this
  // is the half that lets a user see it.
  it('shows a failing schedule as failing, with the reason', async () => {
    mocks.listSchedules.mockResolvedValue([
      {
        ...schedule,
        last_run: '2026-08-02T09:00:00Z',
        last_error: 'no provider configured for the chat this schedule was created from',
      },
    ]);
    renderSchedules();

    // The last error REPLACES the meta line, in danger ink, and still says the word.
    const line = await screen.findByText(
      /no provider configured for the chat this schedule was created from/
    );
    expect(line).toHaveTextContent(/^Failed · /);
    expect(line).toHaveClass('text-text-danger');
    const row = line.closest('.biorouter-list-row') as HTMLElement;
    expect(row.querySelector('[data-slot="status-dot"]')).toHaveAttribute('data-tone', 'danger');
  });

  it('does not claim failure when the last run succeeded', async () => {
    mocks.listSchedules.mockResolvedValue([
      { ...schedule, last_run: '2026-08-02T09:00:00Z', last_error: null },
    ]);
    renderSchedules();

    await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    expect(screen.queryByText(/Failed/)).not.toBeInTheDocument();
  });

  /**
   * One accent per view (spec 3.10): the band's "New schedule" is the only
   * button. The empty state says one short sentence and offers no second copy
   * of the primary.
   */
  it('presents an accessible empty state, and the band is the one way to create', async () => {
    const user = userEvent.setup();
    mocks.listSchedules.mockResolvedValueOnce([]);
    renderSchedules();

    const title = await screen.findByRole('heading', { name: scheduleCopy.emptyTitle });
    const emptyState = title.closest('section');
    expect(emptyState).toHaveAccessibleDescription(scheduleCopy.emptyDescription);
    expect(within(emptyState as HTMLElement).queryByRole('button')).toBeNull();

    expect(screen.getAllByRole('button', { name: scheduleCopy.newSchedule })).toHaveLength(1);
    await user.click(screen.getByRole('button', { name: scheduleCopy.newSchedule }));
    expect(screen.getByRole('dialog', { name: 'Create schedule form' })).toBeInTheDocument();
  });

  it('runs a schedule now from the row menu, and says so in one line', async () => {
    const user = userEvent.setup();
    renderSchedules();

    await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    openRowMenu();
    await user.click(await screen.findByRole('menuitem', { name: scheduleCopy.runNow }));

    expect(mocks.runScheduleNow).toHaveBeenCalledWith('nightly-cohort');
    await waitFor(() =>
      expect(toasts.toastSuccess).toHaveBeenCalledWith({ title: scheduleCopy.runStarted })
    );
  });

  it('opens the same menu on a right-click of the row', async () => {
    renderSchedules();

    const open = await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    fireEvent.contextMenu(open);

    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.textContent)).toEqual([
      scheduleCopy.edit,
      scheduleCopy.runNow,
      scheduleCopy.delete,
    ]);
  });

  /** Spec 6.9: the status word is the confirmation; only a failure toasts. */
  it('pauses without a success toast, and names the schedule when it cannot', async () => {
    mocks.pauseSchedule.mockRejectedValueOnce(new Error('daemon said no'));
    renderSchedules();

    fireEvent.click(await screen.findByRole('button', { name: 'Pause nightly-cohort' }));

    await waitFor(() =>
      expect(toasts.toastError).toHaveBeenCalledWith({
        title: scheduleCopy.couldNotPause('nightly-cohort'),
        msg: 'daemon said no',
      })
    );
    expect(toasts.toastSuccess).not.toHaveBeenCalled();
  });
});

/**
 * The flat rebuild (astryx §3.10, §4.2). Each assertion names a shape the list
 * used to have and no longer may.
 */
describe('the schedule list is rows and hairlines, not boxes', () => {
  const SOURCE = readFileSync(join(__dirname, 'SchedulesView.tsx'), 'utf8');
  /** See the note on the same constant in `ScheduleDetailView.test.tsx`. */
  const CODE = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

  /**
   * ⚠ At the SOURCE, because jsdom never runs Tailwind: a card and a bare div
   * render identically there, so only the class string can be asserted.
   */
  it('mounts no Card', () => {
    expect(CODE).not.toMatch(/from '\.\.\/ui\/card'/);
    expect(CODE).not.toContain('<Card');
  });

  /**
   * The band (spec 3.10): the title, its help and the actions share one 44px
   * line, Refresh as a ghost round icon and "New schedule" as the view's one
   * accent. This reverses the 2026-09-07 "actions on their own line" decision,
   * recorded as reversed in astryx §4.2. Asserted through the DOM: a source grep
   * for `<PageHeader` would pass on a view that mounted it and then put a button
   * somewhere else as well.
   */
  it('puts both header actions in the band’s action cluster', async () => {
    renderSchedules();

    const heading = await screen.findByRole('heading', { level: 1, name: 'Scheduler' });
    const band = screen.getByTestId('page-header');
    expect(band).toContainElement(heading);

    const create = screen.getAllByRole('button', { name: 'New schedule' })[0];
    const strip = create.closest('.biorouter-page-header-actions');
    expect(strip).not.toBeNull();
    expect(strip).toContainElement(screen.getByRole('button', { name: 'Refresh schedules' }));
  });

  it('says a paused schedule is paused as text, never as a filled pill', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, paused: true }]);
    renderSchedules();

    const paused = await screen.findByText(/^Paused · /);
    // `bg-background-warning/15` is what the pill was, and rule 4 of the
    // settings vocabulary bans every hand-mixed alpha.
    for (let node: HTMLElement | null = paused; node; node = node.parentElement) {
      expect(node.className).not.toMatch(/bg-background-\w+\/\d/);
      if (node.classList.contains('biorouter-list-row')) break;
    }
    // The hue is the row's leading status dot (spec 3.10), muted for Paused.
    const row = paused.closest('.biorouter-list-row') as HTMLElement;
    expect(row.querySelector('[data-slot="status-dot"]')).toHaveAttribute('data-tone', 'idle');
    // Paused rows offer Resume, the one verb pair (Pause/Resume) everywhere.
    expect(screen.getByRole('button', { name: 'Resume nightly-cohort' })).toBeInTheDocument();
  });

  /**
   * Two lines, never more: the name, then ONE meta line in the sans face with
   * tabular figures. Asserted at the class because jsdom has no layout.
   */
  it('gives each row a title line and one sans meta line', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, last_run: '2026-10-07T21:00:00Z' }]);
    renderSchedules();

    const open = await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    const lines = open.querySelectorAll('h3, p');
    expect(lines).toHaveLength(2);
    const meta = lines[1] as HTMLElement;
    expect(meta).toHaveTextContent(/^Scheduled · .+ · Last run /);
    expect(meta).toHaveClass('tabular-nums');
    expect(meta.querySelector('.font-mono')).toBeNull();
    expect(open.querySelector('svg')).toBeNull();
  });

  it('puts each schedule on the shared hairline row, with no card around the list', async () => {
    renderSchedules();
    const open = await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    const row = open.closest('.biorouter-list-row');
    expect(row).not.toBeNull();
    expect(row?.parentElement).toHaveClass('biorouter-list-shell');
  });

  /**
   * ⚠ The one state a sandbox cannot be put into. `paused` and `last_error` are
   * fields on the schedule record and survive a restart; `currently_running` is
   * reconciled against a live process, so a daemon started against a JSON that
   * claims a run is in flight clears the flag before the interface ever sees
   * it — measured. So the branch is exercised here or nowhere.
   */
  it('says a running schedule is running, and offers the run’s own actions', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, currently_running: true }]);
    renderSchedules();

    const running = await screen.findByText(/^Running · /);
    for (let node: HTMLElement | null = running; node; node = node.parentElement) {
      expect(node.className).not.toMatch(/bg-background-\w+\/\d/);
      if (node.classList.contains('biorouter-list-row')) break;
    }
    // Motion means "still going" (astryx §4.4): only this state's dot is live.
    const row = running.closest('.biorouter-list-row') as HTMLElement;
    expect(row.querySelector('[data-slot="status-dot"]')).toHaveAttribute('data-live', 'true');

    // Pause is meaningless mid-run and is replaced by Stop, not disabled.
    expect(screen.getByRole('button', { name: 'Stop nightly-cohort' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Pause nightly-cohort' })).not.toBeInTheDocument();
    // Inspect leads the menu; Edit and Run now wait for the run.
    openRowMenu();
    expect(await screen.findByRole('menuitem', { name: scheduleCopy.inspectRun })).toBeEnabled();
    expect(screen.getByRole('menuitem', { name: scheduleCopy.edit })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  /**
   * The resting state is STATED. The pills said nothing at all for a schedule
   * that was simply live, so a row gave no answer to "will this run?".
   */
  it('states the resting state rather than leaving the row silent', async () => {
    renderSchedules();
    expect(await screen.findByText(/^Scheduled · /)).toBeInTheDocument();
  });

  /**
   * The list used to render behind `!isLoading && schedules.length > 0`, and
   * `fetchSchedules` raises `isLoading` on the fifteen-second POLL as well as
   * on first load — so with rows already on screen none of the three branches
   * matched and the list unmounted for the length of every poll's request.
   * Pre-existing, and much more visible now the rows sit on the canvas with no
   * card to hold the space.
   */
  it('keeps the rows on screen while a refresh is in flight', async () => {
    renderSchedules();
    const row = await screen.findByRole('button', { name: 'View schedule nightly-cohort' });

    let finishRefresh!: (jobs: unknown[]) => void;
    mocks.listSchedules.mockReturnValueOnce(
      new Promise<unknown[]>((resolve) => (finishRefresh = resolve))
    );
    fireEvent.click(screen.getByRole('button', { name: 'Refresh schedules' }));

    expect(row).toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'No schedules yet' })).not.toBeInTheDocument();

    await act(async () => finishRefresh([schedule]));
    expect(
      await screen.findByRole('button', { name: 'View schedule nightly-cohort' })
    ).toBeInTheDocument();
  });
});
