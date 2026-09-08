import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import type { ReactNode } from 'react';
import SchedulesView from './SchedulesView';

const mocks = vi.hoisted(() => ({
  listSchedules: vi.fn(),
  createSchedule: vi.fn(),
  deleteSchedule: vi.fn(),
  pauseSchedule: vi.fn(),
  unpauseSchedule: vi.fn(),
  updateSchedule: vi.fn(),
  killRunningJob: vi.fn(),
  inspectRunningJob: vi.fn(),
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

vi.mock('../../toasts', () => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

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
});

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

    await user.click(await screen.findByRole('button', { name: 'Delete nightly-cohort' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Delete "nightly-cohort"?' })).toBeInTheDocument();

    fireEvent.pointerDown(document.body);

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(mocks.deleteSchedule).not.toHaveBeenCalled();
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

    expect(await screen.findByText('Failed')).toBeInTheDocument();
    expect(
      screen.getByText(/no provider configured for the chat this schedule was created from/)
    ).toBeInTheDocument();
  });

  it('does not claim failure when the last run succeeded', async () => {
    mocks.listSchedules.mockResolvedValue([
      { ...schedule, last_run: '2026-08-02T09:00:00Z', last_error: null },
    ]);
    renderSchedules();

    await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    expect(screen.queryByText('Failed')).not.toBeInTheDocument();
  });

  it('presents an accessible empty state that opens schedule creation', async () => {
    const user = userEvent.setup();
    mocks.listSchedules.mockResolvedValueOnce([]);
    renderSchedules();

    const title = await screen.findByRole('heading', { name: 'No schedules yet' });
    const emptyState = title.closest('section');
    expect(emptyState).toHaveAccessibleDescription(
      'Create a schedule to run a saved workflow automatically at the time you choose.'
    );

    // The empty state's action and the header's are the same act, so they are
    // the same words. There are two of them on screen; the one inside the
    // empty state is the one this test is about.
    await user.click(
      within(emptyState as HTMLElement).getByRole('button', {
        name: 'New schedule',
      })
    );
    expect(screen.getByRole('dialog', { name: 'Create schedule form' })).toBeInTheDocument();
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

  it('says a paused schedule is paused as text, never as a filled pill', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, paused: true }]);
    renderSchedules();

    const paused = await screen.findByText('Paused');
    // `bg-background-warning/15` is what the pill was, and rule 4 of the
    // settings vocabulary bans every hand-mixed alpha.
    for (let node: HTMLElement | null = paused; node; node = node.parentElement) {
      expect(node.className).not.toMatch(/bg-background-\w+\/\d/);
      if (node.classList.contains('biorouter-list-row')) break;
    }
    // The hue is a dot beside the word (§3.4), not a fill behind it.
    expect(paused.querySelector('.rounded-full')).not.toBeNull();
  });

  it('puts each schedule on the shared hairline row, with no card around the list', async () => {
    renderSchedules();
    const open = await screen.findByRole('button', { name: 'View schedule nightly-cohort' });
    const row = open.closest('.biorouter-list-row');
    expect(row).not.toBeNull();
    expect(row?.parentElement).toHaveClass('biorouter-list-shell');
  });
});
