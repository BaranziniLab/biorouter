import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import type { ReactNode } from 'react';
import ScheduleDetailView from './ScheduleDetailView';

const mocks = vi.hoisted(() => ({
  getScheduleSessions: vi.fn(),
  listSchedules: vi.fn(),
  runScheduleNow: vi.fn(),
  pauseSchedule: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock('../../schedule', () => ({
  ...mocks,
  unpauseSchedule: vi.fn(),
  updateSchedule: vi.fn(),
  killRunningJob: vi.fn(),
  inspectRunningJob: vi.fn(),
}));
vi.mock('../../toasts', () => mocks);
vi.mock('../../api', () => ({ getSession: vi.fn() }));
vi.mock('../sessions/SessionHistoryView', () => ({ default: () => null }));
vi.mock('./ScheduleModal', () => ({ ScheduleModal: () => null }));
vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const schedule = {
  id: 'daily-meditation',
  source: '/tmp/daily-meditation.yaml',
  cron: '0 0 3 * * *',
  last_run: null,
  currently_running: false,
  paused: false,
};

function renderDetails() {
  return render(
    <MemoryRouter>
      <ScheduleDetailView scheduleId={schedule.id} onNavigateBack={vi.fn()} />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.listSchedules.mockResolvedValue([schedule]);
  mocks.getScheduleSessions.mockResolvedValue([]);
  mocks.pauseSchedule.mockResolvedValue(undefined);
});

describe('manual schedule run feedback', () => {
  it('announces a pending run without claiming success or submitting twice', async () => {
    let finish!: (id: string) => void;
    mocks.runScheduleNow.mockReturnValue(new Promise<string>((resolve) => (finish = resolve)));
    renderDetails();
    const run = await screen.findByRole('button', { name: 'Run now' });
    fireEvent.click(run);
    fireEvent.click(run);

    expect(mocks.runScheduleNow).toHaveBeenCalledTimes(1);
    expect(run).toBeDisabled();
    expect(screen.getByRole('status')).toHaveTextContent('Waiting for the scheduled run to finish');
    expect(mocks.toastSuccess).not.toHaveBeenCalled();

    await act(async () => finish('finished-session'));
    await waitFor(() => expect(run).not.toBeDisabled());
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(mocks.getScheduleSessions).toHaveBeenCalledTimes(2);
  });

  it('clears pending feedback and allows a retry after a failed run', async () => {
    let fail!: (reason: Error) => void;
    mocks.runScheduleNow.mockReturnValue(new Promise<string>((_, reject) => (fail = reject)));
    renderDetails();
    const run = await screen.findByRole('button', { name: 'Run now' });
    fireEvent.click(run);
    expect(screen.getByRole('status')).toBeInTheDocument();

    await act(async () => fail(new Error('Provider unavailable')));
    await waitFor(() => expect(run).not.toBeDisabled());
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(mocks.toastError).toHaveBeenCalledWith(
      expect.objectContaining({ msg: 'Provider unavailable' })
    );
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it('does not describe another pending action as a schedule run', async () => {
    let finish!: () => void;
    mocks.pauseSchedule.mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    renderDetails();
    fireEvent.click(await screen.findByRole('button', { name: 'Pause' }));
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    expect(mocks.runScheduleNow).not.toHaveBeenCalled();
    await act(async () => finish());
  });
});

describe('one session id, one typeface', () => {
  /**
   * A session id was rendered three times on this screen in two faces.
   *
   * In a single run card, an unnamed session's heading printed
   * `Session ID: <id>` in the body font while the row 26 lines below printed
   * `ID: <id>` in `font-mono` — the SAME string, in one card, visible without
   * scrolling. The running-schedule line above printed a third copy in the body
   * font again. The working directory beside them was body here and mono in the
   * sidebar, the history view and the shared-session view.
   *
   * The rule is `main.css`'s own D-31 — "mono for data, sans for chrome" — and
   * an id and a path are both data. What made this a bug rather than a
   * preference is that the two renderings were on screen at the same time.
   *
   * ⚠ **The card is gone and so is the duplication.** A run is now one row, and
   * the row prints an unnamed run's id ONCE, in mono. So the assertion moved
   * from "every copy is mono" to "there is exactly one copy, and it is mono" —
   * which is the stronger statement, and the one that fails if a second
   * rendering of the same id is ever added back beside the first.
   *
   * ⚠ jsdom never runs Tailwind, so `getComputedStyle(...).fontFamily` reports
   * the same thing whatever the class says. The assertion has to be on the
   * class, and it walks up from the text node because the class sits on a
   * wrapping span rather than on the element holding the text.
   */
  const monoAncestor = (element: HTMLElement | null): boolean => {
    for (let node = element; node; node = node.parentElement) {
      if (node.classList?.contains('font-mono')) return true;
    }
    return false;
  };

  it('sets an unnamed session id and its working directory in the data face', async () => {
    mocks.getScheduleSessions.mockResolvedValue([
      { id: 'sess-20260902-7f3', name: null, workingDir: '/tmp/work', messageCount: 2 },
    ]);
    renderDetails();

    const rendered = await screen.findAllByText('sess-20260902-7f3');
    expect(rendered).toHaveLength(1);
    for (const node of rendered) {
      expect(monoAncestor(node as HTMLElement)).toBe(true);
    }
    expect(monoAncestor(screen.getByText('/tmp/work') as HTMLElement)).toBe(true);
  });

  it('sets a running schedule’s current session id in the same face', async () => {
    mocks.listSchedules.mockResolvedValue([
      { ...schedule, currently_running: true, current_session_id: 'sess-running-1' },
    ]);
    renderDetails();
    expect(monoAncestor((await screen.findByText('sess-running-1')) as HTMLElement)).toBe(true);
  });
});

/**
 * The flat rebuild (astryx §4.5). Each assertion below names a shape the view
 * used to have and no longer may.
 */
describe('the detail view is rows and hairlines, not boxes', () => {
  const SOURCE = readFileSync(join(__dirname, 'ScheduleDetailView.tsx'), 'utf8');
  /**
   * Comments stripped, for the reason `settingsVocabulary.test.ts` records for
   * its own banned-class rule: prose explaining why a construction is gone is
   * not a use of it. The docblock below names `<Card>` and `h-screen` on
   * purpose — that history is the point of the docblock — and a raw substring
   * search over the file would read those two words as the defect.
   */
  const CODE = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');

  /**
   * ⚠ Asserted at the SOURCE, and it has to be. jsdom has no layout engine and
   * never runs Tailwind, so a card and a bare div render identically here —
   * the box is only real in a browser against the built stylesheet.
   */
  it('mounts no Card, and sizes itself from its parent rather than the viewport', () => {
    expect(CODE).not.toMatch(/from '\.\.\/ui\/card'/);
    expect(CODE).not.toContain('<Card');
    // `h-screen` is the anti-pattern MainPanelLayout's own comment warns about:
    // it forces viewport height regardless of the parent, which breaks the view
    // inside an embedded pane.
    expect(CODE).not.toContain('h-screen');
    expect(CODE).toContain('<MainPanelLayout>');
  });

  it('renders the facts as definition rows, one hairline list', async () => {
    renderDetails();

    for (const label of ['Runs', 'Cron', 'Workflow', 'Last run', 'Id']) {
      expect(await screen.findByText(label)).toBeInTheDocument();
    }

    // The row is the shared settings row, and the value sits on its trailing
    // edge — not a `**Label:** value` sentence inside a card.
    const cronLabel = await screen.findByText('Cron');
    const row = cronLabel.closest('.biorouter-settings-row');
    expect(row).not.toBeNull();
    expect(row).toHaveTextContent('0 0 3 * * *');

    // The id is a definition row now, not a "Viewing Schedule ID:" sentence.
    expect(screen.queryByText(/Viewing Schedule ID/)).not.toBeInTheDocument();
  });

  it('says a paused schedule is paused as text, never as a filled pill', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, paused: true }]);
    renderDetails();

    const paused = await screen.findByText('Paused');
    // A hand-mixed alpha fill (`bg-background-warning/15`) is what the status
    // used to be, and rule 4 of the settings vocabulary bans exactly that.
    for (let node: HTMLElement | null = paused; node; node = node.parentElement) {
      expect(node.className).not.toMatch(/bg-background-\w+\/\d/);
      if (node.classList.contains('biorouter-settings-section')) break;
    }
    // The hue is carried by a dot beside the word, the §3.4 idiom.
    expect(paused.querySelector('.rounded-full')).not.toBeNull();
  });

  it('turns the paused sentence into one note rather than loose warning prose', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, paused: true }]);
    renderDetails();
    expect(
      await screen.findByText(/This schedule is paused and will not run automatically/)
    ).toBeInTheDocument();
  });

  /**
   * ⚠ The one state a sandbox cannot be put into: `currently_running` is
   * reconciled against a live process, so a daemon started against a JSON that
   * claims a run is in flight clears the flag before the interface sees it
   * (measured). The branch is exercised here or nowhere.
   */
  it('replaces the idle actions with the run’s own while a run is in flight', async () => {
    mocks.listSchedules.mockResolvedValue([{ ...schedule, currently_running: true }]);
    renderDetails();

    expect(await screen.findByRole('button', { name: 'Inspect run' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop run' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Run now' })).toBeDisabled();
    // Nothing is left greyed out that could simply be absent.
    expect(screen.queryByRole('button', { name: 'Pause' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Edit' })).not.toBeInTheDocument();

    // One note, and only the one that applies.
    expect(await screen.findByText(/This schedule is running/)).toBeInTheDocument();
    expect(screen.queryByText(/This schedule is paused/)).not.toBeInTheDocument();
  });
});
